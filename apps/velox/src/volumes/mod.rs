pub mod portal;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use bande_l10n::{text, tr, trn};

use crate::engine::{Task, Volume};
use crate::resources::{
    Act, Acts, Pane, Scope, age, boxed, confirm, matches, push, search, size, tell, tool,
};
use bande_app_menu::Entry;
use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Button, Cell, Column, Menu, MenuItem, Row, Table, TextField, Ui, Widget, WidgetId,
};

use crate::containers::{INSPECTOR_WIDTH, Split, panel};

const PROJECT_PREFIX: &str = "project:";
const IMPORT: &str = "volume-import";
const EXPORT: &str = "volume-export";
const REMOVE: &str = "volume-remove";
const PRUNE: &str = "volume-prune";
const COPY_NAME: &str = "volume-copy-name";
const COPY_MOUNTPOINT: &str = "volume-copy-mountpoint";
const INSPECTOR: &str = "volume-inspector";
const INSPECTOR_SYMBOL: &str = include_str!("../../data/symbols/inspector.svg");
const IMPORT_SYMBOL: &str = include_str!("../../data/symbols/import.svg");
const PRUNE_SYMBOL: &str = include_str!("../../data/symbols/prune.svg");
const REMOVE_SYMBOL: &str = include_str!("../../data/symbols/remove.svg");
const NAME_WIDTH: f32 = 200.0;
const SIZE_WIDTH: f32 = 96.0;
const USED_BY_WIDTH: f32 = 160.0;
const CREATED_WIDTH: f32 = 130.0;
const INSPECTED_COLUMNS: usize = 2;

