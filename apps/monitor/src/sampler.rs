use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::cpu::{CpuShare, CpuTicks, read_cpu_ticks};
use crate::proc::{
    DrmUse, IoBytes, Stat, display_name, parse_cgroup_unit, parse_cmdline, parse_drm_fdinfo,
    parse_io, parse_kilobytes, parse_passwd, parse_stat, parse_uid,
};
use crate::system::{Memory, Transferred, read_disks, read_interfaces, read_memory};

const KERNEL_THREAD_DAEMON: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub command: Vec<String>,
    pub executable: Option<PathBuf>,
    pub uid: u32,
    pub user: String,
    pub unit: Option<String>,
    pub kernel_thread: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub start_ticks: u64,
    pub identity: Rc<Identity>,
    pub cpu_percent: f32,
    pub cpu_seconds: f64,
    pub threads: u32,
    pub proportional: Option<u64>,
    pub resident: u64,
    pub disk: Option<Rates>,
    pub gpu: Option<GpuUse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuUse {
    pub percent: f32,
    pub seconds: f64,
    pub memory: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rates {
    pub read_per_second: f64,
    pub written_per_second: f64,
    pub read_total: u64,
    pub written_total: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub name: String,
    pub received_per_second: f64,
    pub sent_per_second: f64,
    pub received_total: u64,
    pub sent_total: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub seconds: f64,
    pub cpu: CpuShare,
    pub cores: Vec<CpuShare>,
    pub processes: Vec<Process>,
    pub memory: Memory,
    pub disk: Rates,
    pub interfaces: Vec<Interface>,
    pub gpu: bool,
}

struct Known {
    start_ticks: u64,
    identity: Rc<Identity>,
    cpu_ticks: u64,
    io: Option<IoBytes>,
    reads_io: bool,
    reads_rollup: bool,
    gpu_ns: Option<u64>,
}

struct Previous {
    cpu: CpuTicks,
    cores: Vec<CpuTicks>,
    disk: Transferred,
    interfaces: HashMap<String, Transferred>,
}

pub struct Sampler {
    root: PathBuf,
    ticks_per_second: f64,
    page_size: u64,
    users: HashMap<u32, String>,
    known: HashMap<u32, Known>,
    previous: Option<Previous>,
    proportional: bool,
    disk: bool,
    gpu: bool,
    gpu_seen: bool,
    buffer: Vec<u8>,
}

const DRM_DEVICES: &str = "/dev/dri/";

fn read_gpu(folder: &Path) -> Option<DrmUse> {
    let mut clients = HashMap::new();
    for entry in fs::read_dir(folder.join("fd")).ok()?.filter_map(Result::ok) {
        let drm = fs::read_link(entry.path()).is_ok_and(|target| target.starts_with(DRM_DEVICES));
        if !drm {
            continue;
        }
        let info = fs::read_to_string(folder.join("fdinfo").join(entry.file_name()));
        if let Some((client, used)) = info.ok().as_deref().and_then(parse_drm_fdinfo) {
            clients.insert(client, used);
        }
    }
    (!clients.is_empty()).then(|| DrmUse {
        engine_ns: clients.values().map(|used| used.engine_ns).sum(),
        memory: clients.values().map(|used| used.memory).sum(),
    })
}

fn rate(now: u64, then: u64, seconds: f64) -> f64 {
    if seconds > 0.0 {
        now.saturating_sub(then) as f64 / seconds
    } else {
        0.0
    }
}

impl Sampler {
    pub fn new(root: &Path, ticks_per_second: u64, page_size: u64) -> Self {
        let users = fs::read_to_string(root.join("etc/passwd"))
            .map(|passwd| parse_passwd(&passwd))
            .unwrap_or_default();
        Self {
            root: root.to_path_buf(),
            ticks_per_second: ticks_per_second as f64,
            page_size,
            users,
            known: HashMap::new(),
            previous: None,
            proportional: false,
            disk: false,
            gpu: false,
            gpu_seen: false,
            buffer: Vec::new(),
        }
    }

    pub fn live() -> Self {
        Self::new(
            Path::new("/"),
            rustix::param::clock_ticks_per_second(),
            rustix::param::page_size() as u64,
        )
    }

    pub fn measure_proportional_memory(&mut self, measure: bool) {
        self.proportional = measure;
    }

    pub fn measure_disk(&mut self, measure: bool) {
        self.disk = measure;
    }

    pub fn measure_gpu(&mut self, measure: bool) {
        self.gpu = measure;
    }

    fn read(&mut self, path: &Path) -> io::Result<&str> {
        self.buffer.clear();
        File::open(path)?.read_to_end(&mut self.buffer)?;
        std::str::from_utf8(&self.buffer).map_err(io::Error::other)
    }

    fn identify(&mut self, folder: &Path, pid: u32, stat: &Stat) -> Identity {
        let command = fs::read(folder.join("cmdline"))
            .map(|bytes| parse_cmdline(&bytes))
            .unwrap_or_default();
        let uid = self
            .read(&folder.join("status"))
            .ok()
            .and_then(parse_uid)
            .unwrap_or(0);
        let unit = self
            .read(&folder.join("cgroup"))
            .ok()
            .and_then(parse_cgroup_unit);
        Identity {
            name: display_name(&stat.comm, &command),
            executable: fs::read_link(folder.join("exe")).ok(),
            user: self
                .users
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| uid.to_string()),
            uid,
            unit,
            kernel_thread: pid == KERNEL_THREAD_DAEMON || stat.ppid == KERNEL_THREAD_DAEMON,
            command,
        }
    }

    fn process(&mut self, pid: u32, seconds: f64, first: bool) -> Option<Process> {
        let folder = self.root.join("proc").join(pid.to_string());
        let stat = parse_stat(self.read(&folder.join("stat")).ok()?)?;
        if self
            .known
            .get(&pid)
            .is_none_or(|known| known.start_ticks != stat.start_ticks)
        {
            let identity = Rc::new(self.identify(&folder, pid, &stat));
            let readable = !identity.kernel_thread;
            self.known.insert(
                pid,
                Known {
                    start_ticks: stat.start_ticks,
                    identity,
                    cpu_ticks: if first { stat.cpu_ticks } else { 0 },
                    io: (!first).then_some(IoBytes::default()),
                    reads_io: readable,
                    reads_rollup: readable,
                    gpu_ns: None,
                },
            );
        }
        let (reads_io, reads_rollup, kernel_thread) = self.known.get(&pid).map(|known| {
            (
                known.reads_io,
                known.reads_rollup,
                known.identity.kernel_thread,
            )
        })?;
        let drm = ((self.gpu || first) && !kernel_thread)
            .then(|| read_gpu(&folder))
            .flatten();
        self.gpu_seen |= drm.is_some();
        let io = (self.disk && reads_io)
            .then(|| self.read(&folder.join("io")).ok().and_then(parse_io))
            .flatten();
        let proportional = (self.proportional && reads_rollup)
            .then(|| {
                self.read(&folder.join("smaps_rollup"))
                    .ok()
                    .and_then(|rollup| parse_kilobytes(rollup, "Pss"))
            })
            .flatten();
        let ticks_per_second = self.ticks_per_second;
        let resident = stat.resident_pages * self.page_size;
        let known = self.known.get_mut(&pid)?;
        let disk = io.map(|now| {
            let then = known.io.unwrap_or(now);
            Rates {
                read_per_second: rate(now.read, then.read, seconds),
                written_per_second: rate(now.written, then.written, seconds),
                read_total: now.read,
                written_total: now.written,
            }
        });
        let gpu = drm.map(|now| {
            let then = known.gpu_ns.unwrap_or(now.engine_ns);
            GpuUse {
                percent: (rate(now.engine_ns, then, seconds) / 1e7) as f32,
                seconds: now.engine_ns as f64 / 1e9,
                memory: now.memory,
            }
        });
        known.gpu_ns = drm.map(|now| now.engine_ns);
        let cpu_percent = rate(stat.cpu_ticks, known.cpu_ticks, seconds) / ticks_per_second * 100.0;
        known.cpu_ticks = stat.cpu_ticks;
        known.io = io;
        known.reads_io &= !self.disk || io.is_some();
        known.reads_rollup &= !self.proportional || proportional.is_some();
        Some(Process {
            pid,
            ppid: stat.ppid,
            start_ticks: stat.start_ticks,
            identity: known.identity.clone(),
            cpu_percent: cpu_percent as f32,
            cpu_seconds: stat.cpu_ticks as f64 / ticks_per_second,
            threads: stat.threads,
            proportional,
            resident,
            disk,
            gpu,
        })
    }

    pub fn sample(&mut self) -> io::Result<Snapshot> {
        let (cpu, cores) = read_cpu_ticks(&self.root)?;
        let disk = read_disks(&self.root);
        let interfaces = read_interfaces(&self.root);
        let previous = self.previous.take();
        let first = previous.is_none();
        let seconds = previous.as_ref().map_or(0.0, |previous| {
            cpu.total.saturating_sub(previous.cpu.total) as f64
                / cores.len().max(1) as f64
                / self.ticks_per_second
        });
        let mut pids = fs::read_dir(self.root.join("proc"))?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .collect::<Vec<_>>();
        pids.sort_unstable();
        let processes = pids
            .iter()
            .filter_map(|pid| self.process(*pid, seconds, first))
            .collect::<Vec<_>>();
        self.known.retain(|pid, _| pids.binary_search(pid).is_ok());
        let previous = previous.unwrap_or_else(|| Previous {
            cpu,
            cores: cores.clone(),
            disk,
            interfaces: interfaces.iter().cloned().collect(),
        });
        let snapshot = Snapshot {
            seconds,
            cpu: cpu.share_since(previous.cpu),
            cores: cores
                .iter()
                .zip(&previous.cores)
                .map(|(now, then)| now.share_since(*then))
                .collect(),
            processes,
            memory: read_memory(&self.root).unwrap_or_default(),
            disk: Rates {
                read_per_second: rate(disk.received, previous.disk.received, seconds),
                written_per_second: rate(disk.sent, previous.disk.sent, seconds),
                read_total: disk.received,
                written_total: disk.sent,
            },
            interfaces: interfaces
                .iter()
                .map(|(name, now)| {
                    let then = previous.interfaces.get(name).copied().unwrap_or(*now);
                    Interface {
                        name: name.clone(),
                        received_per_second: rate(now.received, then.received, seconds),
                        sent_per_second: rate(now.sent, then.sent, seconds),
                        received_total: now.received,
                        sent_total: now.sent,
                    }
                })
                .collect(),
            gpu: self.gpu_seen,
        };
        self.previous = Some(Previous {
            cpu,
            cores,
            disk,
            interfaces: interfaces.into_iter().collect(),
        });
        Ok(snapshot)
    }
}
