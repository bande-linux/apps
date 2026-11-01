use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use crate::L10N;
use crate::apps::{Catalog, Group, GroupKey};
use bande_status::history::{charging_spans, time_on_battery};
use bande_status::status::Power;

use crate::format::{format_bytes, format_cpu_time, format_hours, format_percent, format_rate};
use crate::power::{BATTERY_HISTORY, BatteryReport};
use crate::sampler::{Process, Snapshot};
use crate::signal::{SignalError, Stop, stop};
use crate::system::PressureLevel;

pub const HISTORY: usize = 60;
const ENERGY_SAMPLES: usize = 15;
const WAKEUPS_PER_IMPACT: f64 = 10.0;
const MISSING: &str = "\u{2013}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Cpu,
    Memory,
    Energy,
    Disk,
    Network,
    Gpu,
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
    Gpu,
    GpuTime,
    GpuMemory,
    EnergyImpact,
    Wakeups,
    PreventingSleep,
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
    Lines {
        primary: Vec<f32>,
        secondary: Option<Vec<f32>>,
        scale: String,
    },
    Pressure(Vec<(f32, PressureLevel)>),
    Battery {
        charge: Vec<(f32, f32)>,
        charging: Vec<(f32, f32)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopTarget {
    pub name: String,
    pub count: usize,
    pub roots: Vec<(u32, u64)>,
    pub apps: Vec<(String, Vec<(u32, u64)>)>,
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
        for app in target.apps {
            if !all.apps.contains(&app) {
                all.apps.push(app);
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
    pub const ALL: [Tab; 6] = [
        Tab::Cpu,
        Tab::Memory,
        Tab::Energy,
        Tab::Disk,
        Tab::Network,
        Tab::Gpu,
    ];

    pub fn title(self) -> String {
        match self {
            Tab::Cpu => L10N.tr("tab-cpu"),
            Tab::Memory => L10N.tr("tab-memory"),
            Tab::Energy => L10N.tr("tab-energy"),
            Tab::Disk => L10N.tr("tab-disk"),
            Tab::Network => L10N.tr("tab-network"),
            Tab::Gpu => L10N.tr("tab-gpu"),
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
            Tab::Energy => &[
                Column::Name,
                Column::EnergyImpact,
                Column::Wakeups,
                Column::Cpu,
                Column::Gpu,
                Column::PreventingSleep,
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
            Tab::Gpu => &[
                Column::Name,
                Column::Gpu,
                Column::GpuTime,
                Column::GpuMemory,
                Column::Pid,
                Column::User,
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
            Tab::Energy => "energy",
            Tab::Disk => "disk",
            Tab::Network => "network",
            Tab::Gpu => "gpu",
        }
    }

    pub fn has_column_choice(self) -> bool {
        self != Tab::Network
    }

    fn default_sort(self) -> Column {
        match self {
            Tab::Cpu => Column::Cpu,
            Tab::Memory => Column::Memory,
            Tab::Energy => Column::EnergyImpact,
            Tab::Disk => Column::Written,
            Tab::Network => Column::Received,
            Tab::Gpu => Column::Gpu,
        }
    }
}

impl Column {
    pub const CHOOSABLE: [Column; 17] = [
        Column::EnergyImpact,
        Column::Cpu,
        Column::CpuTime,
        Column::Wakeups,
        Column::PreventingSleep,
        Column::Threads,
        Column::Pid,
        Column::User,
        Column::Memory,
        Column::Resident,
        Column::Read,
        Column::Written,
        Column::ReadTotal,
        Column::WrittenTotal,
        Column::Gpu,
        Column::GpuTime,
        Column::GpuMemory,
    ];

    fn uses_gpu(self) -> bool {
        matches!(
            self,
            Column::Gpu | Column::GpuTime | Column::GpuMemory | Column::EnergyImpact
        )
    }

    fn uses_wakeups(self) -> bool {
        matches!(self, Column::Wakeups | Column::EnergyImpact)
    }

    fn uses_disk(self) -> bool {
        matches!(
            self,
            Column::Read | Column::Written | Column::ReadTotal | Column::WrittenTotal
        )
    }

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
            Column::Gpu => "gpu",
            Column::GpuTime => "gpu-time",
            Column::GpuMemory => "gpu-memory",
            Column::EnergyImpact => "energy-impact",
            Column::Wakeups => "wakeups",
            Column::PreventingSleep => "preventing-sleep",
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
            Column::Gpu => L10N.tr("column-gpu"),
            Column::GpuTime => L10N.tr("column-gpu-time"),
            Column::GpuMemory => L10N.tr("column-gpu-memory"),
            Column::EnergyImpact => L10N.tr("column-energy-impact"),
            Column::Wakeups => L10N.tr("column-wakeups"),
            Column::PreventingSleep => L10N.tr("column-preventing-sleep"),
        }
    }

    pub fn numeric(self) -> bool {
        !matches!(self, Column::Name | Column::User | Column::PreventingSleep)
    }

    fn text(self, measure: &Measure) -> String {
        let Measure::Number(value) = measure else {
            return match measure {
                Measure::Text(text) => text.clone(),
                _ => MISSING.to_owned(),
            };
        };
        match self {
            Column::Cpu | Column::Gpu | Column::EnergyImpact => format_percent(*value),
            Column::CpuTime | Column::GpuTime => format_cpu_time(*value),
            Column::GpuMemory
            | Column::Memory
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
        let gpu = |value: fn(&crate::sampler::GpuUse) -> f64| {
            let known = processes
                .iter()
                .filter_map(|process| process.gpu.as_ref())
                .collect::<Vec<_>>();
            if known.is_empty() {
                Measure::Missing
            } else {
                Measure::Number(known.into_iter().map(value).sum())
            }
        };
        match self {
            Column::Name => Measure::Text(name.to_owned()),
            Column::Gpu => gpu(|used| f64::from(used.percent)),
            Column::GpuTime => gpu(|used| used.seconds),
            Column::GpuMemory => gpu(|used| used.memory as f64),
            Column::EnergyImpact => sum(|process| process.energy.unwrap_or(0.0)),
            Column::PreventingSleep => Measure::Text(
                match processes
                    .iter()
                    .any(|process| process.preventing_sleep.is_some())
                {
                    true => L10N.tr("preventing-sleep-yes"),
                    false => L10N.tr("preventing-sleep-no"),
                },
            ),
            Column::Wakeups => {
                let known = processes
                    .iter()
                    .filter_map(|process| process.wakeups)
                    .collect::<Vec<_>>();
                match known.is_empty() {
                    true => Measure::Missing,
                    false => Measure::Number(known.into_iter().sum()),
                }
            }
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
    let roots = members
        .iter()
        .filter(|process| !members.iter().any(|parent| parent.pid == process.ppid))
        .map(|process| id(process))
        .collect::<Vec<_>>();
    let (roots, apps) = match &group.key {
        GroupKey::App(app_id) => (Vec::new(), vec![(app_id.clone(), roots)]),
        _ => (roots, Vec::new()),
    };
    StopTarget {
        name,
        count: 1,
        roots,
        apps,
        processes: members.into_iter().map(id).collect(),
    }
}

pub fn stop_target(target: &StopTarget, how: Stop) -> Result<(), SignalError> {
    match how {
        Stop::Quit => stop_all(&target.roots, how),
        Stop::ForceQuit => stop_all(&target.processes, how),
    }
}

pub fn stop_all(chosen: &[(u32, u64)], how: Stop) -> Result<(), SignalError> {
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
    chosen: [Vec<Column>; Tab::ALL.len()],
    cpu: VecDeque<(f32, f32)>,
    memory: VecDeque<(f32, PressureLevel)>,
    disk: VecDeque<(f64, f64)>,
    network: VecDeque<(f64, f64)>,
    gpu: VecDeque<f32>,
    cores: VecDeque<Vec<f32>>,
    cores_shown: bool,
    uid: u32,
    filter: Filter,
    impacts: HashMap<(u32, u64), VecDeque<f64>>,
    energy: VecDeque<(f64, f64)>,
    power: Option<BatteryReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKind {
    Performance,
    Efficiency,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreGroup {
    pub kind: Option<CoreKind>,
    pub cores: Vec<(usize, Vec<f32>)>,
}

const CORES_STATE: &str = "cores";
const FILTER_STATE: &str = "filter";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Mine,
    Apps,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Mine, Filter::Apps];

    pub fn id(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Mine => "mine",
            Filter::Apps => "apps",
        }
    }

    pub fn title(self) -> String {
        match self {
            Filter::All => L10N.tr("filter-all"),
            Filter::Mine => L10N.tr("filter-mine"),
            Filter::Apps => L10N.tr("filter-apps"),
        }
    }
}

fn find_row<'a>(rows: &'a [Row], key: &RowKey) -> Option<&'a Row> {
    rows.iter().find_map(|row| match &row.key == key {
        true => Some(row),
        false => find_row(&row.children, key),
    })
}

impl Monitor {
    pub fn new(catalog: Catalog, uid: u32) -> Self {
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
            gpu: VecDeque::new(),
            cores: VecDeque::new(),
            cores_shown: false,
            uid,
            filter: Filter::All,
            impacts: HashMap::new(),
            energy: VecDeque::new(),
            power: None,
        }
    }

    fn rate_energy(&mut self, snapshot: &mut Snapshot) {
        if snapshot.seconds <= 0.0 {
            return;
        }
        let mut earlier = std::mem::take(&mut self.impacts);
        let mut total = 0.0;
        for process in &mut snapshot.processes {
            let now = f64::from(process.cpu_percent)
                + process.gpu.map_or(0.0, |used| f64::from(used.percent))
                + process.wakeups.unwrap_or(0.0) / WAKEUPS_PER_IMPACT;
            let id = (process.pid, process.start_ticks);
            let mut history = earlier.remove(&id).unwrap_or_default();
            if history.len() == ENERGY_SAMPLES {
                history.pop_front();
            }
            history.push_back(now);
            let average = history.iter().sum::<f64>() / history.len() as f64;
            process.energy = Some(average);
            total += average;
            self.impacts.insert(id, history);
        }
        let wakeups = snapshot
            .processes
            .iter()
            .filter_map(|process| process.wakeups)
            .fold(0.0, |total, wakeups| total + wakeups);
        push_limited(&mut self.energy, (total, wakeups));
    }

    pub fn update(&mut self, mut snapshot: Snapshot) {
        self.rate_energy(&mut snapshot);
        self.groups = self.catalog.group(&snapshot.processes);
        if snapshot.seconds > 0.0 {
            push_limited(&mut self.cpu, (snapshot.cpu.system, snapshot.cpu.user));
            let memory = &snapshot.memory;
            push_limited(
                &mut self.memory,
                (share(memory.used, memory.total), memory.pressure.level()),
            );
            push_limited(
                &mut self.gpu,
                (gpu_percent(&snapshot) / 100.0).clamp(0.0, 1.0) as f32,
            );
            push_limited(
                &mut self.disk,
                (
                    snapshot.disk.read_per_second,
                    snapshot.disk.written_per_second,
                ),
            );
            push_limited(&mut self.network, network_rates(&snapshot));
            push_limited(
                &mut self.cores,
                snapshot
                    .cores
                    .iter()
                    .map(|core| core.user + core.system)
                    .collect(),
            );
        }
        match (snapshot.battery, &snapshot.power) {
            (false, _) => self.power = None,
            (true, Some(report)) => self.power = Some(report.clone()),
            (true, None) => {}
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

    pub fn state(&self) -> String {
        let mut state = Tab::ALL
            .iter()
            .filter(|tab| tab.has_column_choice())
            .map(|tab| {
                let ids = self.chosen[tab.index()]
                    .iter()
                    .map(|column| column.id())
                    .collect::<Vec<_>>();
                format!("{} {}\n", tab.id(), ids.join(" "))
            })
            .collect::<String>();
        if self.cores_shown {
            state.push_str(CORES_STATE);
            state.push('\n');
        }
        if self.filter != Filter::All {
            state.push_str(&format!("{FILTER_STATE} {}\n", self.filter.id()));
        }
        state
    }

    pub fn restore(&mut self, text: &str) {
        self.cores_shown = text.lines().any(|line| line.trim() == CORES_STATE);
        self.filter = text
            .lines()
            .find_map(|line| line.strip_prefix(FILTER_STATE)?.strip_prefix(' '))
            .and_then(|id| {
                Filter::ALL
                    .into_iter()
                    .find(|filter| filter.id() == id.trim())
            })
            .unwrap_or(Filter::All);
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
        self.columns().contains(&Column::Memory)
    }

    pub fn measures_gpu(&self) -> bool {
        self.tab == Tab::Gpu || self.columns().iter().any(|column| column.uses_gpu())
    }

    pub fn measures_wakeups(&self) -> bool {
        self.columns().iter().any(|column| column.uses_wakeups())
    }

    pub fn measures_sleep_blockers(&self) -> bool {
        self.columns().contains(&Column::PreventingSleep)
    }

    pub fn measures_battery(&self) -> bool {
        self.tab == Tab::Energy
    }

    pub fn measures_disk(&self) -> bool {
        self.columns().iter().any(|column| column.uses_disk())
    }

    pub fn tabs(&self) -> Vec<Tab> {
        Tab::ALL
            .into_iter()
            .filter(|tab| match tab {
                Tab::Gpu => self.snapshot.gpu,
                Tab::Energy => self.snapshot.battery,
                _ => true,
            })
            .collect()
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
                    apps: Vec::new(),
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
                    .filter(|process| self.passes_filter(group, process))
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
            .map(|interface| (interface, interface.title()))
            .filter(|(_, title)| title.to_lowercase().contains(query))
            .map(|(interface, title)| {
                let measure = |column: Column| match column {
                    Column::Name => Measure::Text(title.clone()),
                    Column::Received => Measure::Number(interface.received_per_second),
                    Column::Sent => Measure::Number(interface.sent_per_second),
                    Column::ReceivedTotal => Measure::Number(interface.received_total as f64),
                    Column::SentTotal => Measure::Number(interface.sent_total as f64),
                    _ => Measure::Missing,
                };
                Candidate {
                    row: Row {
                        key: RowKey::Interface(interface.name.clone()),
                        name: title.clone(),
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
                    name: title.clone(),
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
        let full = || "100 %".to_owned();
        match self.tab {
            Tab::Cpu => Graph::Lines {
                primary: self
                    .cpu
                    .iter()
                    .map(|(system, user)| system + user)
                    .collect(),
                secondary: Some(self.cpu.iter().map(|(system, _)| *system).collect()),
                scale: full(),
            },
            Tab::Memory => Graph::Pressure(self.memory.iter().copied().collect()),
            Tab::Energy => match self.battery_history() {
                Some(report) => battery_graph(report),
                None => {
                    let peak = self
                        .energy
                        .iter()
                        .fold(0.0, |peak: f64, (impact, _)| peak.max(*impact));
                    Graph::Lines {
                        primary: self
                            .energy
                            .iter()
                            .map(|(impact, _)| match peak > 0.0 {
                                true => (impact / peak) as f32,
                                false => 0.0,
                            })
                            .collect(),
                        secondary: None,
                        scale: format_percent(peak),
                    }
                }
            },
            Tab::Disk => rates_graph(&self.disk),
            Tab::Network => rates_graph(&self.network),
            Tab::Gpu => Graph::Lines {
                primary: self.gpu.iter().copied().collect(),
                secondary: None,
                scale: full(),
            },
        }
    }

    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rebuild();
    }

    fn passes_filter(&self, group: &Group, process: &Process) -> bool {
        match self.filter {
            Filter::All => true,
            Filter::Mine => process.identity.uid == self.uid,
            Filter::Apps => group.app.is_some(),
        }
    }

    pub fn cores_shown(&self) -> bool {
        self.cores_shown && self.tab == Tab::Cpu
    }

    pub fn set_cores_shown(&mut self, shown: bool) {
        self.cores_shown = shown;
    }

    pub fn core_groups(&self) -> Vec<CoreGroup> {
        let count = self.cores.back().map_or(0, Vec::len);
        let capacities = &self.snapshot.capacities;
        let fastest = capacities.iter().copied().max().unwrap_or(0);
        let mixed = capacities.iter().any(|capacity| *capacity != fastest);
        let history = |core: usize| {
            self.cores
                .iter()
                .map(|shares| shares.get(core).copied().unwrap_or(0.0))
                .collect::<Vec<_>>()
        };
        let group = |kind: Option<CoreKind>| CoreGroup {
            kind,
            cores: (0..count)
                .filter(|core| {
                    let capacity = capacities.get(*core).copied().unwrap_or(fastest);
                    match kind {
                        None => true,
                        Some(CoreKind::Performance) => capacity == fastest,
                        Some(CoreKind::Efficiency) => capacity != fastest,
                    }
                })
                .map(|core| (core, history(core)))
                .collect(),
        };
        match mixed {
            true => vec![
                group(Some(CoreKind::Performance)),
                group(Some(CoreKind::Efficiency)),
            ],
            false => vec![group(None)],
        }
    }

    pub fn memory_bar(&self) -> Option<Vec<(usize, f32)>> {
        let memory = &self.snapshot.memory;
        (self.tab == Tab::Memory).then(|| {
            [
                (2, memory.app),
                (3, memory.kernel),
                (4, memory.compressed),
                (5, memory.cached),
            ]
            .into_iter()
            .map(|(readout, bytes)| (readout, share(bytes, memory.total)))
            .collect()
        })
    }

    pub fn graph_description(&self) -> String {
        let snapshot = &self.snapshot;
        let percent = |share: f32| format_percent(f64::from(share) * 100.0).into();
        let (received, sent) = network_rates(snapshot);
        match self.tab {
            Tab::Cpu => L10N.tr_args(
                "graph-cpu",
                [
                    ("total", percent(snapshot.cpu.user + snapshot.cpu.system)),
                    ("user", percent(snapshot.cpu.user)),
                    ("system", percent(snapshot.cpu.system)),
                ],
            ),
            Tab::Memory => {
                let memory = &snapshot.memory;
                let level = match memory.pressure.level() {
                    PressureLevel::Normal => L10N.tr("pressure-normal"),
                    PressureLevel::Elevated => L10N.tr("pressure-elevated"),
                    PressureLevel::Critical => L10N.tr("pressure-critical"),
                };
                L10N.tr_args(
                    "graph-memory",
                    [
                        ("level", level.into()),
                        ("used", percent(share(memory.used, memory.total))),
                    ],
                )
            }
            Tab::Energy => {
                let impact = format_percent(self.energy.back().map_or(0.0, |(impact, _)| *impact));
                match self.power.as_ref().and_then(|report| report.battery) {
                    Some(battery) => L10N.tr_args(
                        "graph-battery-charge",
                        [
                            ("charge", format!("{:.0}", battery.percent).into()),
                            ("impact", impact.into()),
                        ],
                    ),
                    None => L10N.tr_args("graph-energy", [("impact", impact.into())]),
                }
            }
            Tab::Disk => L10N.tr_args(
                "graph-disk",
                [
                    ("read", format_rate(snapshot.disk.read_per_second).into()),
                    (
                        "written",
                        format_rate(snapshot.disk.written_per_second).into(),
                    ),
                ],
            ),
            Tab::Network => L10N.tr_args(
                "graph-network",
                [
                    ("received", format_rate(received).into()),
                    ("sent", format_rate(sent).into()),
                ],
            ),
            Tab::Gpu => L10N.tr_args(
                "graph-gpu",
                [("used", format_percent(gpu_percent(snapshot)).into())],
            ),
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
                    (L10N.tr("summary-app"), format_bytes(memory.app)),
                    (L10N.tr("summary-kernel"), format_bytes(memory.kernel)),
                    (
                        L10N.tr("summary-compressed"),
                        format_bytes(memory.compressed),
                    ),
                    (L10N.tr("summary-cached"), format_bytes(memory.cached)),
                    (L10N.tr("summary-swap"), format_bytes(memory.swap_used)),
                ]
            }
            Tab::Energy => {
                let (impact, wakeups) = self.energy.back().copied().unwrap_or((0.0, 0.0));
                let mut readouts = vec![
                    (L10N.tr("summary-energy-impact"), format_percent(impact)),
                    (L10N.tr("summary-wakeups"), format!("{wakeups:.0}")),
                ];
                readouts.extend(self.battery_readouts());
                readouts
            }
            Tab::Disk => vec![
                (
                    L10N.tr("summary-reads"),
                    format!("{:.0}", snapshot.disk_operations.reads_per_second),
                ),
                (
                    L10N.tr("summary-writes"),
                    format!("{:.0}", snapshot.disk_operations.writes_per_second),
                ),
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
            Tab::Gpu => {
                let users = snapshot
                    .processes
                    .iter()
                    .filter_map(|process| process.gpu)
                    .filter(|used| used.percent > 0.0 || used.memory > 0)
                    .collect::<Vec<_>>();
                vec![
                    (
                        L10N.tr("summary-gpu"),
                        format!("{} %", format_percent(gpu_percent(snapshot))),
                    ),
                    (
                        L10N.tr("summary-gpu-memory"),
                        format_bytes(users.iter().map(|used| used.memory).sum()),
                    ),
                    (L10N.tr("summary-gpu-processes"), users.len().to_string()),
                ]
            }
        }
    }
}

fn share(part: u64, total: u64) -> f32 {
    match total {
        0 => 0.0,
        total => part as f32 / total as f32,
    }
}

fn gpu_percent(snapshot: &Snapshot) -> f64 {
    snapshot
        .processes
        .iter()
        .filter_map(|process| process.gpu)
        .map(|used| f64::from(used.percent))
        .sum()
}

fn network_rates(snapshot: &Snapshot) -> (f64, f64) {
    snapshot
        .interfaces
        .iter()
        .fold((0.0, 0.0), |(received, sent), interface| {
            (
                received + interface.received_per_second,
                sent + interface.sent_per_second,
            )
        })
}

impl Monitor {
    fn battery_history(&self) -> Option<&BatteryReport> {
        self.power
            .as_ref()
            .filter(|report| !report.history.is_empty())
    }

    fn battery_readouts(&self) -> Vec<(String, String)> {
        let Some(report) = &self.power else {
            return Vec::new();
        };
        let Some(battery) = report.battery else {
            return Vec::new();
        };
        let hours = |duration: Option<std::time::Duration>| {
            duration.map_or(MISSING.to_owned(), |duration| {
                format_hours(duration.as_secs())
            })
        };
        let time = match battery.power {
            Power::Battery { remaining } => (L10N.tr("summary-time-remaining"), hours(remaining)),
            Power::Charging { until_full } => {
                (L10N.tr("summary-time-until-full"), hours(until_full))
            }
            Power::Full => (L10N.tr("summary-time-until-full"), MISSING.to_owned()),
        };
        let on_battery = match battery.power {
            Power::Battery { .. } => time_on_battery(&report.history, report.now)
                .map_or(MISSING.to_owned(), |seconds| {
                    format_hours(u64::from(seconds))
                }),
            _ => MISSING.to_owned(),
        };
        vec![
            (
                L10N.tr("summary-remaining-charge"),
                format!("{:.0} %", battery.percent),
            ),
            time,
            (L10N.tr("summary-time-on-battery"), on_battery),
        ]
    }
}

fn battery_graph(report: &BatteryReport) -> Graph {
    let start = report.now.saturating_sub(BATTERY_HISTORY);
    let along =
        |at: u32| (at.saturating_sub(start) as f32 / BATTERY_HISTORY as f32).clamp(0.0, 1.0);
    Graph::Battery {
        charge: report
            .history
            .iter()
            .map(|sample| {
                (
                    along(sample.at),
                    (sample.percent / 100.0).clamp(0.0, 1.0) as f32,
                )
            })
            .collect(),
        charging: charging_spans(&report.history, report.now)
            .into_iter()
            .map(|(from, to)| (along(from), along(to)))
            .filter(|(from, to)| to > from)
            .collect(),
    }
}

fn rates_graph(history: &VecDeque<(f64, f64)>) -> Graph {
    let peak = history.iter().fold(0.0, |peak: f64, (primary, secondary)| {
        peak.max(*primary).max(*secondary)
    });
    let scaled = |value: f64| match peak > 0.0 {
        true => (value / peak) as f32,
        false => 0.0,
    };
    Graph::Lines {
        primary: history
            .iter()
            .map(|(primary, _)| scaled(*primary))
            .collect(),
        secondary: Some(
            history
                .iter()
                .map(|(_, secondary)| scaled(*secondary))
                .collect(),
        ),
        scale: format_rate(peak),
    }
}
