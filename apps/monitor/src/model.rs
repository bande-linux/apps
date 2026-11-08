use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;


use bande_l10n::{Text, text, tr};

use crate::apps::{Catalog, Group, GroupKey};
use bande_status::history::{ChargeSample, charging_spans, time_on_battery};
use bande_status::status::Power;
use bande_toolkit::ui::{Band, GraphKind, Series, SeriesColor};

use crate::format::{format_bytes, format_cpu_time, format_hours, format_percent, format_rate};
use crate::power::{BATTERY_HISTORY, BatteryReport};
use crate::sampler::{Process, Snapshot};
use crate::signal::{SignalError, Stop, stop};
use crate::system::PressureLevel;

pub const HISTORY: usize = 60;
const BATTERY_SAMPLES: usize = 145;
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
    Compressed,
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
pub struct Graph {
    pub title: String,
    pub span: String,
    pub kind: GraphKind,
    pub samples: usize,
    pub series: Vec<Series>,
    pub bands: Vec<Band>,
    pub scale: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    pub label: String,
    pub value: String,
    pub series: Option<SeriesColor>,
}

impl Readout {
    fn new(message: Text, value: String) -> Self {
        Self {
            label: message.tr(),
            value,
            series: None,
        }
    }

    fn keyed(message: Text, value: String, series: SeriesColor) -> Self {
        Self {
            series: Some(series),
            ..Self::new(message, value)
        }
    }
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
            Tab::Cpu => tr!("CPU"),
            Tab::Memory => tr!("Memory"),
            Tab::Energy => tr!("Energy"),
            Tab::Disk => tr!("Disk"),
            Tab::Network => tr!("Network"),
            Tab::Gpu => tr!("GPU"),
        }
    }

    pub fn columns(self) -> &'static [Column] {
        match self {
            Tab::Cpu => &[
                Column::Name,
                Column::Cpu,
                Column::CpuTime,
                Column::Threads,
                Column::Wakeups,
                Column::Pid,
                Column::User,
            ],
            Tab::Memory => &[
                Column::Name,
                Column::Memory,
                Column::Compressed,
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
                Column::Pid,
                Column::User,
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
    pub const CHOOSABLE: [Column; 18] = [
        Column::EnergyImpact,
        Column::Cpu,
        Column::CpuTime,
        Column::Wakeups,
        Column::PreventingSleep,
        Column::Threads,
        Column::Pid,
        Column::User,
        Column::Memory,
        Column::Compressed,
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
            Column::Compressed => "compressed",
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

    pub fn title(self) -> String {
        match self {
            Column::Name => tr!("Name"),
            Column::Cpu => tr!("% CPU"),
            Column::CpuTime => tr!("CPU Time"),
            Column::Threads => tr!("Threads"),
            Column::Pid => tr!("PID"),
            Column::User => tr!("User"),
            Column::Memory => tr!("Memory"),
            Column::Compressed => tr!("Compressed"),
            Column::Resident => tr!("Resident"),
            Column::Read => tr!("Read/s"),
            Column::Written => tr!("Written/s"),
            Column::ReadTotal => tr!("Bytes Read"),
            Column::WrittenTotal => tr!("Bytes Written"),
            Column::Received => tr!("Received/s"),
            Column::Sent => tr!("Sent/s"),
            Column::ReceivedTotal => tr!("Bytes Received"),
            Column::SentTotal => tr!("Bytes Sent"),
            Column::Gpu => tr!("% GPU"),
            Column::GpuTime => tr!("GPU Time"),
            Column::GpuMemory => tr!("GPU Memory"),
            Column::EnergyImpact => tr!("Energy Impact"),
            Column::Wakeups => tr!("Wake-ups/s"),
            Column::PreventingSleep => tr!("Preventing Sleep"),
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
            | Column::Compressed
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
        let traffic = |value: fn(&crate::sampler::Traffic) -> f64| {
            let known = processes
                .iter()
                .filter_map(|process| process.network.as_ref())
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
                    true => tr!("Yes"),
                    false => tr!("No"),
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
            Column::Memory => sum(|process| process.footprint.unwrap_or(process.resident) as f64),
            Column::Compressed => {
                let known = processes
                    .iter()
                    .filter_map(|process| process.swapped)
                    .collect::<Vec<_>>();
                match known.is_empty() {
                    true => Measure::Missing,
                    false => Measure::Number(known.into_iter().sum::<u64>() as f64),
                }
            }
            Column::Resident => sum(|process| process.resident as f64),
            Column::Read => disk(|rates| rates.read_per_second),
            Column::Written => disk(|rates| rates.written_per_second),
            Column::ReadTotal => disk(|rates| rates.read_total as f64),
            Column::WrittenTotal => disk(|rates| rates.written_total as f64),
            Column::Received => traffic(|traffic| traffic.received_per_second),
            Column::Sent => traffic(|traffic| traffic.sent_per_second),
            Column::ReceivedTotal => traffic(|traffic| traffic.received_total as f64),
            Column::SentTotal => traffic(|traffic| traffic.sent_total as f64),
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
        _ => stop_all(&target.processes, how),
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
    sorts: [Option<(Column, bool)>; Tab::ALL.len()],
    cpu: VecDeque<(f32, f32)>,
    memory: VecDeque<(f32, PressureLevel)>,
    disk: VecDeque<(f64, f64)>,
    network: VecDeque<(f64, f64)>,
    gpu: VecDeque<f32>,
    cores: VecDeque<Vec<f32>>,
    cores_shown: bool,
    uid: u32,
    filter: Filter,
    pinned: Vec<(u32, u64)>,
    frequency: Frequency,
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
const FREQUENCY_STATE: &str = "frequency";
const SORT_STATE: &str = "sort";
const ASCENDING: &str = "ascending";
const DESCENDING: &str = "descending";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Frequency {
    VeryOften,
    #[default]
    Often,
    Normally,
}

impl Frequency {
    pub const ALL: [Frequency; 3] = [Frequency::VeryOften, Frequency::Often, Frequency::Normally];

    pub fn interval(self) -> Duration {
        Duration::from_secs(match self {
            Frequency::VeryOften => 1,
            Frequency::Often => 2,
            Frequency::Normally => 5,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Frequency::VeryOften => "very-often",
            Frequency::Often => "often",
            Frequency::Normally => "normally",
        }
    }

    pub fn title(self) -> String {
        let seconds = self.interval().as_secs();
        match self {
            Frequency::VeryOften => tr!("Very Often ({seconds} sec)", seconds = seconds),
            Frequency::Often => tr!("Often ({seconds} sec)", seconds = seconds),
            Frequency::Normally => tr!("Normally ({seconds} sec)", seconds = seconds),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Hierarchy,
    Mine,
    System,
    Active,
    Apps,
    Selected,
}

impl Filter {
    pub const ALL: [Filter; 7] = [
        Filter::All,
        Filter::Hierarchy,
        Filter::Mine,
        Filter::System,
        Filter::Active,
        Filter::Apps,
        Filter::Selected,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Hierarchy => "hierarchy",
            Filter::Mine => "mine",
            Filter::System => "system",
            Filter::Active => "active",
            Filter::Apps => "apps",
            Filter::Selected => "selected",
        }
    }

    pub fn title(self) -> String {
        match self {
            Filter::All => tr!("All Processes"),
            Filter::Hierarchy => tr!("All Processes, Hierarchically"),
            Filter::Mine => tr!("My Processes"),
            Filter::System => tr!("System Processes"),
            Filter::Active => tr!("Active Processes"),
            Filter::Apps => tr!("Apps"),
            Filter::Selected => tr!("Selected Processes"),
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
            sorts: [None; Tab::ALL.len()],
            cpu: VecDeque::new(),
            memory: VecDeque::new(),
            disk: VecDeque::new(),
            network: VecDeque::new(),
            gpu: VecDeque::new(),
            cores: VecDeque::new(),
            cores_shown: false,
            uid,
            filter: Filter::All,
            pinned: Vec::new(),
            frequency: Frequency::default(),
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
            (self.sort, self.descending) = self.tab_sort(tab);
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
        if !matches!(self.filter, Filter::All | Filter::Selected) {
            state.push_str(&format!("{FILTER_STATE} {}\n", self.filter.id()));
        }
        for (tab, sort) in Tab::ALL.iter().zip(&self.sorts) {
            if let Some((column, descending)) = sort {
                let order = if *descending { DESCENDING } else { ASCENDING };
                state.push_str(&format!(
                    "{SORT_STATE} {} {} {order}\n",
                    tab.id(),
                    column.id()
                ));
            }
        }
        if self.frequency != Frequency::default() {
            state.push_str(&format!("{FREQUENCY_STATE} {}\n", self.frequency.id()));
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
        self.frequency = text
            .lines()
            .find_map(|line| line.strip_prefix(FREQUENCY_STATE)?.strip_prefix(' '))
            .and_then(|id| {
                Frequency::ALL
                    .into_iter()
                    .find(|frequency| frequency.id() == id.trim())
            })
            .unwrap_or_default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            if words.clone().next() == Some(SORT_STATE) {
                let mut words = words.skip(1);
                let tab = words
                    .next()
                    .and_then(|id| Tab::ALL.into_iter().find(|tab| tab.id() == id));
                let column = words.next().and_then(Column::parse);
                let descending = words.next() == Some(DESCENDING);
                if let (Some(tab), Some(column)) = (tab, column) {
                    self.sorts[tab.index()] = Some((column, descending));
                }
                continue;
            }
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
        (self.sort, self.descending) = self.tab_sort(self.tab);
        self.rebuild();
    }

    fn tab_sort(&self, tab: Tab) -> (Column, bool) {
        let chosen = &self.chosen[tab.index()];
        [self.sorts[tab.index()], Some((tab.default_sort(), true))]
            .into_iter()
            .flatten()
            .find(|(column, _)| chosen.contains(column))
            .unwrap_or((Column::Name, false))
    }

    pub fn sort(&self) -> (Column, bool) {
        (self.sort, self.descending)
    }

    pub fn set_sort(&mut self, column: Column, descending: bool) {
        self.sort = column;
        self.descending = descending;
        self.sorts[self.tab.index()] = Some((column, descending));
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

    pub fn measures_footprint(&self) -> bool {
        self.columns()
            .iter()
            .any(|column| matches!(column, Column::Memory | Column::Compressed))
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

    pub fn measures_network(&self) -> bool {
        self.tab == Tab::Network
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
            GroupKey::Kernel => tr!("Kernel"),
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
        self.rows = if self.filter == Filter::Hierarchy {
            self.tree_rows(columns, &query)
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
                    children = shown
                        .iter()
                        .map(|process| self.process_candidate(process, columns, Vec::new()))
                        .collect();
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

    fn process_candidate(
        &self,
        process: &Process,
        columns: &[Column],
        children: Vec<Row>,
    ) -> Candidate {
        let name = &process.identity.name;
        Candidate {
            row: Row {
                key: RowKey::Process {
                    pid: process.pid,
                    start_ticks: process.start_ticks,
                },
                name: name.clone(),
                icon: None,
                cells: columns
                    .iter()
                    .map(|column| column.text(&column.measure(&[process], name, process)))
                    .collect(),
                responding: self.unresponsive.process_responds(process.pid),
                revealed: !children.is_empty() && !process.identity.kernel_thread,
                children,
            },
            measure: self.sort.measure(&[process], name, process),
            name: name.clone(),
            pid: process.pid,
        }
    }

    fn tree_rows(&self, columns: &[Column], query: &str) -> Vec<Row> {
        let processes = &self.snapshot.processes;
        let known = processes
            .iter()
            .map(|process| process.pid)
            .collect::<HashSet<_>>();
        let mut children: HashMap<u32, Vec<&Process>> = HashMap::new();
        let mut roots = Vec::new();
        for process in processes {
            let has_parent = process.ppid != process.pid && known.contains(&process.ppid);
            if query.is_empty() && has_parent {
                children.entry(process.ppid).or_default().push(process);
            } else if query.is_empty() || self.matches(process, query) {
                roots.push(process);
            }
        }
        self.level_rows(&roots, &children, columns)
    }

    fn level_rows(
        &self,
        level: &[&Process],
        children: &HashMap<u32, Vec<&Process>>,
        columns: &[Column],
    ) -> Vec<Row> {
        let mut candidates = level
            .iter()
            .map(|process| {
                let below = children
                    .get(&process.pid)
                    .map(|below| self.level_rows(below, children, columns))
                    .unwrap_or_default();
                self.process_candidate(process, columns, below)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| order(a, b, self.descending));
        candidates
            .into_iter()
            .map(|candidate| candidate.row)
            .collect()
    }

    pub fn graph(&self) -> Graph {
        let titled = |message: Text| Graph {
            title: message.tr(),
            span: self.span(),
            kind: GraphKind::Area,
            samples: HISTORY,
            series: Vec::new(),
            bands: Vec::new(),
            scale: None,
        };
        match self.tab {
            Tab::Cpu => Graph {
                kind: GraphKind::Stacked,
                series: vec![
                    Series::new(
                        SeriesColor::Secondary,
                        self.cpu.iter().map(|(system, _)| *system),
                    ),
                    Series::new(SeriesColor::Primary, self.cpu.iter().map(|(_, user)| *user)),
                ],
                ..titled(text!("CPU Load"))
            },
            Tab::Memory => Graph {
                series: pressure_series(&self.memory),
                ..titled(text!("Memory Pressure"))
            },
            Tab::Energy => match self.battery_history() {
                Some(report) => Graph {
                    span: tr!("12 hr"),
                    samples: BATTERY_SAMPLES,
                    ..battery_graph(report, titled(text!("Battery (last 12 hours)")))
                },
                None => {
                    let peak = self
                        .energy
                        .iter()
                        .fold(0.0, |peak: f64, (impact, _)| peak.max(*impact));
                    Graph {
                        series: vec![Series::new(
                            SeriesColor::Primary,
                            self.energy.iter().map(|(impact, _)| match peak > 0.0 {
                                true => (impact / peak) as f32,
                                false => 0.0,
                            }),
                        )],
                        scale: Some(format_percent(peak)),
                        ..titled(text!("Energy Impact"))
                    }
                }
            },
            Tab::Disk => rates_graph(&self.disk, titled(text!("Data Read and Written"))),
            Tab::Network => rates_graph(&self.network, titled(text!("Data Received and Sent"))),
            Tab::Gpu => Graph {
                series: vec![Series::new(SeriesColor::Primary, self.gpu.iter().copied())],
                scale: Some("100 %".to_owned()),
                ..titled(text!("GPU Usage"))
            },
        }
    }

    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn frequency(&self) -> Frequency {
        self.frequency
    }

    pub fn set_frequency(&mut self, frequency: Frequency) {
        self.frequency = frequency;
    }

    pub fn span(&self) -> String {
        let minutes = HISTORY as u64 * self.frequency.interval().as_secs() / 60;
        tr!("{minutes} min", minutes = minutes)
    }

    pub fn set_filter(&mut self, filter: Filter) {
        if filter == Filter::Selected {
            self.pinned = self
                .selection
                .iter()
                .flat_map(|key| self.members(key))
                .collect();
        }
        self.filter = filter;
        self.rebuild();
    }

    fn members(&self, key: &RowKey) -> Vec<(u32, u64)> {
        match key {
            RowKey::Group(key) => self
                .group(key)
                .map(|group| {
                    group
                        .members
                        .iter()
                        .map(|member| &self.snapshot.processes[*member])
                        .map(|process| (process.pid, process.start_ticks))
                        .collect()
                })
                .unwrap_or_default(),
            RowKey::Process { pid, start_ticks } => vec![(*pid, *start_ticks)],
        }
    }

    fn passes_filter(&self, group: &Group, process: &Process) -> bool {
        match self.filter {
            Filter::All | Filter::Hierarchy => true,
            Filter::Mine => process.identity.uid == self.uid,
            Filter::System => process.identity.uid == 0,
            Filter::Active => process.cpu_percent > 0.0,
            Filter::Apps => group.app.is_some(),
            Filter::Selected => self.pinned.contains(&(process.pid, process.start_ticks)),
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

    pub fn graph_description(&self) -> String {
        let snapshot = &self.snapshot;
        let percent = |share: f32| format_percent(f64::from(share) * 100.0);
        let (received, sent) = network_rates(snapshot);
        match self.tab {
            Tab::Cpu => tr!(
                "CPU {total} %, user {user} %, system {system} %",
                total = percent(snapshot.cpu.user + snapshot.cpu.system),
                user = percent(snapshot.cpu.user),
                system = percent(snapshot.cpu.system)
            ),
            Tab::Memory => {
                let memory = &snapshot.memory;
                let level = match memory.pressure.level() {
                    PressureLevel::Normal => tr!("normal"),
                    PressureLevel::Elevated => tr!("elevated"),
                    PressureLevel::Critical => tr!("critical"),
                };
                tr!(
                    "Memory pressure {level}, {used} % used",
                    level = level,
                    used = percent(share(memory.used, memory.total))
                )
            }
            Tab::Energy => {
                let impact = format_percent(self.energy.back().map_or(0.0, |(impact, _)| *impact));
                match self.power.as_ref().and_then(|report| report.battery) {
                    Some(battery) => tr!(
                        "Battery {charge} %, energy impact {impact}",
                        charge = format!("{:.0}", battery.percent),
                        impact = impact
                    ),
                    None => tr!("Energy impact {impact}", impact = impact),
                }
            }
            Tab::Disk => tr!(
                "Disk read {read}, written {written}",
                read = format_rate(snapshot.disk.read_per_second),
                written = format_rate(snapshot.disk.written_per_second),
            ),
            Tab::Network => tr!(
                "Network received {received}, sent {sent}",
                received = format_rate(received),
                sent = format_rate(sent)
            ),
            Tab::Gpu => tr!("GPU {used} %", used = format_percent(gpu_percent(snapshot))),
        }
    }

    pub fn summary(&self) -> Vec<Vec<Readout>> {
        let snapshot = &self.snapshot;
        let percent = |share: f32| format!("{} %", format_percent(f64::from(share) * 100.0));
        match self.tab {
            Tab::Cpu => vec![
                vec![
                    Readout::keyed(
                        text!("System"),
                        percent(snapshot.cpu.system),
                        SeriesColor::Secondary,
                    ),
                    Readout::keyed(
                        text!("User"),
                        percent(snapshot.cpu.user),
                        SeriesColor::Primary,
                    ),
                    Readout::new(text!("Idle"), percent(snapshot.cpu.idle)),
                ],
                vec![
                    Readout::new(
                        text!("Threads"),
                        snapshot
                            .processes
                            .iter()
                            .map(|process| u64::from(process.threads))
                            .sum::<u64>()
                            .to_string(),
                    ),
                    Readout::new(text!("Processes"), snapshot.processes.len().to_string()),
                ],
            ],
            Tab::Memory => {
                let memory = &snapshot.memory;
                vec![
                    vec![
                        Readout::new(text!("Physical Memory"), format_bytes(memory.total)),
                        Readout::new(text!("Memory Used"), format_bytes(memory.used)),
                        Readout::new(text!("Cached Files"), format_bytes(memory.cached)),
                        Readout::new(text!("Swap Used"), format_bytes(memory.swap_used)),
                    ],
                    vec![
                        Readout::new(text!("App Memory"), format_bytes(memory.app)),
                        Readout::new(text!("Wired Memory"), format_bytes(memory.kernel)),
                        Readout::new(text!("Compressed"), format_bytes(memory.compressed)),
                    ],
                ]
            }
            Tab::Energy => {
                let (impact, wakeups) = self.energy.back().copied().unwrap_or((0.0, 0.0));
                let mut columns = vec![vec![
                    Readout::keyed(
                        text!("Energy Impact"),
                        format_percent(impact),
                        SeriesColor::Primary,
                    ),
                    Readout::new(text!("Wake-ups/s"), format!("{wakeups:.0}")),
                ]];
                let battery = self.battery_readouts();
                if !battery.is_empty() {
                    columns.push(battery);
                }
                columns
            }
            Tab::Disk => vec![
                vec![
                    Readout::keyed(
                        text!("Data Read/s"),
                        format_rate(snapshot.disk.read_per_second),
                        SeriesColor::Primary,
                    ),
                    Readout::keyed(
                        text!("Data Written/s"),
                        format_rate(snapshot.disk.written_per_second),
                        SeriesColor::Secondary,
                    ),
                    Readout::new(
                        text!("Reads/s"),
                        format!("{:.0}", snapshot.disk_operations.reads_per_second),
                    ),
                    Readout::new(
                        text!("Writes/s"),
                        format!("{:.0}", snapshot.disk_operations.writes_per_second),
                    ),
                ],
                vec![
                    Readout::new(text!("Data Read"), format_bytes(snapshot.disk.read_total)),
                    Readout::new(
                        text!("Data Written"),
                        format_bytes(snapshot.disk.written_total),
                    ),
                ],
            ],
            Tab::Network => {
                let interfaces = &snapshot.interfaces;
                let total = |value: fn(&crate::sampler::Interface) -> f64| {
                    interfaces.iter().map(value).sum::<f64>()
                };
                vec![
                    vec![
                        Readout::keyed(
                            text!("Data Received/s"),
                            format_rate(total(|interface| interface.received_per_second)),
                            SeriesColor::Primary,
                        ),
                        Readout::keyed(
                            text!("Data Sent/s"),
                            format_rate(total(|interface| interface.sent_per_second)),
                            SeriesColor::Secondary,
                        ),
                    ],
                    vec![
                        Readout::new(
                            text!("Data Received"),
                            format_bytes(total(|interface| interface.received_total as f64) as u64),
                        ),
                        Readout::new(
                            text!("Data Sent"),
                            format_bytes(total(|interface| interface.sent_total as f64) as u64),
                        ),
                    ],
                ]
            }
            Tab::Gpu => {
                let users = snapshot
                    .processes
                    .iter()
                    .filter_map(|process| process.gpu)
                    .filter(|used| used.percent > 0.0 || used.memory > 0)
                    .collect::<Vec<_>>();
                vec![vec![
                    Readout::keyed(
                        text!("GPU Usage"),
                        format!("{} %", format_percent(gpu_percent(snapshot))),
                        SeriesColor::Primary,
                    ),
                    Readout::new(
                        text!("GPU Memory"),
                        format_bytes(users.iter().map(|used| used.memory).sum()),
                    ),
                    Readout::new(text!("Processes Using the GPU"), users.len().to_string()),
                ]]
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

    fn battery_readouts(&self) -> Vec<Readout> {
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
            Power::Battery { remaining } => Readout::new(text!("Time Remaining"), hours(remaining)),
            Power::Charging { until_full } => {
                Readout::new(text!("Time Until Full"), hours(until_full))
            }
            Power::Full => Readout::new(text!("Time Until Full"), MISSING.to_owned()),
        };
        let on_battery = match battery.power {
            Power::Battery { .. } => time_on_battery(&report.history, report.now)
                .map_or(MISSING.to_owned(), |seconds| {
                    format_hours(u64::from(seconds))
                }),
            _ => MISSING.to_owned(),
        };
        vec![
            Readout::new(
                text!("Remaining Charge"),
                format!("{:.0} %", battery.percent),
            ),
            time,
            Readout::new(text!("Time on Battery"), on_battery),
        ]
    }
}

fn pressure_series(history: &VecDeque<(f32, PressureLevel)>) -> Vec<Series> {
    let levels = history.iter().map(|(_, level)| *level).collect::<Vec<_>>();
    let mut series = Vec::new();
    let mut start = 0;
    while let Some(level) = levels.get(start).copied() {
        let end = levels[start..]
            .iter()
            .position(|other| *other != level)
            .map_or(levels.len(), |offset| start + offset);
        let color = match level {
            PressureLevel::Normal => SeriesColor::Success,
            PressureLevel::Elevated => SeriesColor::Warning,
            PressureLevel::Critical => SeriesColor::Danger,
        };
        let values = history
            .iter()
            .enumerate()
            .map(|(index, (used, _))| (start..=end).contains(&index).then_some(*used))
            .collect();
        series.push(Series { color, values });
        start = end;
    }
    series
}

fn charge_at(history: &[ChargeSample], at: f64) -> Option<f32> {
    let after = history
        .iter()
        .position(|sample| f64::from(sample.at) >= at)?;
    let next = &history[after];
    let percent = match after.checked_sub(1) {
        Some(before) => {
            let previous = &history[before];
            let (from, to) = (f64::from(previous.at), f64::from(next.at));
            previous.percent + (next.percent - previous.percent) * (at - from) / (to - from)
        }
        None if f64::from(next.at) <= at => next.percent,
        None => return None,
    };
    Some((percent / 100.0).clamp(0.0, 1.0) as f32)
}

fn battery_graph(report: &BatteryReport, graph: Graph) -> Graph {
    let start = report.now.saturating_sub(BATTERY_HISTORY);
    let along =
        |at: u32| (at.saturating_sub(start) as f32 / BATTERY_HISTORY as f32).clamp(0.0, 1.0);
    let slot = f64::from(BATTERY_HISTORY) / (BATTERY_SAMPLES - 1) as f64;
    Graph {
        series: vec![Series {
            color: SeriesColor::Primary,
            values: (0..BATTERY_SAMPLES)
                .map(|index| charge_at(&report.history, f64::from(start) + index as f64 * slot))
                .collect(),
        }],
        bands: charging_spans(&report.history, report.now)
            .into_iter()
            .map(|(from, to)| along(from)..along(to))
            .filter(|along| along.end > along.start)
            .map(|along| Band {
                color: SeriesColor::Success,
                along,
            })
            .collect(),
        ..graph
    }
}

fn rates_graph(history: &VecDeque<(f64, f64)>, graph: Graph) -> Graph {
    let peak = history.iter().fold(0.0, |peak: f64, (primary, secondary)| {
        peak.max(*primary).max(*secondary)
    });
    let scaled = |value: f64| match peak > 0.0 {
        true => (value / peak) as f32,
        false => 0.0,
    };
    Graph {
        series: vec![
            Series::new(
                SeriesColor::Primary,
                history.iter().map(|(primary, _)| scaled(*primary)),
            ),
            Series::new(
                SeriesColor::Secondary,
                history.iter().map(|(_, secondary)| scaled(*secondary)),
            ),
        ],
        scale: Some(format_rate(peak)),
        ..graph
    }
}
