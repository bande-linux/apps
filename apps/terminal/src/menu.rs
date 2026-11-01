use std::path::PathBuf;

use bande_app_menu::{Entry, Standard, app_submenu_quitting_on, window_submenu};
use bande_toolkit::ui::{Menu, MenuItem};
use bande_xdg::path_from_file_uri;

use crate::L10N;
use crate::keys::Command;

const COMMANDS: [(Command, &str, &str, &str); 21] = [
    (Command::Copy, "copy", "menu-copy", "Shift+Ctrl+C"),
    (Command::Paste, "paste", "menu-paste", "Shift+Ctrl+V"),
    (
        Command::SelectAll,
        "select-all",
        "menu-select-all",
        "Shift+Ctrl+A",
    ),
    (
        Command::ClearScrollback,
        "clear-scrollback",
        "menu-clear-scrollback",
        "Shift+Ctrl+K",
    ),
    (Command::BiggerText, "bigger", "menu-bigger", "Ctrl+="),
    (Command::SmallerText, "smaller", "menu-smaller", "Ctrl+-"),
    (
        Command::DefaultTextSize,
        "default-size",
        "menu-default-size",
        "Ctrl+0",
    ),
    (Command::NewTab, "new-tab", "menu-new-tab", "Shift+Ctrl+T"),
    (
        Command::NewWindow,
        "new-window",
        "menu-new-window",
        "Shift+Ctrl+N",
    ),
    (Command::NextTab, "next-tab", "menu-next-tab", "Ctrl+PgDn"),
    (
        Command::PreviousTab,
        "previous-tab",
        "menu-previous-tab",
        "Ctrl+PgUp",
    ),
    (Command::Find, "find", "menu-find", "Shift+Ctrl+F"),
    (
        Command::FindNext,
        "find-next",
        "menu-find-next",
        "Shift+Ctrl+G",
    ),
    (
        Command::SplitSideBySide,
        "split-side-by-side",
        "menu-split-side-by-side",
        "Shift+Ctrl+D",
    ),
    (
        Command::SplitStacked,
        "split-stacked",
        "menu-split-stacked",
        "Shift+Ctrl+E",
    ),
    (
        Command::NextPane,
        "next-pane",
        "menu-next-pane",
        "Shift+Ctrl+]",
    ),
    (
        Command::PreviousPane,
        "previous-pane",
        "menu-previous-pane",
        "Shift+Ctrl+[",
    ),
    (
        Command::Settings,
        "settings",
        "menu-settings",
        "Shift+Ctrl+,",
    ),
    (
        Command::PreviousPrompt,
        "previous-prompt",
        "menu-previous-prompt",
        "Shift+Ctrl+Up",
    ),
    (
        Command::NextPrompt,
        "next-prompt",
        "menu-next-prompt",
        "Shift+Ctrl+Down",
    ),
    (
        Command::FindPrevious,
        "find-previous",
        "menu-find-previous",
        "Shift+Ctrl+H",
    ),
];

pub const CLOSE_TAB_SHORTCUT: &str = "Shift+Ctrl+W";
pub const QUIT_SHORTCUT: &str = "Shift+Ctrl+Q";
const OPEN_LINK: &str = "open-link:";
const COPY_LINK: &str = "copy-link:";
const CLOSE_TAB: &str = "close-tab:";
const CLOSE_OTHER_TABS: &str = "close-other-tabs:";
const SHOW_IN_FILES: &str = "show-in-files";
const REVEAL: &str = "reveal:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuCommand {
    Run(Command),
    OpenLink(String),
    CopyLink(String),
    ShowInFiles,
    Reveal(PathBuf),
    CloseTab(u32),
    CloseOtherTabs(u32),
}

impl MenuCommand {
    pub fn action(&self) -> String {
        match self {
            Self::Run(command) => COMMANDS
                .iter()
                .find(|(known, ..)| known == command)
                .map(|(_, action, ..)| (*action).to_owned())
                .unwrap_or_default(),
            Self::OpenLink(uri) => format!("{OPEN_LINK}{uri}"),
            Self::CopyLink(uri) => format!("{COPY_LINK}{uri}"),
            Self::ShowInFiles => SHOW_IN_FILES.to_owned(),
            Self::Reveal(path) => format!("{REVEAL}{}", path.display()),
            Self::CloseTab(id) => format!("{CLOSE_TAB}{id}"),
            Self::CloseOtherTabs(id) => format!("{CLOSE_OTHER_TABS}{id}"),
        }
    }

    pub fn parse(action: &str) -> Option<Self> {
        if let Some((command, ..)) = COMMANDS.iter().find(|(_, known, ..)| *known == action) {
            return Some(Self::Run(*command));
        }
        if action == SHOW_IN_FILES {
            return Some(Self::ShowInFiles);
        }
        let tab = |rest: &str| rest.parse().ok();
        if let Some(uri) = action.strip_prefix(OPEN_LINK) {
            Some(Self::OpenLink(uri.to_owned()))
        } else if let Some(uri) = action.strip_prefix(COPY_LINK) {
            Some(Self::CopyLink(uri.to_owned()))
        } else if let Some(path) = action.strip_prefix(REVEAL) {
            Some(Self::Reveal(PathBuf::from(path)))
        } else if let Some(id) = action.strip_prefix(CLOSE_TAB) {
            tab(id).map(Self::CloseTab)
        } else {
            action
                .strip_prefix(CLOSE_OTHER_TABS)
                .and_then(tab)
                .map(Self::CloseOtherTabs)
        }
    }
}

