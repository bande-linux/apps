use unbound_toolkit::ui::{Menu, MenuItem};

use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Quit,
    ForceQuit,
    ShowInFiles,
    CopyName,
    CopyPid,
}

const ACTIONS: [(MenuCommand, &str); 5] = [
    (MenuCommand::Quit, "quit"),
    (MenuCommand::ForceQuit, "force-quit"),
    (MenuCommand::ShowInFiles, "show-in-files"),
    (MenuCommand::CopyName, "copy-name"),
    (MenuCommand::CopyPid, "copy-pid"),
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
}

pub fn process_menu(selected: Selected) -> Menu {
    let force_quit = item("menu-force-quit", MenuCommand::ForceQuit).enabled(selected.stoppable);
    Menu::new(vec![
        item("menu-quit", MenuCommand::Quit)
            .shortcut("Alt+Super+Q")
            .enabled(selected.stoppable)
            .alternate(force_quit),
        MenuItem::separator(),
        item("menu-show-in-files", MenuCommand::ShowInFiles)
            .shortcut("Shift+Super+R")
            .enabled(selected.located),
        MenuItem::separator(),
        item("menu-copy-name", MenuCommand::CopyName),
        item("menu-copy-pid", MenuCommand::CopyPid).enabled(selected.has_pid),
    ])
}

pub fn force_quit_menu(located: bool) -> Menu {
    Menu::new(vec![
        item("menu-force-quit", MenuCommand::ForceQuit).shortcut("Enter"),
        MenuItem::separator(),
        item("menu-show-in-files", MenuCommand::ShowInFiles).enabled(located),
    ])
}
