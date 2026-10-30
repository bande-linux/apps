use std::cmp::Ordering;
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::L10N;
use crate::apps::{Catalog, Group, GroupKey};
use crate::format::{format_bytes, format_cpu_time, format_percent, format_rate};
use crate::sampler::{Process, Snapshot};
use crate::signal::{SignalError, Stop, stop};

pub const HISTORY: usize = 60;
const MISSING: &str = "\u{2013}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Cpu,
    Memory,
    Disk,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Cpu,
    CpuTime,
    Threads,
    Pid,
    User,
    Memory,
    Resident,
    Read,
    Written,
    ReadTotal,
    WrittenTotal,
    Received,
    Sent,
    ReceivedTotal,
    SentTotal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    Group(GroupKey),
    Process { pid: u32, start_ticks: u64 },
    Interface(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: RowKey,
    pub name: String,
    pub icon: Option<String>,
    pub cells: Vec<String>,
    pub responding: bool,
    pub revealed: bool,
    pub children: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Graph {
    Stacked { lower: Vec<f32>, upper: Vec<f32> },
    Filled(Vec<f32>),
    Mirrored { up: Vec<f32>, down: Vec<f32> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopTarget {
    pub name: String,
    pub count: usize,
    pub roots: Vec<(u32, u64)>,
    pub processes: Vec<(u32, u64)>,
}

fn merged(targets: impl IntoIterator<Item = StopTarget>) -> Option<StopTarget> {
    targets.into_iter().reduce(|mut all, target| {
        all.count += target.count;
        for root in target.roots {
            if !all.roots.contains(&root) {
                all.roots.push(root);
            }
        }
        for process in target.processes {
            if !all.processes.contains(&process) {
                all.processes.push(process);
            }
        }
        all
    })
}

#[derive(Debug, Default)]
struct Unresponsive(Vec<(u32, String)>);

impl Unresponsive {
    fn set(&mut self, pid: u32, app_id: &str, responding: bool) -> bool {
        let known = self.0.iter().position(|(known, _)| *known == pid);
        match (known, responding) {
            (Some(index), true) => {
                self.0.remove(index);
                true
            }
            (None, false) => {
                self.0.push((pid, app_id.to_owned()));
                true
            }
            _ => false,
        }
    }

    fn process_responds(&self, pid: u32) -> bool {
        !self.0.iter().any(|(known, _)| *known == pid)
    }

    fn group_responds(&self, group: &Group, processes: &[Process]) -> bool {
        group
            .members
            .iter()
            .all(|member| self.process_responds(processes[*member].pid))
            && !self
                .0
                .iter()
                .any(|(_, app_id)| matches!(&group.key, GroupKey::App(id) if id == app_id))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Measure {
    Number(f64),
    Text(String),
    Missing,
}

impl Measure {
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(a), Self::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
            (Self::Missing, Self::Missing) => Ordering::Equal,
            (Self::Missing, _) => Ordering::Less,
            (_, Self::Missing) => Ordering::Greater,
            (Self::Number(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) => Ordering::Greater,
        }
    }
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Cpu, Tab::Memory, Tab::Disk, Tab::Network];

    pub fn title(self) -> String {
        match self {
            Tab::Cpu => L10N.tr("tab-cpu"),
            Tab::Memory => L10N.tr("tab-memory"),
            Tab::Disk => L10N.tr("tab-disk"),
            Tab::Network => L10N.tr("tab-network"),
        }
    }

    pub fn columns(self) -> &'static [Column] {
        match self {
            Tab::Cpu => &[
                Column::Name,
                Column::Cpu,
                Column::CpuTime,
                Column::Threads,
                Column::Pid,
                Column::User,
            ],
            Tab::Memory => &[
                Column::Name,
                Column::Memory,
                Column::Resident,
                Column::Threads,
                Column::Pid,
                Column::User,
            ],
            Tab::Disk => &[
                Column::Name,
                Column::Read,
                Column::Written,
                Column::ReadTotal,
                Column::WrittenTotal,
                Column::Pid,
            ],
            Tab::Network => &[
                Column::Name,
                Column::Received,
                Column::Sent,
                Column::ReceivedTotal,
                Column::SentTotal,
            ],
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    fn id(self) -> &'static str {
        match self {
            Tab::Cpu => "cpu",
            Tab::Memory => "memory",
            Tab::Disk => "disk",
            Tab::Network => "network",
        }
    }

    pub fn has_column_choice(self) -> bool {
        self != Tab::Network
    }

    fn default_sort(self) -> Column {
        match self {
            Tab::Cpu => Column::Cpu,
            Tab::Memory => Column::Memory,
            Tab::Disk => Column::Written,
            Tab::Network => Column::Received,
        }
    }
}

impl Column {
    pub const CHOOSABLE: [Column; 11] = [
        Column::Cpu,
        Column::CpuTime,
        Column::Threads,
        Column::Pid,
        Column::User,
        Column::Memory,
        Column::Resident,
        Column::Read,
        Column::Written,
        Column::ReadTotal,
        Column::WrittenTotal,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Cpu => "cpu",
            Column::CpuTime => "cpu-time",
            Column::Threads => "threads",
            Column::Pid => "pid",
            Column::User => "user",
            Column::Memory => "memory",
            Column::Resident => "resident",
            Column::Read => "read",
            Column::Written => "written",
            Column::ReadTotal => "read-total",
            Column::WrittenTotal => "written-total",
            Column::Received => "received",
            Column::Sent => "sent",
            Column::ReceivedTotal => "received-total",
            Column::SentTotal => "sent-total",
        }
    }

    pub fn parse(id: &str) -> Option<Column> {
        Column::CHOOSABLE
            .into_iter()
            .chain([Column::Name])
            .find(|column| column.id() == id)
    }

    pub fn title(self, tab: Tab) -> String {
        match self {
            Column::Name if tab == Tab::Network => L10N.tr("column-interface"),
            Column::Name => L10N.tr("column-name"),
            Column::Cpu => L10N.tr("column-cpu"),
            Column::CpuTime => L10N.tr("column-cpu-time"),
            Column::Threads => L10N.tr("column-threads"),
            Column::Pid => L10N.tr("column-pid"),
            Column::User => L10N.tr("column-user"),
            Column::Memory => L10N.tr("column-memory"),
            Column::Resident => L10N.tr("column-resident"),
            Column::Read => L10N.tr("column-read"),
            Column::Written => L10N.tr("column-written"),
            Column::ReadTotal => L10N.tr("column-read-total"),
            Column::WrittenTotal => L10N.tr("column-written-total"),
            Column::Received => L10N.tr("column-received"),
            Column::Sent => L10N.tr("column-sent"),
            Column::ReceivedTotal => L10N.tr("column-received-total"),
            Column::SentTotal => L10N.tr("column-sent-total"),
        }
    }

    pub fn numeric(self) -> bool {
        !matches!(self, Column::Name | Column::User)
    }

    fn text(self, measure: &Measure) -> String {
        let Measure::Number(value) = measure else {
            return match measure {
                Measure::Text(text) => text.clone(),
                _ => MISSING.to_owned(),
            };
        };
        match self {
            Column::Cpu => format_percent(*value),
            Column::CpuTime => format_cpu_time(*value),
            Column::Memory
            | Column::Resident
            | Column::ReadTotal
            | Column::WrittenTotal
            | Column::ReceivedTotal
            | Column::SentTotal => format_bytes(*value as u64),
            Column::Read | Column::Written | Column::Received | Column::Sent => format_rate(*value),
            _ => format!("{value:.0}"),
        }
    }

    fn measure(self, processes: &[&Process], name: &str, leader: &Process) -> Measure {
        let sum = |value: fn(&Process) -> f64| {
            Measure::Number(processes.iter().map(|process| value(process)).sum())
        };
        let disk = |value: fn(&crate::sampler::Rates) -> f64| {
            let known = processes
                .iter()
                .filter_map(|process| process.disk.as_ref())
                .collect::<Vec<_>>();
            if known.is_empty() {
                Measure::Missing
            } else {
                Measure::Number(known.into_iter().map(value).sum())
            }
        };
        match self {
            Column::Name => Measure::Text(name.to_owned()),
            Column::Cpu => sum(|process| f64::from(process.cpu_percent)),
            Column::CpuTime => sum(|process| process.cpu_seconds),
            Column::Threads => sum(|process| f64::from(process.threads)),
            Column::Pid => Measure::Number(f64::from(leader.pid)),
            Column::User => Measure::Text(leader.identity.user.clone()),
            Column::Memory => {
                sum(|process| process.proportional.unwrap_or(process.resident) as f64)
            }
            Column::Resident => sum(|process| process.resident as f64),
            Column::Read => disk(|rates| rates.read_per_second),
            Column::Written => disk(|rates| rates.written_per_second),
            Column::ReadTotal => disk(|rates| rates.read_total as f64),
            Column::WrittenTotal => disk(|rates| rates.written_total as f64),
            Column::Received | Column::Sent | Column::ReceivedTotal | Column::SentTotal => {
                Measure::Missing
            }
        }
    }
}

struct Candidate {
    row: Row,
    measure: Measure,
    name: String,
    pid: u32,
}

fn order(a: &Candidate, b: &Candidate, descending: bool) -> Ordering {
    let primary = a.measure.compare(&b.measure);
    let primary = if descending {
        primary.reverse()
    } else {
        primary
    };
    primary
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.pid.cmp(&b.pid))
}

fn push_limited<T>(history: &mut VecDeque<T>, value: T) {
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(value);
}

fn group_target(group: &Group, processes: &[Process], name: String) -> StopTarget {
    let id = |process: &Process| (process.pid, process.start_ticks);
    let members = group
        .members
        .iter()
        .map(|member| &processes[*member])
        .collect::<Vec<_>>();
    StopTarget {
        name,
        count: 1,
        roots: members
            .iter()
            .filter(|process| !members.iter().any(|parent| parent.pid == process.ppid))
            .map(|process| id(process))
            .collect(),
        processes: members.into_iter().map(id).collect(),
    }
}

pub fn stop_target(target: &StopTarget, how: Stop) -> Result<(), SignalError> {
    let chosen = match how {
        Stop::Quit => &target.roots,
        Stop::ForceQuit => &target.processes,
    };
    let mut outcome = Ok(());
    for (pid, start_ticks) in chosen {
        match stop(*pid, *start_ticks, how) {
            Ok(()) | Err(SignalError::Exited) => {}
            Err(error) => {
                if outcome.is_ok() {
                    outcome = Err(error);
                }
            }
        }
    }
    outcome
}

pub struct AppList {
    catalog: Catalog,
    uid: u32,
    processes: Vec<Process>,
    apps: Vec<Group>,
    selection: Vec<GroupKey>,
    unresponsive: Unresponsive,
}

impl AppList {
    pub fn new(catalog: Catalog, uid: u32) -> Self {
        Self {
            catalog,
            uid,
            processes: Vec::new(),
            apps: Vec::new(),
            selection: Vec::new(),
            unresponsive: Unresponsive::default(),
        }
    }

    pub fn set_responding(&mut self, pid: u32, app_id: &str, responding: bool) -> bool {
        self.unresponsive.set(pid, app_id, responding)
    }

    pub fn update(&mut self, snapshot: Snapshot) {
        let uid = self.uid;
        let processes = snapshot.processes;
        let mut apps = self
            .catalog
            .group(&processes)
            .into_iter()
            .filter(|group| group.app.is_some() && processes[group.leader].identity.uid == uid)
            .collect::<Vec<_>>();
        apps.sort_by_cached_key(|group| group.name.to_lowercase());
        self.processes = processes;
        self.apps = apps;
        let selection = std::mem::take(&mut self.selection);
        self.set_selection(selection);
    }

    pub fn rows(&self) -> impl Iterator<Item = AppRow<'_>> {
        self.apps.iter().map(|group| AppRow {
            key: &group.key,
            name: group.name.as_str(),
            icon: group.app.as_ref().and_then(|app| app.icon.as_deref()),
            responding: self.unresponsive.group_responds(group, &self.processes),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    pub fn selection(&self) -> &[GroupKey] {
        &self.selection
    }

    pub fn set_selection(&mut self, mut selection: Vec<GroupKey>) {
        selection.retain(|key| self.apps.iter().any(|group| &group.key == key));
        self.selection = selection;
    }

    fn selected(&self) -> impl Iterator<Item = &Group> {
        self.apps
            .iter()
            .filter(|group| self.selection.contains(&group.key))
    }

    pub fn selected_entry(&self) -> Option<PathBuf> {
        let [key] = self.selection.as_slice() else {
            return None;
        };
        let group = self.apps.iter().find(|group| &group.key == key)?;
        group.app.as_ref().map(|app| app.path.clone())
    }

    pub fn stop_target(&self) -> Option<StopTarget> {
        merged(
            self.selected()
                .map(|group| group_target(group, &self.processes, group.name.clone())),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppRow<'a> {
    pub key: &'a GroupKey,
    pub name: &'a str,
    pub icon: Option<&'a str>,
    pub responding: bool,
}
pub struct Monitor {
    catalog: Catalog,
    snapshot: Snapshot,
    groups: Vec<Group>,
    tab: Tab,
    sort: Column,
    descending: bool,
    selection: Vec<RowKey>,
    search: String,
    rows: Vec<Row>,
    unresponsive: Unresponsive,
    chosen: [Vec<Column>; 4],
    cpu: VecDeque<(f32, f32)>,
    memory: VecDeque<f32>,
    disk: VecDeque<(f64, f64)>,
    network: VecDeque<(f64, f64)>,
}

fn find_row<'a>(rows: &'a [Row], key: &RowKey) -> Option<&'a Row> {
    rows.iter().find_map(|row| match &row.key == key {
        true => Some(row),
        false => find_row(&row.children, key),
    })
}

impl Monitor {
    pub fn new(catalog: Catalog) -> Self {
        Self {
            catalog,
            snapshot: Snapshot::default(),
            groups: Vec::new(),
            tab: Tab::Cpu,
            sort: Tab::Cpu.default_sort(),
            descending: true,
            selection: Vec::new(),
            search: String::new(),
            rows: Vec::new(),
            unresponsive: Unresponsive::default(),
            chosen: Tab::ALL.map(|tab| tab.columns().to_vec()),
            cpu: VecDeque::new(),
            memory: VecDeque::new(),
            disk: VecDeque::new(),
            network: VecDeque::new(),
        }
    }

    pub fn update(&mut self, snapshot: Snapshot) {
        self.groups = self.catalog.group(&snapshot.processes);
        if snapshot.seconds > 0.0 {
            push_limited(&mut self.cpu, (snapshot.cpu.system, snapshot.cpu.user));
            let memory = &snapshot.memory;
            push_limited(
                &mut self.memory,
                if memory.total > 0 {
                    memory.used as f32 / memory.total as f32
                } else {
                    0.0
                },
            );
            push_limited(
                &mut self.disk,
                (
                    snapshot.disk.read_per_second,
                    snapshot.disk.written_per_second,
                ),
            );
            push_limited(
                &mut self.network,
                snapshot
                    .interfaces
                    .iter()
                    .fold((0.0, 0.0), |(received, sent), interface| {
                        (
                            received + interface.received_per_second,
                            sent + interface.sent_per_second,
                        )
                    }),
            );
        }
        self.snapshot = snapshot;
        self.rebuild();
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            let sort = tab.default_sort();
            (self.sort, self.descending) = match self.chosen[tab.index()].contains(&sort) {
                true => (sort, true),
                false => (Column::Name, false),
            };
            self.rebuild();
        }
    }

    pub fn columns(&self) -> &[Column] {
        &self.chosen[self.tab.index()]
    }

    pub fn toggle_column(&mut self, column: Column) -> bool {
        if column == Column::Name || !self.tab.has_column_choice() {
            return false;
        }
        let tab = self.tab;
        let chosen = &mut self.chosen[tab.index()];
        match chosen.iter().position(|shown| *shown == column) {
            Some(index) => {
                chosen.remove(index);
            }
            None => {
                chosen.push(column);
                chosen.sort_by_key(|shown| {
                    std::iter::once(Column::Name)
                        .chain(Column::CHOOSABLE)
                        .position(|known| known == *shown)
                });
            }
        }
        if !self.chosen[tab.index()].contains(&self.sort) {
            let fallback = tab.default_sort();
            self.sort = match self.chosen[tab.index()].contains(&fallback) {
                true => fallback,
                false => Column::Name,
            };
            self.descending = self.sort != Column::Name;
        }
        self.rebuild();
        true
    }

    pub fn chosen_columns(&self) -> String {
        Tab::ALL
            .iter()
            .filter(|tab| tab.has_column_choice())
            .map(|tab| {
                let ids = self.chosen[tab.index()]
                    .iter()
                    .map(|column| column.id())
                    .collect::<Vec<_>>();
                format!("{} {}\n", tab.id(), ids.join(" "))
            })
            .collect()
    }

    pub fn choose_columns(&mut self, text: &str) {
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let Some(tab) = words
                .next()
                .and_then(|id| Tab::ALL.into_iter().find(|tab| tab.id() == id))
                .filter(|tab| tab.has_column_choice())
            else {
                continue;
            };
            let mut columns = vec![Column::Name];
            columns.extend(
                words
                    .filter_map(Column::parse)
                    .filter(|column| *column != Column::Name),
            );
            self.chosen[tab.index()] = columns;
        }
        if !self.columns().contains(&self.sort) {
            self.sort = Column::Name;
            self.descending = false;
        }
        self.rebuild();
    }

    pub fn sort(&self) -> (Column, bool) {
        (self.sort, self.descending)
    }

    pub fn set_sort(&mut self, column: Column, descending: bool) {
        self.sort = column;
        self.descending = descending;
        self.rebuild();
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    pub fn set_search(&mut self, search: &str) {
        self.search = search.to_owned();
        self.rebuild();
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selection(&self) -> &[RowKey] {
        &self.selection
    }

    pub fn set_selection(&mut self, selection: Vec<RowKey>) {
        self.selection = selection;
        self.keep_shown_selection();
    }

    pub fn set_responding(&mut self, pid: u32, app_id: &str, responding: bool) -> bool {
        let changed = self.unresponsive.set(pid, app_id, responding);
        if changed {
            self.rebuild();
        }
        changed
    }

    pub fn measures_proportional_memory(&self) -> bool {
        self.tab == Tab::Memory
    }

    fn process(&self, pid: u32, start_ticks: u64) -> Option<&Process> {
        self.snapshot
            .processes
            .iter()
            .find(|process| process.pid == pid && process.start_ticks == start_ticks)
    }

    fn group(&self, key: &GroupKey) -> Option<&Group> {
        self.groups.iter().find(|group| &group.key == key)
    }

    fn target(&self, key: &RowKey) -> Option<StopTarget> {
        match key {
            RowKey::Group(key) => {
                let group = self.group(key)?;
                Some(group_target(
                    group,
                    &self.snapshot.processes,
                    self.group_name(group),
                ))
            }
            RowKey::Process { pid, start_ticks } => {
                let process = self.process(*pid, *start_ticks)?;
                let id = (process.pid, process.start_ticks);
                Some(StopTarget {
                    name: process.identity.name.clone(),
                    count: 1,
                    roots: vec![id],
                    processes: vec![id],
                })
            }
            RowKey::Interface(_) => None,
        }
    }

    pub fn stop_target(&self) -> Option<StopTarget> {
        merged(self.selection.iter().filter_map(|key| self.target(key)))
    }

    fn process_of(&self, key: &RowKey) -> Option<&Process> {
        match key {
            RowKey::Group(key) => self
                .group(key)
                .map(|group| &self.snapshot.processes[group.leader]),
            RowKey::Process { pid, start_ticks } => self.process(*pid, *start_ticks),
            RowKey::Interface(_) => None,
        }
    }

    pub fn selected_pids(&self) -> Vec<u32> {
        self.selection
            .iter()
            .filter_map(|key| self.process_of(key))
            .map(|process| process.pid)
            .collect()
    }

    pub fn selected_names(&self) -> Vec<String> {
        self.selection
            .iter()
            .filter_map(|key| find_row(&self.rows, key))
            .map(|row| row.name.clone())
            .collect()
    }

    pub fn location(&self) -> Option<PathBuf> {
        let [key] = self.selection.as_slice() else {
            return None;
        };
        if let RowKey::Group(key) = key
            && let Some(app) = self.group(key).and_then(|group| group.app.as_ref())
        {
            return Some(app.path.clone());
        }
        self.process_of(key)?.identity.executable.clone()
    }

    fn group_name(&self, group: &Group) -> String {
        match group.key {
            GroupKey::Kernel => L10N.tr("group-kernel"),
            _ => group.name.clone(),
        }
    }

    fn matches(&self, process: &Process, query: &str) -> bool {
        process.identity.name.to_lowercase().contains(query) || process.pid.to_string() == query
    }

    fn keep_shown_selection(&mut self) {
        let rows = &self.rows;
        self.selection.retain(|key| find_row(rows, key).is_some());
    }

    fn rebuild(&mut self) {
        let columns = self.chosen[self.tab.index()].clone();
        let columns = columns.as_slice();
        let sort = self.sort;
        let descending = self.descending;
        let query = self.search.trim().to_lowercase();
        let processes = &self.snapshot.processes;
        self.rows = if self.tab == Tab::Network {
            self.interface_rows(&query)
        } else {
            let mut groups = Vec::new();
            for group in &self.groups {
                let name = self.group_name(group);
                let members = group
                    .members
                    .iter()
                    .map(|member| &processes[*member])
                    .collect::<Vec<_>>();
                let group_matches = query.is_empty() || name.to_lowercase().contains(&query);
                let shown = members
                    .iter()
                    .copied()
                    .filter(|process| group_matches || self.matches(process, &query))
                    .collect::<Vec<_>>();
                if shown.is_empty() {
                    continue;
                }
                let leader = &processes[group.leader];
                let mut children = Vec::new();
                if members.len() > 1 {
                    for process in &shown {
                        let name = &process.identity.name;
                        children.push(Candidate {
                            row: Row {
                                key: RowKey::Process {
                                    pid: process.pid,
                                    start_ticks: process.start_ticks,
                                },
                                name: name.clone(),
                                icon: None,
                                cells: columns
                                    .iter()
                                    .map(|column| {
                                        column.text(&column.measure(&[process], name, process))
                                    })
                                    .collect(),
                                responding: self.unresponsive.process_responds(process.pid),
                                revealed: false,
                                children: Vec::new(),
                            },
                            measure: sort.measure(&[process], name, process),
                            name: name.clone(),
                            pid: process.pid,
                        });
                    }
                    children.sort_by(|a, b| order(a, b, descending));
                }
                let row = Row {
                    key: RowKey::Group(group.key.clone()),
                    name: name.clone(),
                    icon: group.app.as_ref().and_then(|app| app.icon.clone()),
                    cells: columns
                        .iter()
                        .map(|column| column.text(&column.measure(&shown, &name, leader)))
                        .collect(),
                    responding: self.unresponsive.group_responds(group, processes),
                    revealed: !group_matches && !children.is_empty(),
                    children: children.into_iter().map(|child| child.row).collect(),
                };
                groups.push(Candidate {
                    row,
                    measure: sort.measure(&shown, &name, leader),
                    name,
                    pid: leader.pid,
                });
            }
            groups.sort_by(|a, b| order(a, b, descending));
            groups.into_iter().map(|group| group.row).collect()
        };
        self.keep_shown_selection();
    }

    fn interface_rows(&self, query: &str) -> Vec<Row> {
        let mut candidates = self
            .snapshot
            .interfaces
            .iter()
            .filter(|interface| interface.name.to_lowercase().contains(query))
            .map(|interface| {
                let measure = |column: Column| match column {
                    Column::Name => Measure::Text(interface.name.clone()),
                    Column::Received => Measure::Number(interface.received_per_second),
                    Column::Sent => Measure::Number(interface.sent_per_second),
                    Column::ReceivedTotal => Measure::Number(interface.received_total as f64),
                    Column::SentTotal => Measure::Number(interface.sent_total as f64),
                    _ => Measure::Missing,
                };
                Candidate {
                    row: Row {
                        key: RowKey::Interface(interface.name.clone()),
                        name: interface.name.clone(),
                        icon: None,
                        cells: Tab::Network
                            .columns()
                            .iter()
                            .map(|column| column.text(&measure(*column)))
                            .collect(),
                        responding: true,
                        revealed: false,
                        children: Vec::new(),
                    },
                    measure: measure(self.sort),
                    name: interface.name.clone(),
                    pid: 0,
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| order(a, b, self.descending));
        candidates
            .into_iter()
            .map(|candidate| candidate.row)
            .collect()
    }

    pub fn graph(&self) -> Graph {
        match self.tab {
            Tab::Cpu => Graph::Stacked {
                lower: self.cpu.iter().map(|(system, _)| *system).collect(),
                upper: self
                    .cpu
                    .iter()
                    .map(|(system, user)| system + user)
                    .collect(),
            },
            Tab::Memory => Graph::Filled(self.memory.iter().copied().collect()),
            Tab::Disk => mirrored(&self.disk),
            Tab::Network => mirrored(&self.network),
        }
    }

    pub fn summary(&self) -> Vec<(String, String)> {
        let snapshot = &self.snapshot;
        let percent = |share: f32| format!("{} %", format_percent(f64::from(share) * 100.0));
        match self.tab {
            Tab::Cpu => vec![
                (L10N.tr("summary-system"), percent(snapshot.cpu.system)),
                (L10N.tr("summary-user"), percent(snapshot.cpu.user)),
                (L10N.tr("summary-idle"), percent(snapshot.cpu.idle)),
                (
                    L10N.tr("summary-processes"),
                    snapshot.processes.len().to_string(),
                ),
                (
                    L10N.tr("summary-threads"),
                    snapshot
                        .processes
                        .iter()
                        .map(|process| u64::from(process.threads))
                        .sum::<u64>()
                        .to_string(),
                ),
            ],
            Tab::Memory => {
                let memory = &snapshot.memory;
                vec![
                    (L10N.tr("summary-physical"), format_bytes(memory.total)),
                    (L10N.tr("summary-used"), format_bytes(memory.used)),
                    (L10N.tr("summary-cached"), format_bytes(memory.cached)),
                    (L10N.tr("summary-swap"), format_bytes(memory.swap_used)),
                    (
                        L10N.tr("summary-compressed"),
                        format_bytes(memory.compressed),
                    ),
                    (
                        L10N.tr("summary-pressure"),
                        format!("{} %", format_percent(f64::from(memory.pressure))),
                    ),
                ]
            }
            Tab::Disk => vec![
                (
                    L10N.tr("summary-read-rate"),
                    format_rate(snapshot.disk.read_per_second),
                ),
                (
                    L10N.tr("summary-written-rate"),
                    format_rate(snapshot.disk.written_per_second),
                ),
                (
                    L10N.tr("summary-read"),
                    format_bytes(snapshot.disk.read_total),
                ),
                (
                    L10N.tr("summary-written"),
                    format_bytes(snapshot.disk.written_total),
                ),
            ],
            Tab::Network => {
                let interfaces = &snapshot.interfaces;
                let total = |value: fn(&crate::sampler::Interface) -> f64| {
                    interfaces.iter().map(value).sum::<f64>()
                };
                vec![
                    (
                        L10N.tr("summary-received-rate"),
                        format_rate(total(|interface| interface.received_per_second)),
                    ),
                    (
                        L10N.tr("summary-sent-rate"),
                        format_rate(total(|interface| interface.sent_per_second)),
                    ),
                    (
                        L10N.tr("summary-received"),
                        format_bytes(total(|interface| interface.received_total as f64) as u64),
                    ),
                    (
                        L10N.tr("summary-sent"),
                        format_bytes(total(|interface| interface.sent_total as f64) as u64),
                    ),
                ]
            }
        }
    }
}

fn mirrored(history: &VecDeque<(f64, f64)>) -> Graph {
    let peak = history
        .iter()
        .fold(0.0, |peak: f64, (up, down)| peak.max(*up).max(*down));
    let scaled = |value: f64| {
        if peak > 0.0 {
            (value / peak) as f32
        } else {
            0.0
        }
    };
    Graph::Mirrored {
        up: history.iter().map(|(up, _)| scaled(*up)).collect(),
        down: history.iter().map(|(_, down)| scaled(*down)).collect(),
    }
}
