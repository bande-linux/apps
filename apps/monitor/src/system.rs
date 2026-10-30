use std::fs;
use std::path::Path;

use crate::proc::parse_kilobytes;

const SECTOR: u64 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub app: u64,
    pub kernel: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub compressed: u64,
    pub pressure: Pressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pressure {
    pub some: f32,
    pub full: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Normal,
    Elevated,
    Critical,
}

impl Pressure {
    const ELEVATED_SOME: f32 = 10.0;
    const CRITICAL_FULL: f32 = 5.0;

    pub fn level(self) -> PressureLevel {
        if self.full >= Self::CRITICAL_FULL {
            PressureLevel::Critical
        } else if self.some >= Self::ELEVATED_SOME {
            PressureLevel::Elevated
        } else {
            PressureLevel::Normal
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Transferred {
    pub(crate) received: u64,
    pub(crate) sent: u64,
}

pub(crate) fn read_memory(root: &Path) -> Option<Memory> {
    let meminfo = fs::read_to_string(root.join("proc/meminfo")).ok()?;
    let value = |key: &str| parse_kilobytes(&meminfo, key).unwrap_or(0);
    let total = value("MemTotal");
    let swap_total = value("SwapTotal");
    Some(Memory {
        total,
        used: total.saturating_sub(value("MemAvailable")),
        app: value("AnonPages") + value("Shmem"),
        kernel: value("SUnreclaim") + value("KernelStack") + value("PageTables"),
        cached: value("Buffers") + value("Cached") + value("SReclaimable"),
        swap_total,
        swap_used: swap_total.saturating_sub(value("SwapFree")),
        compressed: read_compressed(root),
        pressure: read_pressure(root).unwrap_or_default(),
    })
}

fn read_compressed(root: &Path) -> u64 {
    let Ok(devices) = fs::read_dir(root.join("sys/block")) else {
        return 0;
    };
    devices
        .filter_map(Result::ok)
        .filter(|device| device.file_name().to_string_lossy().starts_with("zram"))
        .filter_map(|device| fs::read_to_string(device.path().join("mm_stat")).ok())
        .filter_map(|stat| stat.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

fn read_pressure(root: &Path) -> Option<Pressure> {
    let pressure = fs::read_to_string(root.join("proc/pressure/memory")).ok()?;
    let average = |kind: &str| {
        pressure
            .lines()
            .find_map(|line| line.strip_prefix(kind)?.strip_prefix(' '))?
            .split_whitespace()
            .find_map(|pair| pair.strip_prefix("avg10="))?
            .parse()
            .ok()
    };
    Some(Pressure {
        some: average("some")?,
        full: average("full").unwrap_or(0.0),
    })
}

pub(crate) fn read_disks(root: &Path) -> Transferred {
    let Ok(stats) = fs::read_to_string(root.join("proc/diskstats")) else {
        return Transferred::default();
    };
    stats
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let name = fields.get(2)?;
            if !root.join("sys/block").join(name).join("device").exists() {
                return None;
            }
            let sectors = |index: usize| fields.get(index)?.parse::<u64>().ok();
            Some(Transferred {
                received: sectors(5)? * SECTOR,
                sent: sectors(9)? * SECTOR,
            })
        })
        .fold(Transferred::default(), |sum, disk| Transferred {
            received: sum.received + disk.received,
            sent: sum.sent + disk.sent,
        })
}

pub(crate) fn read_interfaces(root: &Path) -> Vec<(String, Transferred)> {
    let Ok(dev) = fs::read_to_string(root.join("proc/net/dev")) else {
        return Vec::new();
    };
    dev.lines()
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" {
                return None;
            }
            let counters = counters.split_whitespace().collect::<Vec<_>>();
            let counter = |index: usize| counters.get(index)?.parse::<u64>().ok();
            Some((
                name.to_owned(),
                Transferred {
                    received: counter(0)?,
                    sent: counter(8)?,
                },
            ))
        })
        .collect()
}
