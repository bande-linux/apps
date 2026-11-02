use std::collections::{HashMap, VecDeque};

use bande_toolkit::ui::{Series, SeriesColor};
use serde::Deserialize;

pub const HISTORY: usize = 60;
const INACTIVE: [&str; 2] = ["inactive_file", "total_inactive_file"];
const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CpuUsage {
    total_usage: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Cpu {
    cpu_usage: CpuUsage,
    system_cpu_usage: u64,
    online_cpus: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Memory {
    usage: u64,
    limit: u64,
    stats: Option<HashMap<String, u64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Net {
    rx_bytes: u64,
    tx_bytes: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Io {
    op: String,
    value: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Blkio {
    io_service_bytes_recursive: Option<Vec<Io>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    read: String,
    cpu_stats: Cpu,
    precpu_stats: Cpu,
    memory_stats: Memory,
    networks: Option<HashMap<String, Net>>,
    blkio_stats: Blkio,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    pub cpu: f32,
    pub cpus: u32,
    pub memory: u64,
    pub limit: u64,
    received: u64,
    sent: u64,
    read: u64,
    written: u64,
    at: f64,
}

impl Raw {
    pub fn sample(&self) -> Sample {
        let now = &self.cpu_stats;
        let before = &self.precpu_stats;
        let busy = now
            .cpu_usage
            .total_usage
            .saturating_sub(before.cpu_usage.total_usage);
        let total = now.system_cpu_usage.saturating_sub(before.system_cpu_usage);
        let cpus = now.online_cpus.max(1);
        let cpu = match total {
            0 => 0.0,
            total => busy as f32 / total as f32 * cpus as f32 * 100.0,
        };
        let inactive = self
            .memory_stats
            .stats
            .iter()
            .flat_map(|stats| INACTIVE.iter().filter_map(|key| stats.get(*key)))
            .next()
            .copied()
            .unwrap_or_default();
        let networks = self.networks.iter().flat_map(HashMap::values);
        let io = self.blkio_stats.io_service_bytes_recursive.iter().flatten();
        let io_total = |op: &str| {
            io.clone()
                .filter(|entry| entry.op.eq_ignore_ascii_case(op))
                .map(|entry| entry.value)
                .sum()
        };
        Sample {
            cpu,
            cpus,
            memory: self.memory_stats.usage.saturating_sub(inactive),
            limit: self.memory_stats.limit,
            received: networks.clone().map(|net| net.rx_bytes).sum(),
            sent: networks.map(|net| net.tx_bytes).sum(),
            read: io_total("read"),
            written: io_total("write"),
            at: self
                .read
                .parse::<jiff::Timestamp>()
                .map(|at| at.as_millisecond() as f64 / 1000.0)
                .unwrap_or_default(),
        }
    }
}

pub fn bytes(count: f64) -> String {
    let mut value = count;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    match (unit, value < 10.0) {
        (0, _) => format!("{value:.0} {}", UNITS[unit]),
        (_, true) => format!("{value:.1} {}", UNITS[unit]),
        (_, false) => format!("{value:.0} {}", UNITS[unit]),
    }
}

pub fn percent(cpu: f32) -> String {
    match cpu < 10.0 {
        true => format!("{cpu:.1} %"),
        false => format!("{cpu:.0} %"),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    samples: VecDeque<Sample>,
}

pub struct Graphs {
    pub cpu: (Vec<Series>, String),
    pub memory: (Vec<Series>, String),
    pub network: (Vec<Series>, String),
    pub disk: (Vec<Series>, String),
}

fn rates(pairs: &[(f64, f64, f64)]) -> (Vec<f32>, Vec<f32>, f64, f64) {
    let peak = pairs
        .iter()
        .flat_map(|(_, first, second)| [*first, *second])
        .fold(1.0, f64::max);
    let scaled = |pick: fn(&(f64, f64, f64)) -> f64| {
        pairs
            .iter()
            .map(|pair| (pick(pair) / peak) as f32)
            .collect()
    };
    let (last_first, last_second) = pairs
        .last()
        .map_or((0.0, 0.0), |(_, first, second)| (*first, *second));
    (
        scaled(|pair| pair.1),
        scaled(|pair| pair.2),
        last_first,
        last_second,
    )
}

impl Usage {
    pub fn push(&mut self, sample: Sample) {
        if self.samples.len() == HISTORY + 1 {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn latest(&self) -> Option<&Sample> {
        self.samples.back()
    }

    fn deltas(&self, pick: fn(&Sample) -> (u64, u64)) -> Vec<(f64, f64, f64)> {
        self.samples
            .iter()
            .zip(self.samples.iter().skip(1))
            .map(|(before, after)| {
                let seconds = match after.at - before.at {
                    seconds if seconds > 0.0 => seconds,
                    _ => 1.0,
                };
                let (first_before, second_before) = pick(before);
                let (first_after, second_after) = pick(after);
                (
                    seconds,
                    first_after.saturating_sub(first_before) as f64 / seconds,
                    second_after.saturating_sub(second_before) as f64 / seconds,
                )
            })
            .collect()
    }

    pub fn graphs(&self) -> Graphs {
        let cpu = self
            .samples
            .iter()
            .map(|sample| sample.cpu / (sample.cpus.max(1) as f32 * 100.0));
        let memory = self
            .samples
            .iter()
            .map(|sample| sample.memory as f32 / sample.limit.max(1) as f32);
        let latest = self.latest().copied().unwrap_or_default();
        let (received, sent, down, up) =
            rates(&self.deltas(|sample| (sample.received, sample.sent)));
        let (read, written, reading, writing) =
            rates(&self.deltas(|sample| (sample.read, sample.written)));
        Graphs {
            cpu: (
                vec![Series::new(SeriesColor::Primary, cpu)],
                percent(latest.cpu),
            ),
            memory: (
                vec![Series::new(SeriesColor::Success, memory)],
                bytes(latest.memory as f64),
            ),
            network: (
                vec![
                    Series::new(SeriesColor::Primary, received),
                    Series::new(SeriesColor::Secondary, sent),
                ],
                format!("↓ {}/s  ↑ {}/s", bytes(down), bytes(up)),
            ),
            disk: (
                vec![
                    Series::new(SeriesColor::Primary, read),
                    Series::new(SeriesColor::Secondary, written),
                ],
                format!("R {}/s  W {}/s", bytes(reading), bytes(writing)),
            ),
        }
    }
}
