use unbound_app_menu::{Entry, app_submenu, close_window, window_submenu};
use unbound_toolkit::ui::{Menu, MenuItem};

use crate::L10N;
use crate::model::{Column, Tab};

pub const COLUMN_ACTION: &str = "column:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Quit,
    ForceQuit,
    ShowInFiles,
    Inspect,
    CopyName,
    CopyPid,
    Find,
    Show(Tab),
}

const ACTIONS: [(MenuCommand, &str); 12] = [
    (MenuCommand::Quit, "quit"),
    (MenuCommand::ForceQuit, "force-quit"),
    (MenuCommand::ShowInFiles, "show-in-files"),
    (MenuCommand::Inspect, "inspect"),
    (MenuCommand::CopyName, "copy-name"),
    (MenuCommand::CopyPid, "copy-pid"),
    (MenuCommand::Find, "find"),
    (MenuCommand::Show(Tab::Cpu), "show-cpu"),
    (MenuCommand::Show(Tab::Memory), "show-memory"),
    (MenuCommand::Show(Tab::Disk), "show-disk"),
    (MenuCommand::Show(Tab::Network), "show-network"),
    (MenuCommand::Show(Tab::Gpu), "show-gpu"),
];

impl MenuCommand {
    pub fn action(self) -> &'static str {
        ACTIONS
            .iter()
            .find(|(command, _)| *command == self)
            .map_or("", |(_, action)| action)
    }

    pub fn parse(action: &str) -> Option<Self> {
        ACTIONS
            .iter()
            .find(|(_, known)| *known == action)
            .map(|(command, _)| *command)
    }
}

fn item(message: &str, command: MenuCommand) -> MenuItem {
    MenuItem::new(&L10N.tr(message), command.action())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selected {
    pub stoppable: bool,
    pub located: bool,
    pub has_pid: bool,
    pub inspectable: bool,
}

pub fn process_menu(selected: Selected) -> Menu {
    let force_quit = item("menu-force-quit", MenuCommand::ForceQuit).enabled(selected.stoppable);
    Menu::new(vec![
        item("menu-quit", MenuCommand::Quit)
            .shortcut("Alt+Super+Q")
            .enabled(selected.stoppable)
            .alternate(force_quit),
        MenuItem::separator(),
        item("menu-inspect", MenuCommand::Inspect)
            .shortcut("Super+I")
            .enabled(selected.inspectable),
        item("menu-show-in-files", MenuCommand::ShowInFiles)
            .shortcut("Shift+Super+R")
            .enabled(selected.located),
        MenuItem::separator(),
        item("menu-copy-name", MenuCommand::CopyName),
        item("menu-copy-pid", MenuCommand::CopyPid).enabled(selected.has_pid),
    ])
}

pub fn columns_menu(tab: Tab, shown: &[Column]) -> Menu {
    Menu::new(
        std::iter::once(Column::Name)
            .chain(Column::CHOOSABLE)
            .map(|column| {
                MenuItem::new(
                    &column.title(tab),
                    &format!("{COLUMN_ACTION}{}", column.id()),
                )
                .checked(shown.contains(&column))
                .enabled(column != Column::Name)
            })
            .collect(),
    )
}

pub fn force_quit_menu(located: bool) -> Menu {
    Menu::new(vec![
        item("menu-force-quit", MenuCommand::ForceQuit).shortcut("Enter"),
        MenuItem::separator(),
        item("menu-show-in-files", MenuCommand::ShowInFiles).enabled(located),
    ])
}

fn entry(message: &str, command: MenuCommand) -> Entry {
    Entry::item(&L10N.tr(message), command.action())
}

pub fn app_menus(tabs: &[Tab]) -> Vec<Entry> {
    let mut view = tabs
        .iter()
        .zip(1..)
        .map(|(tab, digit)| {
            Entry::item(&tab.title(), MenuCommand::Show(*tab).action())
                .shortcut(&format!("Super+{digit}"))
        })
        .collect::<Vec<_>>();
    view.extend([
        Entry::separator(),
        entry("menu-quit-process", MenuCommand::Quit).shortcut("Alt+Super+Q"),
        entry("menu-force-quit-process", MenuCommand::ForceQuit),
        entry("menu-inspect", MenuCommand::Inspect).shortcut("Super+I"),
        Entry::separator(),
        entry("menu-show-in-files", MenuCommand::ShowInFiles).shortcut("Shift+Super+R"),
    ]);
    vec![
        app_submenu(&L10N.tr("window-title"), Vec::new()),
        Entry::submenu(
            &L10N.tr("menu-edit"),
            vec![
                entry("menu-copy-name", MenuCommand::CopyName),
                entry("menu-copy-pid", MenuCommand::CopyPid),
                Entry::separator(),
                entry("menu-find", MenuCommand::Find).shortcut("Super+F"),
            ],
        ),
        Entry::submenu(&L10N.tr("menu-view"), view),
        window_submenu(vec![close_window()]),
    ]
}
