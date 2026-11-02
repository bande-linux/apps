use bande_app_menu::Entry;
use bande_toolkit::ui::{Menu, MenuItem};

use crate::L10N;
use crate::engine::Action;

pub const LOGS: &str = "container-logs";
pub const TERMINAL: &str = "container-terminal";
pub const INSPECTOR: &str = "container-inspector";
pub const DELETE: &str = "container-delete";
pub const COPY_NAME: &str = "container-copy-name";
pub const COPY_ID: &str = "container-copy-id";
pub const COPY_RUN: &str = "container-copy-run";
pub const COPY_IP: &str = "container-copy-ip";
pub const COPY_URL: &str = "container-copy-url:";
pub const OPEN_URL: &str = "container-open-url:";
pub const UP: &str = "project-up";
pub const RESTART_PROJECT: &str = "project-restart";
pub const DOWN: &str = "project-down";

pub const ACTIONS: [(Action, &str); 6] = [
    (Action::Start, "container-start"),
    (Action::Stop, "container-stop"),
    (Action::Restart, "container-restart"),
    (Action::Pause, "container-pause"),
    (Action::Resume, "container-resume"),
    (Action::Kill, "container-kill"),
];

pub fn action_named(name: &str) -> Option<Action> {
    ACTIONS
        .iter()
        .find(|(_, action)| *action == name)
        .map(|(action, _)| *action)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Action {
        label: String,
        action: String,
        shortcut: Option<&'static str>,
        enabled: bool,
    },
    Separator,
    Submenu(String, Vec<Item>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub counts: Vec<(Action, usize)>,
    pub removable: usize,
    pub single: bool,
    pub running: bool,
    pub ip: bool,
    pub ports: Vec<u16>,
    pub logs: bool,
    pub compose: bool,
}

fn item(label: String, action: &str) -> Item {
    Item::Action {
        label,
        action: action.to_owned(),
        shortcut: None,
        enabled: true,
    }
}

fn with(item: Item, shortcut: Option<&'static str>, on: bool) -> Item {
    match item {
        Item::Action { label, action, .. } => Item::Action {
            label,
            action,
            shortcut,
            enabled: on,
        },
        other => other,
    }
}

fn counted(key: &str, count: usize) -> String {
    match count > 1 {
        true => L10N.tr_args(&format!("{key}-many"), [("count", count.into())]),
        false => L10N.tr(key),
    }
}

pub fn items(facts: &Facts) -> Vec<Item> {
    let mut items: Vec<Item> = ACTIONS
        .iter()
        .map(|(action, name)| {
            let count = facts
                .counts
                .iter()
                .find(|(counted, _)| counted == action)
                .map_or(0, |(_, count)| *count);
            with(item(counted(name, count), name), None, count > 0)
        })
        .collect();
    let single = facts.single;
    items.extend([
        Item::Separator,
        with(item(L10N.tr(LOGS), LOGS), Some("Ctrl+L"), facts.logs),
        with(
            item(L10N.tr(TERMINAL), TERMINAL),
            None,
            single && facts.running,
        ),
        Item::Separator,
    ]);
    let mut copies = vec![
        with(item(L10N.tr(COPY_NAME), COPY_NAME), None, single),
        with(item(L10N.tr(COPY_ID), COPY_ID), None, single),
        with(item(L10N.tr(COPY_RUN), COPY_RUN), None, single),
        with(item(L10N.tr(COPY_IP), COPY_IP), None, single && facts.ip),
    ];
    copies.extend(
        facts
            .ports
            .iter()
            .map(|port| item(url(*port), &format!("{COPY_URL}{port}"))),
    );
    items.push(Item::Submenu(L10N.tr("container-copy"), copies));
    items.extend(facts.ports.iter().map(|port| {
        item(
            L10N.tr_args("container-open", [("url", url(*port).into())]),
            &format!("{OPEN_URL}{port}"),
        )
    }));
    items.extend([
        Item::Separator,
        with(item(L10N.tr(UP), UP), None, facts.compose),
        with(
            item(L10N.tr(RESTART_PROJECT), RESTART_PROJECT),
            None,
            facts.compose,
        ),
        with(item(L10N.tr(DOWN), DOWN), None, facts.compose),
        Item::Separator,
        with(
            item(counted(DELETE, facts.removable), DELETE),
            Some("Delete"),
            facts.removable > 0,
        ),
    ]);
    items
}

pub fn url(port: u16) -> String {
    format!("http://localhost:{port}")
}

fn shown(items: Vec<Item>) -> Vec<Item> {
    let mut kept: Vec<Item> = Vec::new();
    for item in items {
        match item {
            Item::Action { enabled: false, .. } => {}
            Item::Separator if matches!(kept.last(), None | Some(Item::Separator)) => {}
            Item::Submenu(label, children) => {
                let children = shown(children);
                if !children.is_empty() {
                    kept.push(Item::Submenu(label, children));
                }
            }
            item => kept.push(item),
        }
    }
    if matches!(kept.last(), Some(Item::Separator)) {
        kept.pop();
    }
    kept
}

fn menu_item(item: Item) -> MenuItem {
    match item {
        Item::Action {
            label,
            action,
            shortcut,
            enabled,
        } => {
            let entry = MenuItem::new(&label, &action).enabled(enabled);
            match shortcut {
                Some(shortcut) => entry.shortcut(shortcut),
                None => entry,
            }
        }
        Item::Separator => MenuItem::separator(),
        Item::Submenu(label, children) => MenuItem::submenu(
            &label,
            Menu::new(children.into_iter().map(menu_item).collect()),
        ),
    }
}

pub fn context(items: Vec<Item>) -> Option<Menu> {
    let items = shown(items);
    (!items.is_empty()).then(|| Menu::new(items.into_iter().map(menu_item).collect()))
}

pub fn entry(item: Item) -> Entry {
    match item {
        Item::Action {
            label,
            action,
            shortcut,
            enabled,
        } => {
            let entry = Entry::item(&label, &action).enabled(enabled);
            match shortcut {
                Some(shortcut) => entry.shortcut(shortcut),
                None => entry,
            }
        }
        Item::Separator => Entry::separator(),
        Item::Submenu(label, children) => {
            Entry::submenu(&label, children.into_iter().map(entry).collect())
        }
    }
}
