use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, UNIX_EPOCH};

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
use crate::L10N;

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
        true => L10N.tr_args(
            "machine-snapshot-current",
            [("name", snapshot.name.clone().into())],
        ),
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
        Column::new(&L10N.tr("machine-snapshot-column-name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&L10N.tr("machine-snapshot-column-date"), DATE_WIDTH).unsortable(),
        Column::new(&L10N.tr("machine-snapshot-column-size"), SIZE_WIDTH)
            .numeric()
            .unsortable(),
    ])
    .on_selection(move |keys| push(&select, Intent::Snapshot(keys.to_vec())))
}

pub fn tool_button(intents: &Intents, key: &str, tool: Tool) -> Button {
    let intents = Rc::clone(intents);
    Button::new(&L10N.tr(key), move || {
        push(&intents, Intent::SnapshotTool(tool))
    })
}

fn free_port(fleet: &Fleet) -> u16 {
    (FIRST_PORT..FIRST_PORT.saturating_add(SSH_SEARCH))
        .find(|port| port_free(fleet, *port))
        .unwrap_or(FIRST_PORT)
}

pub fn draft_clone(fleet: &Fleet, entry: &Entry, snapshot: Option<String>) -> CloneDraft {
    let name = L10N.tr_args(
        "machine-clone-name",
        [("name", entry.name().to_owned().into())],
    );
    CloneDraft {
        name,
        port: free_port(fleet).to_string(),
        linked: false,
        snapshot,
    }
}

pub fn next_snapshot_name(entry: &Entry) -> String {
    let count = entry.listed.manifest.snapshots.len() + 1;
    L10N.tr_args("machine-snapshot-default", [("count", count.into())])
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
                true => "machine-snapshot-take-running",
                false => "machine-snapshot-take-stopped",
            };
            let field = Rc::clone(intents);
            let lines = vec![FormLine::Row(
                L10N.tr("machine-snapshot-name"),
                Box::new(TextField::new(
                    &L10N.tr("machine-snapshot-name"),
                    name,
                    move |text| push(&field, Intent::Text(text.to_owned())),
                )) as Box<dyn Widget>,
            )];
            alert(
                intents,
                &L10N.tr("machine-snapshot-take-title"),
                &L10N.tr(message),
            )
            .accessory(Form::new(&L10N.tr("machine-snapshot-name"), lines))
            .cancel(&L10N.tr("machine-cancel"), CANCEL)
            .primary(&L10N.tr("machine-snapshot-take"), CONTINUE)
        }
        Sheet::Revert(id) => {
            let snapshot = snapshot_named(entry, id)?;
            let message = match (entry.running(), snapshot.memory) {
                (false, true) => "machine-revert-resume-message",
                _ => "machine-revert-message",
            };
            alert(
                intents,
                &L10N.tr_args(
                    "machine-revert-title",
                    [("name", snapshot.name.clone().into())],
                ),
                &L10N.tr(message),
            )
            .cancel(&L10N.tr("machine-cancel"), CANCEL)
            .destructive(&L10N.tr("machine-revert"), CONTINUE)
        }
        Sheet::Delete(id) => {
            let snapshot = snapshot_named(entry, id)?;
            alert(
                intents,
                &L10N.tr_args(
                    "machine-delete-snapshot-title",
                    [("name", snapshot.name.clone().into())],
                ),
                &L10N.tr("machine-delete-snapshot-message"),
            )
            .cancel(&L10N.tr("machine-cancel"), CANCEL)
            .destructive(&L10N.tr("machine-delete-confirm"), CONTINUE)
        }
        Sheet::Clone(draft) => {
            let name_field = Rc::clone(intents);
            let port_field = Rc::clone(intents);
            let kind_field = Rc::clone(intents);
            let mut lines = vec![
                FormLine::Row(
                    L10N.tr("machine-name"),
                    Box::new(TextField::new(
                        &L10N.tr("machine-name"),
                        &draft.name,
                        move |text| push(&name_field, Intent::Text(text.to_owned())),
                    )) as Box<dyn Widget>,
                ),
                FormLine::Row(
                    L10N.tr("machine-ssh-port"),
                    Box::new(TextField::new(
                        &L10N.tr("machine-ssh-port"),
                        &draft.port,
                        move |text| push(&port_field, Intent::Port(text.to_owned())),
                    )),
                ),
            ];
            if draft.snapshot.is_none() && entry.listed.manifest.snapshots.is_empty() {
                let kinds = [
                    L10N.tr("machine-clone-full"),
                    L10N.tr("machine-clone-linked"),
                ];
                lines.push(FormLine::Row(
                    L10N.tr("machine-clone-kind"),
                    Box::new(PopUpButton::new(
                        &L10N.tr("machine-clone-kind"),
                        &kinds,
                        usize::from(draft.linked),
                        move |index| push(&kind_field, Intent::Linked(index == 1)),
                    )),
                ));
            }
            let message = match error {
                Some(error) => error.to_owned(),
                None => L10N.tr("machine-clone-message"),
            };
            alert(
                intents,
                &L10N.tr_args(
                    "machine-clone-title",
                    [("name", entry.name().to_owned().into())],
                ),
                &message,
            )
            .accessory(Form::new(&L10N.tr("machine-clone-kind"), lines))
            .cancel(&L10N.tr("machine-cancel"), CANCEL)
            .primary(&L10N.tr("machine-clone-create"), CONTINUE)
        }
    };
    Some(alert)
}

pub fn clone_request(draft: &CloneDraft, fleet: &Fleet) -> Result<NewClone, String> {
    let name = draft.name.trim().to_owned();
    if !valid_name(&name) {
        return Err(L10N.tr("machine-invalid-name"));
    }
    if fleet.entries.values().any(|entry| entry.name() == name)
        || fleet.jobs.values().any(|job| job.name == name)
    {
        return Err(L10N.tr_args("machine-name-taken", [("name", name.into())]));
    }
    let port = draft
        .port
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= LOWEST_PORT)
        .ok_or_else(|| L10N.tr("machine-invalid-port"))?;
    if !port_free(fleet, port) {
        return Err(L10N.tr_args("machine-port-taken", [("port", port.into())]));
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
