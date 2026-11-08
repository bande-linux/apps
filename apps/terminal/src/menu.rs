use std::path::PathBuf;

use bande_l10n::{Text, text, tr};
use bande_app::Menus;
use bande_app_menu::Entry;
use bande_terminal_view::Command;
use bande_toolkit::ui::{Menu, MenuItem};
use bande_xdg::path_from_file_uri;

const COMMANDS: [(Command, &str, Text, Option<&str>); 22] = [
    (Command::Copy, "copy", text!("Copy"), Some("Shift+Ctrl+C")),
    (
        Command::Paste,
        "paste",
        text!("Paste"),
        Some("Shift+Ctrl+V"),
    ),
    (
        Command::SelectAll,
        "select-all",
        text!("Select All"),
        Some("Shift+Ctrl+A"),
    ),
    (
        Command::ClearScrollback,
        "clear-scrollback",
        text!("Clear Scrollback"),
        Some("Shift+Ctrl+K"),
    ),
    (
        Command::BiggerText,
        "bigger",
        text!("Bigger"),
        Some("Ctrl+="),
    ),
    (
        Command::SmallerText,
        "smaller",
        text!("Smaller"),
        Some("Ctrl+-"),
    ),
    (
        Command::DefaultTextSize,
        "default-size",
        text!("Default Size"),
        Some("Ctrl+0"),
    ),
    (
        Command::NewTab,
        "new-tab",
        text!("New Tab"),
        Some("Shift+Ctrl+T"),
    ),
    (
        Command::SelectOutput,
        "select-output",
        text!("Select Command Output"),
        None,
    ),
    (Command::EditTitle, "edit-title", text!("Edit Title"), None),
    (
        Command::NewWindow,
        "new-window",
        text!("New Window"),
        Some("Shift+Ctrl+N"),
    ),
    (
        Command::NextTab,
        "next-tab",
        text!("Show Next Tab"),
        Some("Ctrl+PgDn"),
    ),
    (
        Command::PreviousTab,
        "previous-tab",
        text!("Show Previous Tab"),
        Some("Ctrl+PgUp"),
    ),
    (Command::Find, "find", text!("Find…"), Some("Shift+Ctrl+F")),
    (
        Command::FindNext,
        "find-next",
        text!("Find Next"),
        Some("Shift+Ctrl+G"),
    ),
    (
        Command::SplitSideBySide,
        "split-side-by-side",
        text!("Split Side by Side"),
        Some("Shift+Ctrl+D"),
    ),
    (
        Command::SplitStacked,
        "split-stacked",
        text!("Split Top and Bottom"),
        Some("Shift+Ctrl+E"),
    ),
    (
        Command::NextPane,
        "next-pane",
        text!("Select Next Pane"),
        Some("Shift+Ctrl+]"),
    ),
    (
        Command::PreviousPane,
        "previous-pane",
        text!("Select Previous Pane"),
        Some("Shift+Ctrl+["),
    ),
    (
        Command::PreviousPrompt,
        "previous-prompt",
        text!("Jump to Previous Command"),
        None,
    ),
    (
        Command::NextPrompt,
        "next-prompt",
        text!("Jump to Next Command"),
        None,
    ),
    (
        Command::FindPrevious,
        "find-previous",
        text!("Find Previous"),
        Some("Shift+Ctrl+H"),
    ),
];

const CLOSE_TAB_SHORTCUT: &str = "Shift+Ctrl+W";
const OPEN_LINK: &str = "open-link:";
const COPY_LINK: &str = "copy-link:";
const CLOSE_TAB: &str = "close-tab:";
const CLOSE_OTHER_TABS: &str = "close-other-tabs:";
const SHOW_IN_FILES: &str = "show-in-files";
const LOOK_UP: &str = "look-up";
const LOOK_UP_LONGEST: usize = 100;

