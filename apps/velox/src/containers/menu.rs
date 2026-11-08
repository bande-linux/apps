use bande_l10n::tr;

use crate::engine::Action;
use bande_app_menu::Entry;
use bande_toolkit::ui::{Menu, MenuItem};

pub const LOGS: &str = "container-logs";
pub const TERMINAL: &str = "container-terminal";
pub const SHELL: &str = "container-shell";
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
    pub terminal: bool,
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

fn label(key: &str, count: usize) -> String {
    match (key, count > 1) {
        ("container-start", true) => tr!("Start {count} Containers", count = count),
        ("container-stop", true) => tr!("Stop {count} Containers", count = count),
        ("container-restart", true) => tr!("Restart {count} Containers", count = count),
        ("container-pause", true) => tr!("Pause {count} Containers", count = count),
        ("container-resume", true) => tr!("Resume {count} Containers", count = count),
        ("container-kill", true) => tr!("Kill {count} Containers", count = count),
        ("container-delete", true) => tr!("Delete {count} Containers…", count = count),
        ("container-start", _) => tr!("Start"),
        ("container-stop", _) => tr!("Stop"),
        ("container-restart", _) => tr!("Restart"),
        ("container-pause", _) => tr!("Pause"),
        ("container-resume", _) => tr!("Resume"),
        ("container-kill", _) => tr!("Kill"),
        ("container-delete", _) => tr!("Delete…"),
        ("container-logs", _) => tr!("Show Logs"),
        ("container-terminal", _) => tr!("Open in Terminal"),
        ("container-shell", _) => tr!("Open Shell"),
        ("container-copy-name", _) => tr!("Name"),
        ("container-copy-id", _) => tr!("Container ID"),
        ("container-copy-run", _) => tr!("As docker run Command"),
        ("container-copy-ip", _) => tr!("IP Address"),
        ("project-up", _) => tr!("Up Project"),
        ("project-restart", _) => tr!("Restart Project"),
        ("project-down", _) => tr!("Down Project…"),
        (other, _) => other.to_owned(),
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
            with(item(label(name, count), name), None, count > 0)
        })
        .collect();
    let single = facts.single;
    items.extend([
        Item::Separator,
        with(item(label(LOGS, 1), LOGS), Some("Ctrl+L"), facts.logs),
        with(item(label(SHELL, 1), SHELL), None, single && facts.running),
    ]);
    if facts.terminal {
        items.push(with(
            item(label(TERMINAL, 1), TERMINAL),
            None,
            single && facts.running,
        ));
    }
    items.push(Item::Separator);
    let mut copies = vec![
        with(item(label(COPY_NAME, 1), COPY_NAME), None, single),
        with(item(label(COPY_ID, 1), COPY_ID), None, single),
        with(item(label(COPY_RUN, 1), COPY_RUN), None, single),
        with(item(label(COPY_IP, 1), COPY_IP), None, single && facts.ip),
    ];
    copies.extend(
        facts
            .ports
            .iter()
            .map(|port| item(url(*port), &format!("{COPY_URL}{port}"))),
    );
    items.push(Item::Submenu(tr!("Copy"), copies));
    items.extend(facts.ports.iter().map(|port| {
        item(
            tr!("Open {url}", url = url(*port)),
            &format!("{OPEN_URL}{port}"),
        )
    }));
    items.extend([
        Item::Separator,
        with(item(label(UP, 1), UP), None, facts.compose),
        with(
            item(label(RESTART_PROJECT, 1), RESTART_PROJECT),
            None,
            facts.compose,
        ),
        with(item(label(DOWN, 1), DOWN), None, facts.compose),
        Item::Separator,
        with(
            item(label(DELETE, facts.removable), DELETE),
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