fn command_entry(command: Command) -> Option<(String, String, &'static str)> {
    COMMANDS
        .iter()
        .find(|(known, ..)| *known == command)
        .map(|(_, action, message, shortcut)| (L10N.tr(message), (*action).to_owned(), *shortcut))
}

fn command_item(command: Command) -> MenuItem {
    match command_entry(command) {
        Some((title, action, shortcut)) => MenuItem::new(&title, &action).shortcut(shortcut),
        None => MenuItem::separator(),
    }
}

fn item(message: &str, command: MenuCommand) -> MenuItem {
    MenuItem::new(&L10N.tr(message), &command.action())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalContext {
    pub link: Option<String>,
    pub has_selection: bool,
    pub knows_directory: bool,
    pub tab: u32,
}

pub fn terminal_menu(context: &TerminalContext) -> Menu {
    let mut items = Vec::new();
    if let Some(uri) = &context.link {
        match path_from_file_uri(uri) {
            Some(path) => items.extend([
                item("menu-open", MenuCommand::OpenLink(uri.clone())),
                item(
                    "menu-copy-path",
                    MenuCommand::CopyLink(path.display().to_string()),
                ),
                item("menu-show-in-files", MenuCommand::Reveal(path)),
            ]),
            None => items.extend([
                item("menu-open-link", MenuCommand::OpenLink(uri.clone())),
                item("menu-copy-link", MenuCommand::CopyLink(uri.clone())),
            ]),
        }
        items.push(MenuItem::separator());
    }
    items.extend([
        command_item(Command::Copy).enabled(context.has_selection),
        command_item(Command::Paste),
        command_item(Command::SelectAll),
        MenuItem::separator(),
        command_item(Command::ClearScrollback),
        MenuItem::separator(),
        item("menu-show-in-files", MenuCommand::ShowInFiles).enabled(context.knows_directory),
        MenuItem::separator(),
        command_item(Command::NewTab),
        command_item(Command::NewWindow),
        MenuItem::separator(),
        command_item(Command::SplitSideBySide),
        command_item(Command::SplitStacked),
        MenuItem::separator(),
        item("menu-close-tab", MenuCommand::CloseTab(context.tab)).shortcut(CLOSE_TAB_SHORTCUT),
    ]);
    Menu::new(items)
}

pub fn tab_menu(tab: Option<u32>, tabs: usize) -> Menu {
    let mut items = vec![command_item(Command::NewTab)];
    if let Some(tab) = tab {
        items.extend([
            MenuItem::separator(),
            item("menu-close-tab", MenuCommand::CloseTab(tab)),
            item("menu-close-other-tabs", MenuCommand::CloseOtherTabs(tab)).enabled(tabs > 1),
        ]);
    }
    Menu::new(items)
}

fn command_entries(commands: &[Option<Command>]) -> Vec<Entry> {
    commands
        .iter()
        .map(|command| {
            command
                .and_then(command_entry)
                .map_or_else(Entry::separator, |(title, action, shortcut)| {
                    Entry::item(&title, &action).shortcut(shortcut)
                })
        })
        .collect()
}

pub fn app_menus() -> Vec<Entry> {
    let mut shell = command_entries(&[Some(Command::NewWindow), Some(Command::NewTab), None]);
    shell.extend([
        Entry::item(
            &L10N.tr("menu-show-in-files"),
            &MenuCommand::ShowInFiles.action(),
        ),
        Entry::separator(),
        Entry::item(&L10N.tr("menu-close-tab"), Standard::CloseWindow.action())
            .shortcut(CLOSE_TAB_SHORTCUT),
    ]);
    vec![
        app_submenu_quitting_on(
            &L10N.tr("title"),
            command_entries(&[Some(Command::Settings)]),
            QUIT_SHORTCUT,
        ),
        Entry::submenu(&L10N.tr("menu-shell"), shell),
        Entry::submenu(
            &L10N.tr("menu-edit"),
            command_entries(&[
                Some(Command::Copy),
                Some(Command::Paste),
                Some(Command::SelectAll),
                None,
                Some(Command::ClearScrollback),
                None,
                Some(Command::Find),
                Some(Command::FindNext),
                Some(Command::FindPrevious),
                None,
                Some(Command::PreviousPrompt),
                Some(Command::NextPrompt),
            ]),
        ),
        Entry::submenu(
            &L10N.tr("menu-view"),
            command_entries(&[
                Some(Command::BiggerText),
                Some(Command::SmallerText),
                Some(Command::DefaultTextSize),
                None,
                Some(Command::SplitSideBySide),
                Some(Command::SplitStacked),
            ]),
        ),
        window_submenu(command_entries(&[
            Some(Command::NextTab),
            Some(Command::PreviousTab),
            None,
            Some(Command::NextPane),
            Some(Command::PreviousPane),
        ])),
    ]
}

#[cfg(test)]
mod tests {
    use bande_toolkit::ui::parse_shortcut;

    use super::COMMANDS;
    use crate::keys::shortcut;

    #[test]
    fn every_menu_shortcut_runs_the_command_beside_it() {
        for (command, _, _, keys) in COMMANDS {
            let pressed = parse_shortcut(keys).map(|(key, modifiers)| shortcut(key, modifiers));
            assert_eq!(pressed, Some(Some(command)), "{keys}");
        }
    }
}