pub fn look_up_text(selection: Option<String>) -> Option<String> {
    let text = selection?.trim().to_owned();
    (text.chars().any(char::is_alphanumeric) && text.chars().count() <= LOOK_UP_LONGEST)
        .then_some(text)
}
pub const CLOSE_WINDOW: &str = "close-window";
const REVEAL: &str = "reveal:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuCommand {
    Run(Command),
    OpenLink(String),
    CopyLink(String),
    ShowInFiles,
    LookUp,
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
            Self::LookUp => LOOK_UP.to_owned(),
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
        if action == LOOK_UP {
            return Some(Self::LookUp);
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

fn command_entry(command: Command) -> Option<(String, String, Option<&'static str>)> {
    COMMANDS
        .iter()
        .find(|(known, ..)| *known == command)
        .map(|(_, action, message, shortcut)| (message.tr(), (*action).to_owned(), *shortcut))
}

fn command_item(command: Command) -> MenuItem {
    match command_entry(command) {
        Some((title, action, Some(shortcut))) => MenuItem::new(&title, &action).shortcut(shortcut),
        Some((title, action, None)) => MenuItem::new(&title, &action),
        None => MenuItem::separator(),
    }
}

fn item(message: Text, command: MenuCommand) -> MenuItem {
    MenuItem::new(&message.tr(), &command.action())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalContext {
    pub link: Option<String>,
    pub has_selection: bool,
    pub look_up: Option<String>,
    pub knows_directory: bool,
    pub files: bool,
    pub tab: u32,
}

pub fn terminal_menu(context: &TerminalContext) -> Menu {
    let mut items = Vec::new();
    if let Some(uri) = &context.link {
        match path_from_file_uri(uri) {
            Some(path) => {
                items.extend([
                    item(text!("Open"), MenuCommand::OpenLink(uri.clone())),
                    item(
                        text!("Copy Path"),
                        MenuCommand::CopyLink(path.display().to_string()),
                    ),
                ]);
                if context.files {
                    items.push(item(text!("Show in Files"), MenuCommand::Reveal(path)));
                }
            }
            None => items.extend([
                item(text!("Open Link"), MenuCommand::OpenLink(uri.clone())),
                item(text!("Copy Link"), MenuCommand::CopyLink(uri.clone())),
            ]),
        }
        items.push(MenuItem::separator());
    }
    if let Some(word) = &context.look_up {
        let title = tr!("Look Up “{word}”", word = word.as_str());
        items.extend([
            MenuItem::new(&title, &MenuCommand::LookUp.action()),
            MenuItem::separator(),
        ]);
    }
    items.extend([
        command_item(Command::Copy).enabled(context.has_selection),
        command_item(Command::Paste),
        command_item(Command::SelectAll),
        MenuItem::separator(),
        command_item(Command::ClearScrollback),
        MenuItem::separator(),
    ]);
    if context.files {
        items.extend([
            item(text!("Show in Files"), MenuCommand::ShowInFiles).enabled(context.knows_directory),
            MenuItem::separator(),
        ]);
    }
    items.extend([
        command_item(Command::NewTab),
        command_item(Command::NewWindow),
        MenuItem::separator(),
        command_item(Command::SplitSideBySide),
        command_item(Command::SplitStacked),
        MenuItem::separator(),
        item(text!("Close Tab"), MenuCommand::CloseTab(context.tab)).shortcut(CLOSE_TAB_SHORTCUT),
    ]);
    Menu::new(items)
}

pub fn tab_menu(tab: Option<u32>, tabs: usize) -> Menu {
    let mut items = vec![command_item(Command::NewTab)];
    if let Some(tab) = tab {
        items.extend([
            MenuItem::separator(),
            item(text!("Close Tab"), MenuCommand::CloseTab(tab)),
            item(text!("Close Other Tabs"), MenuCommand::CloseOtherTabs(tab)).enabled(tabs > 1),
        ]);
    }
    Menu::new(items)
}

fn command_entries(commands: &[Option<Command>]) -> Vec<Entry> {
    commands
        .iter()
        .map(|command| {
            command.and_then(command_entry).map_or_else(
                Entry::separator,
                |(title, action, shortcut)| {
                    let entry = Entry::item(&title, &action);
                    match shortcut {
                        Some(shortcut) => entry.shortcut(shortcut),
                        None => entry,
                    }
                },
            )
        })
        .collect()
}

const PRINT_SHORTCUT: &str = "Shift+Ctrl+P";

pub fn app_menus(windowed: bool, files: bool) -> Menus {
    let mut shell = command_entries(&[
        Some(Command::NewWindow),
        Some(Command::NewTab),
        None,
        Some(Command::EditTitle),
        None,
    ]);
    if files {
        shell.extend([
            Entry::item(&tr!("Show in Files"), &MenuCommand::ShowInFiles.action()),
            Entry::separator(),
        ]);
    }
    shell.extend([
        bande_app_menu::print_item(windowed).shortcut(PRINT_SHORTCUT),
        Entry::separator(),
        Entry::item(&tr!("Close Window"), CLOSE_WINDOW).enabled(windowed),
    ]);
    Menus {
        menus: vec![
            Entry::submenu(&tr!("Shell"), shell),
            Entry::submenu(
                &tr!("Edit"),
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
                    Some(Command::SelectOutput),
                ]),
            ),
            Entry::submenu(
                &tr!("View"),
                command_entries(&[
                    Some(Command::BiggerText),
                    Some(Command::SmallerText),
                    Some(Command::DefaultTextSize),
                    None,
                    Some(Command::SplitSideBySide),
                    Some(Command::SplitStacked),
                ]),
            ),
        ],
        window: command_entries(&[
            Some(Command::NextTab),
            Some(Command::PreviousTab),
            None,
            Some(Command::NextPane),
            Some(Command::PreviousPane),
        ]),
    }
}