#[derive(Default)]
pub struct Volumes {
    table: Option<WidgetId>,
    remove: Option<WidgetId>,
    search: Option<WidgetId>,
    split: Option<WidgetId>,
    selected: Vec<String>,
    collapsed: HashSet<String>,
    filter: String,
    inspector: bool,
    inspected: Option<Inspected>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Inspected {
    title: String,
    note: String,
    facts: Vec<(String, String)>,
}

fn columns(inspector: bool) -> Vec<Column> {
    let mut columns = vec![
        Column::new(&tr!("Name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&tr!("Size"), SIZE_WIDTH).numeric().unsortable(),
        Column::new(&tr!("Used By"), USED_BY_WIDTH).unsortable(),
        Column::new(&tr!("Created"), CREATED_WIDTH).unsortable(),
    ];
    if inspector {
        columns.truncate(INSPECTED_COLUMNS);
    }
    columns
}

fn narrowed(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .map(|mut row| {
            row.cells.truncate(INSPECTED_COLUMNS);
            let children = narrowed(std::mem::take(&mut row.children));
            row.children(children)
        })
        .collect()
}

fn inspected(scope: &Scope, selected: &[String]) -> Inspected {
    let plain = |note: String| Inspected {
        title: String::new(),
        note,
        facts: Vec::new(),
    };
    let key = match selected {
        [] => return plain(tr!("No Selection")),
        [key] => key,
        _ => return plain(tr!("{count} Selected", count = selected.len())),
    };
    if let Some(project) = key.strip_prefix(PROJECT_PREFIX) {
        let members = scope
            .store
            .volumes
            .iter()
            .filter(|volume| volume.project() == Some(project))
            .count();
        let live = scope
            .containers
            .values()
            .any(|container| container.project() == Some(project));
        return Inspected {
            title: project.to_owned(),
            note: match live {
                true => trn!("1 volume", "{count} volumes", count = members),
                false => tr!("This Compose project no longer has any containers."),
            },
            facts: Vec::new(),
        };
    }
    let Some(volume) = scope
        .store
        .volumes
        .iter()
        .find(|volume| &volume.name == key)
    else {
        return plain(tr!("No Selection"));
    };
    let used = scope
        .store
        .usage
        .as_ref()
        .and_then(|usage| usage.volume_sizes.get(&volume.name))
        .map_or_else(|| "—".to_owned(), |bytes| size(*bytes));
    let users = users(scope, &volume.name);
    let mut facts = vec![
        (tr!("Driver"), volume.driver.clone()),
        (tr!("Size"), used),
        (tr!("Created"), created(volume)),
        (
            tr!("Used By"),
            match users.is_empty() {
                true => tr!("No container"),
                false => users.join(", "),
            },
        ),
    ];
    if let Some(project) = volume.project() {
        facts.push((tr!("Compose Project"), project.to_owned()));
    }
    facts.push((tr!("Mountpoint"), volume.mountpoint.clone()));
    Inspected {
        title: volume.name.clone(),
        note: String::new(),
        facts,
    }
}

fn created(volume: &Volume) -> String {
    volume
        .created_at
        .as_deref()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .map_or_else(|| "—".to_owned(), |at| age(at.as_second()))
}

fn users<'a>(scope: &'a Scope, name: &str) -> Vec<&'a str> {
    let mut users: Vec<&str> = scope
        .containers
        .values()
        .filter(|container| {
            container
                .mounts
                .iter()
                .flatten()
                .any(|mount| mount.kind == "volume" && mount.name.as_deref() == Some(name))
        })
        .map(|container| container.name())
        .collect();
    users.sort_unstable();
    users
}

fn volume_row(scope: &Scope, volume: &Volume, orphaned: bool) -> Row {
    let users = users(scope, &volume.name);
    let used_by = match (users.is_empty(), orphaned) {
        (true, true) => Cell::new(""),
        (true, false) => Cell::new(tr!("dangling")).badge(scope.palette.warning),
        (false, _) => Cell::new(users.join(", ")),
    };
    let name = Cell::new(volume.name.as_str()).cut_in_middle();
    let name = match users.is_empty() {
        true => name.muted(),
        false => name,
    };
    let size = scope
        .store
        .usage
        .as_ref()
        .and_then(|usage| usage.volume_sizes.get(&volume.name))
        .map_or_else(
            || Cell::new("—").muted(),
            |bytes| Cell::new(size(*bytes)).tabular(),
        );
    Row::new(
        volume.name.clone(),
        vec![name, size, used_by, Cell::new(created(volume)).muted()],
    )
}

fn rows(scope: &Scope, filter: &str) -> Vec<Row> {
    let mut projects: BTreeMap<&str, Vec<&Volume>> = BTreeMap::new();
    let mut loose = Vec::new();
    for volume in &scope.store.volumes {
        if !matches(
            filter,
            &[&volume.name, volume.project().unwrap_or_default()],
        ) {
            continue;
        }
        match volume.project() {
            Some(project) => projects.entry(project).or_default().push(volume),
            None => loose.push(volume),
        }
    }
    let live: HashSet<&str> = scope
        .containers
        .values()
        .filter_map(|container| container.project())
        .collect();
    let mut named: Vec<(String, Row)> = loose
        .into_iter()
        .map(|volume| (volume.name.to_lowercase(), volume_row(scope, volume, false)))
        .collect();
    named.extend(projects.into_iter().map(|(project, mut members)| {
        members.sort_by_key(|volume| volume.name.to_lowercase());
        let orphaned = !live.contains(project);
        let (name, status) = match orphaned {
            false => (
                Cell::new(project),
                Cell::new(trn!("1 volume", "{count} volumes", count = members.len())).muted(),
            ),
            true => (
                Cell::new(project).muted(),
                Cell::new(tr!("orphaned")).badge(scope.palette.warning),
            ),
        };
        let row = Row::new(
            format!("{PROJECT_PREFIX}{project}"),
            vec![name, Cell::new(""), status, Cell::new("")],
        )
        .children(
            members
                .iter()
                .map(|volume| volume_row(scope, volume, orphaned))
                .collect(),
        );
        (project.to_lowercase(), row)
    }));
    named.sort_by(|left, right| left.0.cmp(&right.0));
    named.into_iter().map(|(_, row)| row).collect()
}

fn names(scope: &Scope, keys: &[String]) -> Vec<String> {
    let mut names: Vec<String> = keys
        .iter()
        .flat_map(|key| match key.strip_prefix(PROJECT_PREFIX) {
            Some(project) => scope
                .store
                .volumes
                .iter()
                .filter(|volume| volume.project() == Some(project))
                .map(|volume| volume.name.clone())
                .collect(),
            None => vec![key.clone()],
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn name_sheet(ui: &mut Ui, acts: &Acts, archive: &Path) {
    let stem = archive
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".gz").trim_end_matches(".tar"))
        .unwrap_or_default()
        .to_owned();
    let typed = Rc::new(RefCell::new(stem.clone()));
    let field = {
        let typed = Rc::clone(&typed);
        TextField::new(&tr!("Volume name"), &stem, move |text| {
            *typed.borrow_mut() = text.trim().to_owned();
        })
    };
    let acts = Rc::clone(acts);
    let archive = archive.to_path_buf();
    let message = tr!(
        "Creates a volume from {file}.",
        file = archive.display().to_string()
    );
    ui.present_sheet(
        Alert::new(&tr!("Import Volume"), &message, move |choice| {
            let name = typed.borrow().clone();
            if choice == "import" && !name.is_empty() {
                push(&acts, Act::Task(Task::ImportVolume(name, archive.clone())));
            }
        })
        .accessory(field)
        .primary(&tr!("Import"), "import")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}

impl Pane for Volumes {
    fn build(&mut self, acts: &Acts, palette: &Palette) -> (Vec<Box<dyn Widget>>, Box<dyn Widget>) {
        let import = tool(
            IMPORT_SYMBOL,
            text!("Import Volume"),
            palette,
            acts,
            Act::Command(IMPORT),
        );
        let prune = tool(
            PRUNE_SYMBOL,
            text!("Prune Unused Volumes"),
            palette,
            acts,
            Act::Command(PRUNE),
        );
        let remove = tool(
            REMOVE_SYMBOL,
            text!("Remove Selected"),
            palette,
            acts,
            Act::Command(REMOVE),
        );
        let inspector = tool(
            INSPECTOR_SYMBOL,
            text!("Inspector"),
            palette,
            acts,
            Act::Command(INSPECTOR),
        );
        let field = search(text!("Filter volumes"), &self.filter, acts);
        self.remove = Some(remove.id());
        self.search = Some(field.id());
        let select = Rc::clone(acts);
        let expand = Rc::clone(acts);
        let menu_acts = Rc::clone(acts);
        let table = Table::new(columns(self.inspector))
            .on_selection(move |keys| push(&select, Act::Select(keys.to_vec())))
            .on_expand(move |keys, expanded, _| push(&expand, Act::Expand(keys.to_vec(), expanded)))
            .context_menu(
                |keys| match keys {
                    [] => None,
                    [key] if !key.starts_with(PROJECT_PREFIX) => Some(Menu::new(vec![
                        MenuItem::new(&tr!("Copy Name"), COPY_NAME),
                        MenuItem::new(&tr!("Copy Mountpoint"), COPY_MOUNTPOINT),
                        MenuItem::separator(),
                        MenuItem::new(&tr!("Export as tar…"), EXPORT),
                        MenuItem::separator(),
                        MenuItem::new(&tr!("Remove…"), REMOVE),
                    ])),
                    _ => Some(Menu::new(vec![MenuItem::new(&tr!("Remove…"), REMOVE)])),
                },
                move |action, keys| push(&menu_acts, Act::Menu(action.to_owned(), keys.to_vec())),
            );
        self.table = Some(table.id());
        let split = Split::new(boxed(table), INSPECTOR_WIDTH);
        self.split = Some(split.id());
        self.inspected = None;
        (
            vec![
                boxed(import),
                boxed(prune),
                boxed(remove),
                boxed(inspector),
                boxed(field),
            ],
            boxed(split),
        )
    }

    fn fill(&mut self, ui: &mut Ui, scope: &Scope) {
        let rows = match self.inspector {
            true => narrowed(rows(scope, &self.filter)),
            false => rows(scope, &self.filter),
        };
        let expanded: Vec<String> = rows
            .iter()
            .filter(|row| !row.children.is_empty() && !self.collapsed.contains(&row.key))
            .map(|row| row.key.clone())
            .collect();
        if let Some(table) = self.table.and_then(|id| ui.widget_as::<Table>(id)) {
            table.set_rows(rows);
            table.set_expanded_rows(expanded);
        }
        let selected = !self.selected.is_empty();
        if let Some(button) = self.remove.and_then(|id| ui.widget_as::<Button>(id)) {
            button.set_enabled(selected);
        }
        let shown = self.inspector.then(|| inspected(scope, &self.selected));
        if shown != self.inspected {
            let side = shown
                .as_ref()
                .map(|shown| panel(&shown.title, &shown.note, &shown.facts));
            if let Some(split) = self.split.and_then(|id| ui.widget_as::<Split>(id)) {
                split.set_side(side);
            }
            self.inspected = shown;
        }
    }

    fn act(&mut self, act: Act) {
        match act {
            Act::Select(keys) => self.selected = keys,
            Act::Filter(text) => self.filter = text,
            Act::Expand(keys, expanded) => {
                for key in keys {
                    match expanded {
                        true => self.collapsed.remove(&key),
                        false => self.collapsed.insert(key),
                    };
                }
            }
            _ => {}
        }
    }

    fn search(&self) -> Option<WidgetId> {
        self.search
    }

    fn menu(&self, _scope: &Scope) -> Option<Entry> {
        let one = matches!(self.selected.as_slice(), [key] if !key.starts_with(PROJECT_PREFIX));
        Some(Entry::submenu(
            &tr!("Volume"),
            vec![
                Entry::item(&tr!("Import Volume…"), IMPORT),
                Entry::item(&tr!("Export as tar…"), EXPORT).enabled(one),
                Entry::item(&tr!("Copy Name"), COPY_NAME).enabled(one),
                Entry::separator(),
                Entry::item(&tr!("Remove…"), REMOVE).enabled(!self.selected.is_empty()),
                Entry::item(&tr!("Prune Unused…"), PRUNE),
            ],
        ))
    }

    fn view_entries(&self) -> Vec<Entry> {
        let label = match self.inspector {
            true => text!("Hide Inspector"),
            false => text!("Show Inspector"),
        };
        vec![Entry::item(&label.tr(), INSPECTOR).shortcut("Ctrl+I")]
    }

    fn command(&mut self, ui: &mut Ui, acts: &Acts, scope: &Scope, command: &str) -> bool {
        match command {
            IMPORT => push(acts, Act::Task(Task::ChooseArchive)),
            INSPECTOR => {
                self.inspector = !self.inspector;
                if let Some(table) = self.table.and_then(|id| ui.widget_as::<Table>(id)) {
                    table.set_columns(columns(self.inspector));
                }
                self.fill(ui, scope);
            }
            PRUNE => confirm(
                ui,
                acts,
                &tr!("Prune unused volumes?"),
                &tr!(
                    "Removes every volume no container uses, with its data. This cannot be undone."
                ),
                &tr!("Prune Unused Volumes"),
                Task::PruneVolumes,
            ),
            _ => {
                let keys = self.selected.clone();
                self.menu_action(ui, acts, scope, command, &keys);
            }
        }
        true
    }

    fn menu_action(
        &mut self,
        ui: &mut Ui,
        acts: &Acts,
        scope: &Scope,
        action: &str,
        keys: &[String],
    ) {
        let volume = match keys {
            [key] => scope
                .store
                .volumes
                .iter()
                .find(|volume| &volume.name == key),
            _ => None,
        };
        match (action, volume) {
            (COPY_NAME, Some(volume)) => ui.context().copy(&volume.name),
            (COPY_MOUNTPOINT, Some(volume)) => ui.context().copy(&volume.mountpoint),
            (EXPORT, Some(volume)) => {
                push(acts, Act::Task(Task::ExportVolume(volume.name.clone())))
            }
            (REMOVE, _) => {
                let names = names(scope, keys);
                let title = match names.as_slice() {
                    [] => return,
                    [only] => tr!("Remove “{name}”?", name = only.clone()),
                    _ => tr!("Remove {count} volumes?", count = names.len()),
                };
                confirm(
                    ui,
                    acts,
                    &title,
                    &tr!(
                        "Its data is deleted. A volume still used by a container cannot be removed."
                    ),
                    &tr!("Remove"),
                    Task::RemoveVolumes(names),
                );
            }
            _ => {}
        }
    }

    fn finished(&mut self, ui: &mut Ui, task: &Task, result: &Result<u64, String>) -> bool {
        let (title, message) = match (task, result) {
            (Task::PruneVolumes, Ok(reclaimed)) => (
                tr!("Pruned unused volumes"),
                tr!("Reclaimed {size}.", size = size(*reclaimed)),
            ),
            (Task::ImportVolume(name, _), Ok(_)) => (
                tr!("Imported into “{name}”", name = name.clone()),
                String::new(),
            ),
            (Task::ExportVolume(name), Ok(_)) => {
                (tr!("Exported “{name}”", name = name.clone()), String::new())
            }
            (Task::ImportVolume(name, _), Err(message)) => (
                tr!("Cannot import into “{name}”", name = name.clone()),
                message.clone(),
            ),
            (Task::ExportVolume(name), Err(message)) => (
                tr!("Cannot export “{name}”", name = name.clone()),
                message.clone(),
            ),
            (Task::RemoveVolumes(_) | Task::PruneVolumes, Err(message)) => {
                (tr!("Cannot remove every volume"), message.clone())
            }
            _ => return false,
        };
        tell(ui, &title, &message);
        true
    }
}
