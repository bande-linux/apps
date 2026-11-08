use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, UNIX_EPOCH};

use bande_l10n::{Text, text, tr};
use bande_toolkit::ui::{
    Alert, Button, Cell, Column, Form, FormLine, PopUpButton, Row, Table, TextField, Widget,
    WidgetId,
};
use bande_vm::{CloneKind, NewClone, Snapshot};
use jiff::Timestamp;

use super::backend::{Act, SnapshotOp};
use super::fleet::{Entry, Fleet};
use super::sheet::{CANCEL, CONTINUE, port_free, valid_name};
use super::{Intent, Intents, push};

const NAME_WIDTH: f32 = 260.0;
const DATE_WIDTH: f32 = 170.0;
const SIZE_WIDTH: f32 = 100.0;
const LOWEST_PORT: u16 = 1024;
const FIRST_PORT: u16 = 2222;
const SSH_SEARCH: u16 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Take,
    Revert,
    Delete,
    Clone,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolIds {
    pub table: WidgetId,
    pub revert: WidgetId,
    pub delete: WidgetId,
    pub clone: WidgetId,
    pub take: WidgetId,
}

pub enum Sheet {
    Take { name: String },
    Revert(String),
    Delete(String),
    Clone(CloneDraft),
}

pub struct CloneDraft {
    pub name: String,
    pub port: String,
    pub linked: bool,
    pub snapshot: Option<String>,
}

