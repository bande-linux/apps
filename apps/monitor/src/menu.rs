use bande_app_menu::{Entry, edit_submenu};
use bande_toolkit::ui::{Menu, MenuItem};

use crate::L10N;
use crate::model::{Column, Filter, Frequency, Tab};
use crate::signal::Stop;

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
    ShowCores,
    Filter(Filter),
    Frequency(Frequency),
    Signal(Stop),
}

const ACTIONS: [(MenuCommand, &str); 30] = [
    (MenuCommand::Quit, "quit"),
    (MenuCommand::ForceQuit, "force-quit"),
    (MenuCommand::ShowInFiles, "show-in-files"),
    (MenuCommand::Inspect, "inspect"),
    (MenuCommand::CopyName, "copy-name"),
    (MenuCommand::CopyPid, "copy-pid"),
    (MenuCommand::Find, "find"),
    (MenuCommand::Show(Tab::Cpu), "show-cpu"),
    (MenuCommand::Show(Tab::Memory), "show-memory"),
    (MenuCommand::Show(Tab::Energy), "show-energy"),
    (MenuCommand::Show(Tab::Disk), "show-disk"),
    (MenuCommand::Show(Tab::Network), "show-network"),
    (MenuCommand::Show(Tab::Gpu), "show-gpu"),
    (MenuCommand::ShowCores, "show-cores"),
    (MenuCommand::Filter(Filter::All), "filter-all"),
    (MenuCommand::Filter(Filter::Hierarchy), "filter-hierarchy"),
    (MenuCommand::Filter(Filter::Mine), "filter-mine"),
    (MenuCommand::Filter(Filter::System), "filter-system"),
    (MenuCommand::Filter(Filter::Active), "filter-active"),
    (MenuCommand::Filter(Filter::Apps), "filter-apps"),
    (MenuCommand::Filter(Filter::Selected), "filter-selected"),
    (
        MenuCommand::Frequency(Frequency::VeryOften),
        "frequency-very-often",
    ),
    (MenuCommand::Frequency(Frequency::Often), "frequency-often"),
    (
        MenuCommand::Frequency(Frequency::Normally),
        "frequency-normally",
    ),
    (MenuCommand::Signal(Stop::Hangup), "signal-hup"),
    (MenuCommand::Signal(Stop::Interrupt), "signal-int"),
    (MenuCommand::Signal(Stop::Pause), "signal-stop"),
    (MenuCommand::Signal(Stop::Resume), "signal-cont"),
    (MenuCommand::Signal(Stop::User1), "signal-usr1"),
    (MenuCommand::Signal(Stop::User2), "signal-usr2"),
];

pub(crate) fn signal_title(how: Stop) -> String {
    L10N.tr(&format!("signal-{}", how.id()))
}

fn signal_item_title(how: Stop) -> String {
    match how {
        Stop::Resume => signal_title(how),
        _ => L10N.tr_args("menu-signal-asks", [("signal", signal_title(how).into())]),
    }
}

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
    pub files: bool,
}

pub fn process_menu(selected: Selected) -> Menu {
    let force_quit = item("menu-force-quit", MenuCommand::ForceQuit).enabled(selected.stoppable);
    let mut items = vec![
        item("menu-quit", MenuCommand::Quit)
            .shortcut("Ctrl+E")
            .enabled(selected.stoppable)
            .alternate(force_quit),
        MenuItem::separator(),
        item("menu-inspect", MenuCommand::Inspect)
            .shortcut("Ctrl+I")
            .enabled(selected.inspectable),
    ];
    if selected.files {
        items.push(item("menu-show-in-files", MenuCommand::ShowInFiles).enabled(selected.located));
    }
    items.extend([
        MenuItem::separator(),
        MenuItem::submenu(
            &L10N.tr("menu-send-signal"),
            Menu::new(
                Stop::SENDABLE
                    .map(|how| {
                        MenuItem::new(&signal_item_title(how), MenuCommand::Signal(how).action())
                            .enabled(selected.stoppable)
                    })
                    .to_vec(),
            ),
        ),
        MenuItem::separator(),
        item("menu-copy-name", MenuCommand::CopyName),
        item("menu-copy-pid", MenuCommand::CopyPid).enabled(selected.has_pid),
    ]);
    Menu::new(items)
}

pub fn columns_menu(shown: &[Column]) -> Menu {
    Menu::new(
        std::iter::once(Column::Name)
            .chain(Column::CHOOSABLE)
            .map(|column| {
                MenuItem::new(&column.title(), &format!("{COLUMN_ACTION}{}", column.id()))
                    .checked(shown.contains(&column))
                    .enabled(column != Column::Name)
            })
            .collect(),
    )
}

fn entry(message: &str, command: MenuCommand) -> Entry {
    Entry::item(&L10N.tr(message), command.action())
}

pub fn app_menus(
    tabs: &[Tab],
    shown: Filter,
    frequency: Frequency,
    windowed: bool,
    files: bool,
) -> Vec<Entry> {
    let process = |message: &str, command: MenuCommand| entry(message, command).enabled(windowed);
    let mut view = tabs
        .iter()
        .zip(1..)
        .map(|(tab, digit)| {
            Entry::item(&tab.title(), MenuCommand::Show(*tab).action())
                .shortcut(&format!("Ctrl+{digit}"))
        })
        .collect::<Vec<_>>();
    view.push(Entry::separator());
    view.extend(Filter::ALL.map(|filter| {
        Entry::item(&filter.title(), MenuCommand::Filter(filter).action()).radio(filter == shown)
    }));
    view.extend([
        Entry::separator(),
        Entry::submenu(
            &L10N.tr("menu-frequency"),
            Frequency::ALL
                .map(|choice| {
                    Entry::item(&choice.title(), MenuCommand::Frequency(choice).action())
                        .radio(choice == frequency)
                })
                .to_vec(),
        ),
        entry("menu-show-cores", MenuCommand::ShowCores),
        Entry::separator(),
        process("menu-quit-process", MenuCommand::Quit).shortcut("Ctrl+E"),
        process("menu-force-quit-process", MenuCommand::ForceQuit),
        Entry::submenu(
            &L10N.tr("menu-send-signal"),
            Stop::SENDABLE
                .map(|how| Entry::item(&signal_item_title(how), MenuCommand::Signal(how).action()))
                .to_vec(),
        )
        .enabled(windowed),
        process("menu-inspect", MenuCommand::Inspect).shortcut("Ctrl+I"),
    ]);
    if files {
        view.extend([
            Entry::separator(),
            process("menu-show-in-files", MenuCommand::ShowInFiles),
        ]);
    }
    vec![
        edit_submenu(vec![
            process("menu-copy-name", MenuCommand::CopyName),
            process("menu-copy-pid", MenuCommand::CopyPid),
            Entry::separator(),
            process("menu-find", MenuCommand::Find).shortcut("Ctrl+F"),
        ]),
        Entry::submenu(&L10N.tr("menu-view"), view),
    ]
}
