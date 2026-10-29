use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unbound_xdg::DesktopEntry;

use crate::sampler::Process;

const LAUNCHERS: &[&str] = &[
    "bash", "dash", "env", "fish", "flatpak", "java", "node", "perl", "pkexec", "python",
    "python3", "ruby", "sh", "sudo", "zsh",
];
const INIT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct Catalog {
    apps: Vec<AppInfo>,
    by_id: HashMap<String, usize>,
    by_program: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupKey {
    App(String),
    Process { pid: u32, start_ticks: u64 },
    Kernel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: GroupKey,
    pub name: String,
    pub app: Option<AppInfo>,
    pub leader: usize,
    pub members: Vec<usize>,
}

fn file_name(path: &str) -> Option<&str> {
    Path::new(path).file_name()?.to_str()
}

fn unescape_unit(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("\\x") {
        result.push_str(&rest[..index]);
        let byte = rest
            .get(index + 2..index + 4)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match byte {
            Some(byte) => {
                result.push(char::from(byte));
                rest = &rest[index + 4..];
            }
            None => {
                result.push_str("\\x");
                rest = &rest[index + 2..];
            }
        }
    }
    result.push_str(rest);
    result
}

fn unit_app_ids(unit: &str) -> Vec<String> {
    let Some(rest) = unit.strip_prefix("app-") else {
        return Vec::new();
    };
    let name = rest
        .strip_suffix(".scope")
        .and_then(|scope| scope.rsplit_once('-').map(|(name, _)| name))
        .or_else(|| {
            rest.strip_suffix(".service")
                .map(|service| service.split_once('@').map_or(service, |(name, _)| name))
        });
    let Some(name) = name else {
        return Vec::new();
    };
    let mut ids = vec![unescape_unit(name)];
    if let Some((_, id)) = name.split_once('-') {
        ids.push(unescape_unit(id));
    }
    ids
}

impl Catalog {
    pub fn new(entries: &[DesktopEntry]) -> Self {
        let mut catalog = Self::default();
        let mut programs = HashMap::<String, Vec<usize>>::new();
        for entry in entries {
            let index = catalog.apps.len();
            catalog.apps.push(AppInfo {
                id: entry.app_id().to_owned(),
                name: entry.name.clone(),
                icon: entry.icon.clone(),
                path: entry.path.clone(),
            });
            catalog.by_id.insert(entry.app_id().to_owned(), index);
            let command = entry.command().unwrap_or_default();
            if let Some(program) = command.first().and_then(|first| file_name(first))
                && !entry.terminal
                && !LAUNCHERS.contains(&program)
            {
                programs.entry(program.to_owned()).or_default().push(index);
            }
        }
        catalog.by_program = programs
            .into_iter()
            .filter_map(|(program, apps)| match apps.as_slice() {
                [only] => Some((program, *only)),
                _ => None,
            })
            .collect();
        catalog
    }

    fn app_of(&self, process: &Process) -> Option<usize> {
        let identity = &process.identity;
        if identity.kernel_thread {
            return None;
        }
        let from_unit = identity
            .unit
            .as_deref()
            .map(unit_app_ids)
            .unwrap_or_default()
            .iter()
            .find_map(|id| self.by_id.get(id).copied());
        from_unit.or_else(|| {
            let executable = identity
                .executable
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let argument = identity.command.first().and_then(|first| file_name(first));
            [executable, argument]
                .into_iter()
                .flatten()
                .find_map(|program| self.by_program.get(program).copied())
        })
    }

    pub fn group(&self, processes: &[Process]) -> Vec<Group> {
        let mut tree = Tree {
            processes,
            by_pid: processes
                .iter()
                .enumerate()
                .map(|(index, process)| (process.pid, index))
                .collect(),
            own: processes
                .iter()
                .map(|process| self.app_of(process))
                .collect(),
            roots: vec![Root::Unknown; processes.len()],
        };
        let roots = (0..processes.len())
            .map(|index| tree.root(index))
            .collect::<Vec<_>>();
        let mut groups = Vec::<Group>::new();
        let mut positions = HashMap::<GroupKey, usize>::new();
        for (index, top) in roots.iter().copied().enumerate() {
            let app = tree.own[top].map(|app| &self.apps[app]);
            let key = match app {
                Some(app) => GroupKey::App(app.id.clone()),
                None if processes[top].identity.kernel_thread => GroupKey::Kernel,
                None => GroupKey::Process {
                    pid: processes[top].pid,
                    start_ticks: processes[top].start_ticks,
                },
            };
            let position = *positions.entry(key.clone()).or_insert_with(|| {
                groups.push(Group {
                    key,
                    name: app.map_or_else(
                        || processes[top].identity.name.clone(),
                        |app| app.name.clone(),
                    ),
                    app: app.cloned(),
                    leader: top,
                    members: Vec::new(),
                });
                groups.len() - 1
            });
            let group = &mut groups[position];
            group.members.push(index);
            if roots[index] == index
                && processes[index].start_ticks < processes[group.leader].start_ticks
            {
                group.leader = index;
            }
        }
        groups
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    Unknown,
    Visiting,
    Known(usize),
}

struct Tree<'a> {
    processes: &'a [Process],
    by_pid: HashMap<u32, usize>,
    own: Vec<Option<usize>>,
    roots: Vec<Root>,
}

impl Tree<'_> {
    fn parent(&self, index: usize) -> Option<usize> {
        let ppid = self.processes[index].ppid;
        if ppid == INIT {
            return None;
        }
        self.by_pid.get(&ppid).copied()
    }

    fn root(&mut self, index: usize) -> usize {
        match self.roots[index] {
            Root::Known(root) => return root,
            Root::Visiting => return index,
            Root::Unknown => {}
        }
        self.roots[index] = Root::Visiting;
        let process = &self.processes[index];
        let independent = self.own[index].is_some() || process.identity.kernel_thread;
        let root = match self.parent(index) {
            Some(parent) if !independent => {
                let parent_root = self.root(parent);
                let same_program =
                    self.processes[parent].identity.name == self.processes[index].identity.name;
                if same_program || self.own[parent_root].is_some() {
                    parent_root
                } else {
                    index
                }
            }
            _ => index,
        };
        self.roots[index] = Root::Known(root);
        root
    }
}