fn date(seconds: u64) -> String {
    let at = UNIX_EPOCH + Duration::from_secs(seconds);
    Timestamp::try_from(at)
        .map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn row(all: &[Snapshot], current: Option<&str>, snapshot: &Snapshot) -> Row {
    let name = match current == Some(snapshot.id.as_str()) {
        true => tr!("{name} (current)", name = snapshot.name.clone()),
        false => snapshot.name.clone(),
    };
    let size = match snapshot.memory {
        true => Cell::new(bande_fs::format_size(snapshot.memory_bytes)).tabular(),
        false => Cell::new("—").muted(),
    };
    let children = all
        .iter()
        .filter(|child| child.parent.as_deref() == Some(snapshot.id.as_str()))
        .map(|child| row(all, current, child))
        .collect();
    Row::new(
        snapshot.id.as_str(),
        vec![
            Cell::new(name),
            Cell::new(date(snapshot.created)).muted(),
            size,
        ],
    )
    .children(children)
}

pub fn rows(entry: &Entry) -> Vec<Row> {
    let manifest = &entry.listed.manifest;
    let known: HashMap<&str, &Snapshot> = manifest
        .snapshots
        .iter()
        .map(|snapshot| (snapshot.id.as_str(), snapshot))
        .collect();
    manifest
        .snapshots
        .iter()
        .filter(|snapshot| {
            snapshot
                .parent
                .as_deref()
                .is_none_or(|parent| !known.contains_key(parent))
        })
        .map(|root| {
            row(
                &manifest.snapshots,
                manifest.current_snapshot.as_deref(),
                root,
            )
        })
        .collect()
}

pub fn expanded(entry: &Entry) -> Vec<String> {
    entry
        .listed
        .manifest
        .snapshots
        .iter()
        .map(|snapshot| snapshot.id.clone())
        .collect()
}

pub fn table(intents: &Intents) -> Table {
    let select = Rc::clone(intents);
    Table::new(vec![
        Column::new(&tr!("Name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&tr!("Date"), DATE_WIDTH).unsortable(),
        Column::new(&tr!("Memory"), SIZE_WIDTH)
            .numeric()
            .unsortable(),
    ])
    .on_selection(move |keys| push(&select, Intent::Snapshot(keys.to_vec())))
}

pub fn tool_button(intents: &Intents, title: &str, tool: Tool) -> Button {
    let intents = Rc::clone(intents);
    Button::new(title, move || push(&intents, Intent::SnapshotTool(tool)))
}

fn free_port(fleet: &Fleet) -> u16 {
    (FIRST_PORT..FIRST_PORT.saturating_add(SSH_SEARCH))
        .find(|port| port_free(fleet, *port))
        .unwrap_or(FIRST_PORT)
}

pub fn draft_clone(fleet: &Fleet, entry: &Entry, snapshot: Option<String>) -> CloneDraft {
    let name = tr!("{name}-copy", name = entry.name().to_owned());
    CloneDraft {
        name,
        port: free_port(fleet).to_string(),
        linked: false,
        snapshot,
    }
}

pub fn next_snapshot_name(entry: &Entry) -> String {
    let count = entry.listed.manifest.snapshots.len() + 1;
    tr!("Snapshot {count}", count = count)
}

fn snapshot_named<'a>(entry: &'a Entry, id: &str) -> Option<&'a Snapshot> {
    entry
        .listed
        .manifest
        .snapshots
        .iter()
        .find(|snapshot| snapshot.id == id)
}

fn alert(intents: &Intents, title: &str, message: &str) -> Alert {
    let intents = Rc::clone(intents);
    Alert::new(title, message, move |choice| {
        push(&intents, Intent::Choice(choice.to_owned()))
    })
}

pub fn present(
    sheet: &Sheet,
    intents: &Intents,
    entry: &Entry,
    error: Option<&str>,
) -> Option<Alert> {
    let alert = match sheet {
        Sheet::Take { name } => {
            let message = match entry.running() {
                true => text!("Saves the disks and memory of the running machine."),
                false => text!("Saves the disks of the stopped machine."),
            };
            let field = Rc::clone(intents);
            let lines = vec![FormLine::Row(
                tr!("Snapshot name"),
                Box::new(TextField::new(&tr!("Snapshot name"), name, move |text| {
                    push(&field, Intent::Text(text.to_owned()))
                })) as Box<dyn Widget>,
            )];
            alert(intents, &tr!("Take Snapshot"), &message.tr())
                .accessory(Form::new(&tr!("Snapshot name"), lines))
                .cancel(&tr!("Cancel"), CANCEL)
                .primary(&tr!("Take Snapshot"), CONTINUE)
        }
        Sheet::Revert(id) => {
            let snapshot = snapshot_named(entry, id)?;
            let message = match (entry.running(), snapshot.memory) {
                (false, true) => {
                    text!("Changes since it are lost. The machine starts again from that moment.")
                }
                _ => text!("Changes since it are lost."),
            };
            alert(
                intents,
                &tr!("Revert to “{name}”?", name = snapshot.name.clone()),
                &message.tr(),
            )
            .cancel(&tr!("Cancel"), CANCEL)
            .destructive(&tr!("Revert"), CONTINUE)
        }
        Sheet::Delete(id) => {
            let snapshot = snapshot_named(entry, id)?;
            alert(
                intents,
                &tr!("Delete “{name}”?", name = snapshot.name.clone()),
                &tr!("The machine itself stays as it is. Disk space is reused by later snapshots."),
            )
            .cancel(&tr!("Cancel"), CANCEL)
            .destructive(&tr!("Delete"), CONTINUE)
        }
        Sheet::Clone(draft) => {
            let name_field = Rc::clone(intents);
            let port_field = Rc::clone(intents);
            let kind_field = Rc::clone(intents);
            let mut lines = vec![
                FormLine::Row(
                    tr!("Name"),
                    Box::new(TextField::new(&tr!("Name"), &draft.name, move |text| {
                        push(&name_field, Intent::Text(text.to_owned()))
                    })) as Box<dyn Widget>,
                ),
                FormLine::Row(
                    tr!("SSH port on this computer"),
                    Box::new(TextField::new(
                        &tr!("SSH port on this computer"),
                        &draft.port,
                        move |text| push(&port_field, Intent::Port(text.to_owned())),
                    )),
                ),
            ];
            if draft.snapshot.is_none() && entry.listed.manifest.snapshots.is_empty() {
                let kinds = [tr!("Full copy"), tr!("Linked, saves space")];
                lines.push(FormLine::Row(
                    tr!("Disk"),
                    Box::new(PopUpButton::new(
                        &tr!("Disk"),
                        &kinds,
                        usize::from(draft.linked),
                        move |index| push(&kind_field, Intent::Linked(index == 1)),
                    )),
                ));
            }
            let message = match error {
                Some(error) => error.to_owned(),
                None => tr!("The clone gets its own address, identity and disk."),
            };
            alert(
                intents,
                &tr!("Clone “{name}”", name = entry.name().to_owned()),
                &message,
            )
            .accessory(Form::new(&tr!("Disk"), lines))
            .cancel(&tr!("Cancel"), CANCEL)
            .primary(&tr!("Clone"), CONTINUE)
        }
    };
    Some(alert)
}

pub fn clone_request(draft: &CloneDraft, fleet: &Fleet) -> Result<NewClone, String> {
    let name = draft.name.trim().to_owned();
    if !valid_name(&name) {
        return Err(tr!(
            "Use lowercase letters, digits and hyphens for the name."
        ));
    }
    if fleet.entries.values().any(|entry| entry.name() == name)
        || fleet.jobs.values().any(|job| job.name == name)
    {
        return Err(tr!("A machine named “{name}” already exists.", name = name));
    }
    let port = draft
        .port
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= LOWEST_PORT)
        .ok_or_else(|| tr!("Choose a port from 1024 to 65535."))?;
    if !port_free(fleet, port) {
        return Err(tr!("Port {port} is in use.", port = port));
    }
    Ok(NewClone {
        name,
        ssh_port: port,
        kind: match draft.linked {
            true => CloneKind::Linked,
            false => CloneKind::Full,
        },
        snapshot: draft.snapshot.clone(),
    })
}

pub fn snapshot_op(sheet: Sheet) -> Option<SnapshotOp> {
    match sheet {
        Sheet::Take { name } => {
            let name = name.trim().to_owned();
            (!name.is_empty()).then_some(SnapshotOp::Take(name))
        }
        Sheet::Revert(id) => Some(SnapshotOp::Revert(id)),
        Sheet::Delete(id) => Some(SnapshotOp::Delete(id)),
        Sheet::Clone(_) => None,
    }
}

pub fn can_revert(entry: &Entry, id: &str) -> bool {
    entry.can(Act::Revert)
        && snapshot_named(entry, id).is_some_and(|snapshot| snapshot.memory || !entry.running())
}
