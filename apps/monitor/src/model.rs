use std::cmp::Ordering;
use std::collections::{HashSet, VecDeque};

use crate::L10N;
use crate::apps::{Catalog, Group, GroupKey};
use crate::format::{format_bytes, format_cpu_time, format_percent, format_rate};
use crate::sampler::{Process, Snapshot};
use crate::signal::{SignalError, Stop, stop};

pub const HISTORY: usize = 60;
const MISSING: &str = "\u{2013}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Cpu,
    Memory,
    Disk,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Cpu,
    CpuTime,
    Threads,
    Pid,
    User,
    Memory,
    Resident,
    Read,
    Written,
    ReadTotal,
    WrittenTotal,
    Received,
    Sent,
    ReceivedTotal,
    SentTotal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    Group(GroupKey),
    Process { pid: u32, start_ticks: u64 },
    Interface(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    None,
    Collapsed,
    Expanded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: RowKey,
    pub depth: u8,
    pub disclosure: Disclosure,
    pub name: String,
    pub icon: Option<String>,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Graph {
    Stacked { lower: Vec<f32>, upper: Vec<f32> },
    Filled(Vec<f32>),
    Mirrored { up: Vec<f32>, down: Vec<f32> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopTarget {
    pub name: String,
    pub roots: Vec<(u32, u64)>,
    pub processes: Vec<(u32, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
enum Measure {
    Number(f64),
    Text(String),
    Missing,
}

impl Measure {
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(a), Self::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
            (Self::Missing, Self::Missing) => Ordering::Equal,
            (Self::Missing, _) => Ordering::Less,
            (_, Self::Missing) => Ordering::Greater,
            (Self::Number(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) => Ordering::Greater,
        }
    }
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Cpu, Tab::Memory, Tab::Disk, Tab::Network];

    pub fn title(self) -> String {
        match self {
            Tab::Cpu => L10N.tr("tab-cpu"),
            Tab::Memory => L10N.tr("tab-memory"),
            Tab::Disk => L10N.tr("tab-disk"),
            Tab::Network => L10N.tr("tab-network"),
        }
    }

    pub fn columns(self) -> &'static [Column] {
        match self {
            Tab::Cpu => &[
                Column::Name,
                Column::Cpu,
                Column::CpuTime,
                Column::Threads,
                Column::Pid,
                Column::User,
            ],
            Tab::Memory => &[
                Column::Name,
                Column::Memory,
                Column::Resident,
                Column::Threads,
                Column::Pid,
                Column::User,
            ],
            Tab::Disk => &[
                Column::Name,
                Column::Read,
                Column::Written,
                Column::ReadTotal,
                Column::WrittenTotal,
                Column::Pid,
            ],
            Tab::Network => &[
                Column::Name,
                Column::Received,
                Column::Sent,
                Column::ReceivedTotal,
                Column::SentTotal,
            ],
        }
    }

    fn default_sort(self) -> Column {
        match self {
            Tab::Cpu => Column::Cpu,
            Tab::Memory => Column::Memory,
            Tab::Disk => Column::Written,
            Tab::Network => Column::Received,
        }
    }
}

impl Column {
    pub fn title(self, tab: Tab) -> String {
        match self {
            Column::Name if tab == Tab::Network => L10N.tr("column-interface"),
            Column::Name => L10N.tr("column-name"),
            Column::Cpu => L10N.tr("column-cpu"),
            Column::CpuTime => L10N.tr("column-cpu-time"),
            Column::Threads => L10N.tr("column-threads"),
            Column::Pid => L10N.tr("column-pid"),
            Column::User => L10N.tr("column-user"),
            Column::Memory => L10N.tr("column-memory"),
            Column::Resident => L10N.tr("column-resident"),
            Column::Read => L10N.tr("column-read"),
            Column::Written => L10N.tr("column-written"),
            Column::ReadTotal => L10N.tr("column-read-total"),
            Column::WrittenTotal => L10N.tr("column-written-total"),
            Column::Received => L10N.tr("column-received"),
            Column::Sent => L10N.tr("column-sent"),
            Column::ReceivedTotal => L10N.tr("column-received-total"),
            Column::SentTotal => L10N.tr("column-sent-total"),
        }
    }

    pub fn numeric(self) -> bool {
        !matches!(self, Column::Name | Column::User)
    }

    fn text(self, measure: &Measure) -> String {
        let Measure::Number(value) = measure else {
            return match measure {
                Measure::Text(text) => text.clone(),
                _ => MISSING.to_owned(),
            };
        };
        match self {
            Column::Cpu => format_percent(*value),
            Column::CpuTime => format_cpu_time(*value),
            Column::Memory
            | Column::Resident
            | Column::ReadTotal
            | Column::WrittenTotal
            | Column::ReceivedTotal
            | Column::SentTotal => format_bytes(*value as u64),
            Column::Read | Column::Written | Column::Received | Column::Sent => format_rate(*value),
            _ => format!("{value:.0}"),
        }
    }

    fn measure(self, processes: &[&Process], name: &str, leader: &Process) -> Measure {
        let sum = |value: fn(&Process) -> f64| {
            Measure::Number(processes.iter().map(|process| value(process)).sum())
        };
        let disk = |value: fn(&crate::sampler::Rates) -> f64| {
            let known = processes
                .iter()
                .filter_map(|process| process.disk.as_ref())
                .collect::<Vec<_>>();
            if known.is_empty() {
                Measure::Missing
            } else {
                Measure::Number(known.into_iter().map(value).sum())
            }
        };
        match self {
            Column::Name => Measure::Text(name.to_owned()),
            Column::Cpu => sum(|process| f64::from(process.cpu_percent)),
            Column::CpuTime => sum(|process| process.cpu_seconds),
            Column::Threads => sum(|process| f64::from(process.threads)),
            Column::Pid => Measure::Number(f64::from(leader.pid)),
            Column::User => Measure::Text(leader.identity.user.clone()),
            Column::Memory => {
                sum(|process| process.proportional.unwrap_or(process.resident) as f64)
            }
            Column::Resident => sum(|process| process.resident as f64),
            Column::Read => disk(|rates| rates.read_per_second),
            Column::Written => disk(|rates| rates.written_per_second),
            Column::ReadTotal => disk(|rates| rates.read_total as f64),
            Column::WrittenTotal => disk(|rates| rates.written_total as f64),
            Column::Received | Column::Sent | Column::ReceivedTotal | Column::SentTotal => {
                Measure::Missing
            }
        }
    }
}

struct Candidate {
    row: Row,
    measure: Measure,
    name: String,
    pid: u32,
}

fn order(a: &Candidate, b: &Candidate, descending: bool) -> Ordering {
    let primary = a.measure.compare(&b.measure);
    let primary = if descending {
        primary.reverse()
    } else {
        primary
    };
    primary
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.pid.cmp(&b.pid))
}

fn push_limited<T>(history: &mut VecDeque<T>, value: T) {
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(value);
}

pub struct Monitor {
    catalog: Catalog,
    snapshot: Snapshot,
    groups: Vec<Group>,
    tab: Tab,
    sort: Column,
    descending: bool,
    expanded: HashSet<GroupKey>,
    selection: Option<RowKey>,
    search: String,
    rows: Vec<Row>,
    cpu: VecDeque<(f32, f32)>,
    memory: VecDeque<f32>,
    disk: VecDeque<(f64, f64)>,
    network: VecDeque<(f64, f64)>,
}

impl Monitor {
    pub fn new(catalog: Catalog) -> Self {
        Self {
            catalog,
            snapshot: Snapshot::default(),
            groups: Vec::new(),
            tab: Tab::Cpu,
            sort: Tab::Cpu.default_sort(),
            descending: true,
            expanded: HashSet::new(),
            selection: None,
            search: String::new(),
            rows: Vec::new(),
            cpu: VecDeque::new(),
            memory: VecDeque::new(),
            disk: VecDeque::new(),
            network: VecDeque::new(),
        }
    }

    pub fn update(&mut self, snapshot: Snapshot) {
        self.groups = self.catalog.group(&snapshot.processes);
        if snapshot.seconds > 0.0 {
            push_limited(&mut self.cpu, (snapshot.cpu.system, snapshot.cpu.user));
            let memory = &snapshot.memory;
            push_limited(
                &mut self.memory,
                if memory.total > 0 {
                    memory.used as f32 / memory.total as f32
                } else {
                    0.0
                },
            );
            push_limited(
                &mut self.disk,
                (
                    snapshot.disk.read_per_second,
                    snapshot.disk.written_per_second,
                ),
            );
            push_limited(
                &mut self.network,
                snapshot
                    .interfaces
                    .iter()
                    .fold((0.0, 0.0), |(received, sent), interface| {
                        (
                            received + interface.received_per_second,
                            sent + interface.sent_per_second,
                        )
                    }),
            );
        }
        self.snapshot = snapshot;
        self.rebuild();
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.sort = tab.default_sort();
            self.descending = true;
            self.rebuild();
        }
    }

    pub fn sort(&self) -> (Column, bool) {
        (self.sort, self.descending)
    }

    pub fn sort_by(&mut self, column: Column) {
        if self.sort == column {
            self.descending = !self.descending;
        } else {
            self.sort = column;
            self.descending = column.numeric();
        }
        self.rebuild();
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    pub fn set_search(&mut self, search: &str) {
        self.search = search.to_owned();
        self.rebuild();
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selection(&self) -> Option<&RowKey> {
        self.selection.as_ref()
    }

    pub fn selected_index(&self) -> Option<usize> {
        let selection = self.selection.as_ref()?;
        self.rows.iter().position(|row| &row.key == selection)
    }

    pub fn select(&mut self, index: usize) {
        self.selection = self.rows.get(index).map(|row| row.key.clone());
    }

    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return;
        };
        let index = match self.selected_index() {
            Some(index) => index.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        };
        self.select(index);
    }

    pub fn expand(&mut self) -> bool {
        let Some(RowKey::Group(key)) = self.selection.clone() else {
            return false;
        };
        let expandable = self
            .rows
            .iter()
            .any(|row| row.key == RowKey::Group(key.clone()) && row.disclosure != Disclosure::None);
        if !expandable || !self.expanded.insert(key) {
            return false;
        }
        self.rebuild();
        true
    }

    pub fn collapse(&mut self) -> bool {
        match self.selection.clone() {
            Some(RowKey::Group(key)) => {
                if !self.expanded.remove(&key) {
                    return false;
                }
            }
            Some(RowKey::Process { pid, start_ticks }) => {
                let Some(group) = self.group_of(pid, start_ticks) else {
                    return false;
                };
                let key = group.key.clone();
                self.expanded.remove(&key);
                self.selection = Some(RowKey::Group(key));
            }
            _ => return false,
        }
        self.rebuild();
        true
    }

    pub fn toggle(&mut self, index: usize) {
        let Some(RowKey::Group(key)) = self.rows.get(index).map(|row| row.key.clone()) else {
            return;
        };
        if self.expanded.remove(&key) {
            if let Some(RowKey::Process { pid, start_ticks }) = self.selection.clone()
                && self
                    .group_of(pid, start_ticks)
                    .is_some_and(|group| group.key == key)
            {
                self.selection = Some(RowKey::Group(key));
            }
        } else {
            self.expanded.insert(key);
        }
        self.rebuild();
    }

    pub fn measures_proportional_memory(&self) -> bool {
        self.tab == Tab::Memory
    }

    fn group_of(&self, pid: u32, start_ticks: u64) -> Option<&Group> {
        self.groups.iter().find(|group| {
            group.members.iter().any(|member| {
                let process = &self.snapshot.processes[*member];
                process.pid == pid && process.start_ticks == start_ticks
            })
        })
    }

    pub fn stop_target(&self) -> Option<StopTarget> {
        let id = |process: &Process| (process.pid, process.start_ticks);
        match self.selection.as_ref()? {
            RowKey::Group(key) => {
                let group = self.groups.iter().find(|group| &group.key == key)?;
                let members = group
                    .members
                    .iter()
                    .map(|member| &self.snapshot.processes[*member])
                    .collect::<Vec<_>>();
                let roots = members
                    .iter()
                    .filter(|process| !members.iter().any(|parent| parent.pid == process.ppid))
                    .map(|process| id(process))
                    .collect();
                Some(StopTarget {
                    name: self.group_name(group),
                    roots,
                    processes: members.into_iter().map(id).collect(),
                })
            }
            RowKey::Process { pid, start_ticks } => {
                let process =
                    self.snapshot.processes.iter().find(|process| {
                        process.pid == *pid && process.start_ticks == *start_ticks
                    })?;
                Some(StopTarget {
                    name: process.identity.name.clone(),
                    roots: vec![id(process)],
                    processes: vec![id(process)],
                })
            }
            RowKey::Interface(_) => None,
        }
    }

    pub fn stop(target: &StopTarget, how: Stop) -> Result<(), SignalError> {
        let chosen = match how {
            Stop::Quit => &target.roots,
            Stop::ForceQuit => &target.processes,
        };
        let mut outcome = Ok(());
        for (pid, start_ticks) in chosen {
            match stop(*pid, *start_ticks, how) {
                Ok(()) | Err(SignalError::Exited) => {}
                Err(error) => {
                    if outcome.is_ok() {
                        outcome = Err(error);
                    }
                }
            }
        }
        outcome
    }

    pub fn executable(&self) -> Option<std::path::PathBuf> {
        let (pid, start_ticks) = match self.selection.as_ref()? {
            RowKey::Group(key) => {
                let group = self.groups.iter().find(|group| &group.key == key)?;
                let leader = &self.snapshot.processes[group.leader];
                (leader.pid, leader.start_ticks)
            }
            RowKey::Process { pid, start_ticks } => (*pid, *start_ticks),
            RowKey::Interface(_) => return None,
        };
        self.snapshot
            .processes
            .iter()
            .find(|process| process.pid == pid && process.start_ticks == start_ticks)?
            .identity
            .executable
            .clone()
    }

    fn group_name(&self, group: &Group) -> String {
        match group.key {
            GroupKey::Kernel => L10N.tr("group-kernel"),
            _ => group.name.clone(),
        }
    }

    fn matches(&self, process: &Process, query: &str) -> bool {
        process.identity.name.to_lowercase().contains(query) || process.pid.to_string() == query
    }

    fn rebuild(&mut self) {
        let columns = self.tab.columns();
        let sort = self.sort;
        let descending = self.descending;
        let query = self.search.trim().to_lowercase();
        self.rows = if self.tab == Tab::Network {
            self.interface_rows(&query)
        } else {
            let mut groups = Vec::new();
            for group in &self.groups {
                let name = self.group_name(group);
                let members = group
                    .members
                    .iter()
                    .map(|member| &self.snapshot.processes[*member])
                    .collect::<Vec<_>>();
                let group_matches = query.is_empty() || name.to_lowercase().contains(&query);
                let shown = members
                    .iter()
                    .copied()
                    .filter(|process| group_matches || self.matches(process, &query))
                    .collect::<Vec<_>>();
                if shown.is_empty() {
                    continue;
                }
                let leader = &self.snapshot.processes[group.leader];
                let key = RowKey::Group(group.key.clone());
                let disclosure = if members.len() == 1 {
                    Disclosure::None
                } else if !group_matches || self.expanded.contains(&group.key) {
                    Disclosure::Expanded
                } else {
                    Disclosure::Collapsed
                };
                let row = Row {
                    key,
                    depth: 0,
                    disclosure,
                    name: name.clone(),
                    icon: group.app.as_ref().and_then(|app| app.icon.clone()),
                    cells: columns
                        .iter()
                        .map(|column| column.text(&column.measure(&shown, &name, leader)))
                        .collect(),
                };
                let mut children = Vec::new();
                if disclosure == Disclosure::Expanded {
                    for process in &shown {
                        let name = &process.identity.name;
                        children.push(Candidate {
                            row: Row {
                                key: RowKey::Process {
                                    pid: process.pid,
                                    start_ticks: process.start_ticks,
                                },
                                depth: 1,
                                disclosure: Disclosure::None,
                                name: name.clone(),
                                icon: None,
                                cells: columns
                                    .iter()
                                    .map(|column| {
                                        column.text(&column.measure(&[process], name, process))
                                    })
                                    .collect(),
                            },
                            measure: sort.measure(&[process], name, process),
                            name: name.clone(),
                            pid: process.pid,
                        });
                    }
                    children.sort_by(|a, b| order(a, b, descending));
                }
                groups.push((
                    Candidate {
                        row,
                        measure: sort.measure(&shown, &name, leader),
                        name,
                        pid: leader.pid,
                    },
                    children,
                ));
            }
            groups.sort_by(|(a, _), (b, _)| order(a, b, descending));
            groups
                .into_iter()
                .flat_map(|(group, children)| {
                    std::iter::once(group.row).chain(children.into_iter().map(|child| child.row))
                })
                .collect()
        };
        if let Some(selection) = &self.selection
            && !self.rows.iter().any(|row| &row.key == selection)
        {
            self.selection = None;
        }
    }

    fn interface_rows(&self, query: &str) -> Vec<Row> {
        let mut candidates = self
            .snapshot
            .interfaces
            .iter()
            .filter(|interface| interface.name.to_lowercase().contains(query))
            .map(|interface| {
                let measure = |column: Column| match column {
                    Column::Name => Measure::Text(interface.name.clone()),
                    Column::Received => Measure::Number(interface.received_per_second),
                    Column::Sent => Measure::Number(interface.sent_per_second),
                    Column::ReceivedTotal => Measure::Number(interface.received_total as f64),
                    Column::SentTotal => Measure::Number(interface.sent_total as f64),
                    _ => Measure::Missing,
                };
                Candidate {
                    row: Row {
                        key: RowKey::Interface(interface.name.clone()),
                        depth: 0,
                        disclosure: Disclosure::None,
                        name: interface.name.clone(),
                        icon: None,
                        cells: Tab::Network
                            .columns()
                            .iter()
                            .map(|column| column.text(&measure(*column)))
                            .collect(),
                    },
                    measure: measure(self.sort),
                    name: interface.name.clone(),
                    pid: 0,
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| order(a, b, self.descending));
        candidates
            .into_iter()
            .map(|candidate| candidate.row)
            .collect()
    }

    pub fn graph(&self) -> Graph {
        match self.tab {
            Tab::Cpu => Graph::Stacked {
                lower: self.cpu.iter().map(|(system, _)| *system).collect(),
                upper: self
                    .cpu
                    .iter()
                    .map(|(system, user)| system + user)
                    .collect(),
            },
            Tab::Memory => Graph::Filled(self.memory.iter().copied().collect()),
            Tab::Disk => mirrored(&self.disk),
            Tab::Network => mirrored(&self.network),
        }
    }

    pub fn summary(&self) -> Vec<(String, String)> {
        let snapshot = &self.snapshot;
        let percent = |share: f32| format!("{} %", format_percent(f64::from(share) * 100.0));
        match self.tab {
            Tab::Cpu => vec![
                (L10N.tr("summary-system"), percent(snapshot.cpu.system)),
                (L10N.tr("summary-user"), percent(snapshot.cpu.user)),
                (L10N.tr("summary-idle"), percent(snapshot.cpu.idle)),
                (
                    L10N.tr("summary-processes"),
                    snapshot.processes.len().to_string(),
                ),
                (
                    L10N.tr("summary-threads"),
                    snapshot
                        .processes
                        .iter()
                        .map(|process| u64::from(process.threads))
                        .sum::<u64>()
                        .to_string(),
                ),
            ],
            Tab::Memory => {
                let memory = &snapshot.memory;
                vec![
                    (L10N.tr("summary-physical"), format_bytes(memory.total)),
                    (L10N.tr("summary-used"), format_bytes(memory.used)),
                    (L10N.tr("summary-cached"), format_bytes(memory.cached)),
                    (L10N.tr("summary-swap"), format_bytes(memory.swap_used)),
                    (
                        L10N.tr("summary-compressed"),
                        format_bytes(memory.compressed),
                    ),
                    (
                        L10N.tr("summary-pressure"),
                        format!("{} %", format_percent(f64::from(memory.pressure))),
                    ),
                ]
            }
            Tab::Disk => vec![
                (
                    L10N.tr("summary-read-rate"),
                    format_rate(snapshot.disk.read_per_second),
                ),
                (
                    L10N.tr("summary-written-rate"),
                    format_rate(snapshot.disk.written_per_second),
                ),
                (
                    L10N.tr("summary-read"),
                    format_bytes(snapshot.disk.read_total),
                ),
                (
                    L10N.tr("summary-written"),
                    format_bytes(snapshot.disk.written_total),
                ),
            ],
            Tab::Network => {
                let interfaces = &snapshot.interfaces;
                let total = |value: fn(&crate::sampler::Interface) -> f64| {
                    interfaces.iter().map(value).sum::<f64>()
                };
                vec![
                    (
                        L10N.tr("summary-received-rate"),
                        format_rate(total(|interface| interface.received_per_second)),
                    ),
                    (
                        L10N.tr("summary-sent-rate"),
                        format_rate(total(|interface| interface.sent_per_second)),
                    ),
                    (
                        L10N.tr("summary-received"),
                        format_bytes(total(|interface| interface.received_total as f64) as u64),
                    ),
                    (
                        L10N.tr("summary-sent"),
                        format_bytes(total(|interface| interface.sent_total as f64) as u64),
                    ),
                ]
            }
        }
    }
}

fn mirrored(history: &VecDeque<(f64, f64)>) -> Graph {
    let peak = history
        .iter()
        .fold(0.0, |peak: f64, (up, down)| peak.max(*up).max(*down));
    let scaled = |value: f64| {
        if peak > 0.0 {
            (value / peak) as f32
        } else {
            0.0
        }
    };
    Graph::Mirrored {
        up: history.iter().map(|(up, _)| scaled(*up)).collect(),
        down: history.iter().map(|(_, down)| scaled(*down)).collect(),
    }
}
