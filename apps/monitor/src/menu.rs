
use bande_l10n::{Text, text, tr};

use crate::model::{Column, Filter, Frequency, Tab};
use crate::signal::Stop;
use bande_app_menu::{Entry, edit_submenu};
use bande_toolkit::ui::{Menu, MenuItem};

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
    match how.id() {
        "hup" => tr!("Hangup (SIGHUP)"),
        "int" => tr!("Interrupt (SIGINT)"),
        "stop" => tr!("Pause (SIGSTOP)"),
        "cont" => tr!("Continue (SIGCONT)"),
        "usr1" => tr!("User 1 (SIGUSR1)"),
        "usr2" => tr!("User 2 (SIGUSR2)"),
        other => other.to_owned(),
    }
}

fn signal_item_title(how: Stop) -> String {
    match how {
        Stop::Resume => signal_title(how),
        _ => tr!("{signal}…", signal = signal_title(how)),
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

fn item(message: Text, command: MenuCommand) -> MenuItem {
    MenuItem::new(&message.tr(), command.action())
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
    let force_quit = item(text!("Force Quit…"), MenuCommand::ForceQuit).enabled(selected.stoppable);
    let mut items = vec![
        item(text!("Quit…"), MenuCommand::Quit)
            .shortcut("Ctrl+E")
            .enabled(selected.stoppable)
            .alternate(force_quit),
        MenuItem::separator(),
        item(text!("Inspect"), MenuCommand::Inspect)
            .shortcut("Ctrl+I")
            .enabled(selected.inspectable),
    ];
    if selected.files {
        items
            .push(item(text!("Show in Files"), MenuCommand::ShowInFiles).enabled(selected.located));
    }
    items.extend([
        MenuItem::separator(),
        MenuItem::submenu(
            &tr!("Send Signal to Process"),
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
        item(text!("Copy Name"), MenuCommand::CopyName),
        item(text!("Copy PID"), MenuCommand::CopyPid).enabled(selected.has_pid),
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

fn entry(message: Text, command: MenuCommand) -> Entry {
    Entry::item(&message.tr(), command.action())
}

pub fn app_menus(
    tabs: &[Tab],
    shown: Filter,
    frequency: Frequency,
    windowed: bool,
    files: bool,
) -> Vec<Entry> {
    let process = |message: Text, command: MenuCommand| entry(message, command).enabled(windowed);
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
            &tr!("Update Frequency"),
            Frequency::ALL
                .map(|choice| {
                    Entry::item(&choice.title(), MenuCommand::Frequency(choice).action())
                        .radio(choice == frequency)
                })
                .to_vec(),
        ),
        entry(text!("Show CPU Cores"), MenuCommand::ShowCores),
        Entry::separator(),
        process(text!("Quit Process…"), MenuCommand::Quit).shortcut("Ctrl+E"),
        process(text!("Force Quit Process…"), MenuCommand::ForceQuit),
        Entry::submenu(
            &tr!("Send Signal to Process"),
            Stop::SENDABLE
                .map(|how| Entry::item(&signal_item_title(how), MenuCommand::Signal(how).action()))
                .to_vec(),
        )
        .enabled(windowed),
        process(text!("Inspect"), MenuCommand::Inspect).shortcut("Ctrl+I"),
    ]);
    if files {
        view.extend([
            Entry::separator(),
            process(text!("Show in Files"), MenuCommand::ShowInFiles),
        ]);
    }
    vec![
        edit_submenu(vec![
            process(text!("Copy Name"), MenuCommand::CopyName),
            process(text!("Copy PID"), MenuCommand::CopyPid),
            Entry::separator(),
            process(text!("Find"), MenuCommand::Find).shortcut("Ctrl+F"),
        ]),
        Entry::submenu(&tr!("View"), view),
    ]
}
