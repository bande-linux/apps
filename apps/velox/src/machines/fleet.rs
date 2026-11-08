use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use bande_l10n::{text, tr};
use bande_vm::{Accel, ActiveState, Network, UsbDevice};

const SSH_PORT: u16 = 22;

use super::backend::{Act, Listed, Phase, Update};
use super::readiness::Blocker;
use crate::containers::UsageHistory;
use crate::engine::Sample;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Probing,
    Blocked(Blocker),
    Ready(Accel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Stopped,
    Starting,
    Running,
    Paused,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    pub cpu: f32,
    pub memory: u64,
}

pub struct Entry {
    pub listed: Listed,
    pub unit: ActiveState,
    pub paused: bool,
    pub pending: Option<Act>,
    pub usage: Option<Usage>,
    pub history: UsageHistory,
    pub network: (u64, u64),
    pub address: Option<std::net::Ipv4Addr>,
    cpu_sample: Option<(Instant, u64)>,
}

pub struct Job {
    pub name: String,
    pub os: String,
    pub phase: Phase,
}

pub struct Notice {
    pub id: Option<String>,
    pub title: String,
    pub message: String,
}

pub struct Fleet {
    pub status: Status,
    pub entries: BTreeMap<String, Entry>,
    pub jobs: BTreeMap<u64, Job>,
    pub created: Vec<(u64, String)>,
    pub notices: Vec<Notice>,
    pub focus: Option<String>,
    pub to_open: Vec<String>,
    pub chosen_iso: Option<PathBuf>,
    pub usb_devices: Vec<UsbDevice>,
    pub usb: BTreeMap<String, Vec<String>>,
}

impl Entry {
    pub fn id(&self) -> &str {
        &self.listed.manifest.id
    }

    pub fn name(&self) -> &str {
        &self.listed.manifest.name
    }

    pub fn power(&self) -> Power {
        match (self.unit, self.paused) {
            (ActiveState::Activating, _) => Power::Starting,
            (ActiveState::Active, true) => Power::Paused,
            (ActiveState::Active, false) => Power::Running,
            (ActiveState::Deactivating, _) => Power::Stopping,
            (ActiveState::Inactive, _) => Power::Stopped,
            (ActiveState::Failed, _) => Power::Failed,
        }
    }

    pub fn running(&self) -> bool {
        self.unit.is_running()
    }

    pub fn ssh_target(&self) -> Option<(String, u16)> {
        match &self.listed.manifest.network {
            Network::Shared => self
                .listed
                .manifest
                .ssh_port()
                .map(|port| ("localhost".to_owned(), port)),
            Network::Bridged(_) => self.address.map(|address| (address.to_string(), SSH_PORT)),
        }
    }

    pub fn can(&self, act: Act) -> bool {
        let pending = self.pending;
        match act {
            Act::Start => pending.is_none() && !self.running(),
            Act::ShutDown | Act::Restart => pending.is_none() && self.power() == Power::Running,
            Act::ForceStop => {
                self.running() && matches!(pending, None | Some(Act::ShutDown | Act::Restart))
            }
            Act::Delete | Act::Clone => pending.is_none() && !self.running(),
            Act::Pause => pending.is_none() && self.power() == Power::Running,
            Act::Suspend => {
                pending.is_none()
                    && self.power() == Power::Running
                    && self.listed.manifest.shares.is_empty()
            }
            Act::Resume => pending.is_none() && self.power() == Power::Paused,
            Act::TakeSnapshot => {
                pending.is_none()
                    && matches!(self.power(), Power::Running | Power::Stopped)
                    && self.listed.manifest.shares.is_empty()
            }
            Act::Revert | Act::DeleteSnapshot => {
                pending.is_none() && matches!(self.power(), Power::Running | Power::Stopped)
            }
        }
    }

    pub fn ssh_command(&self) -> Option<Vec<String>> {
        let (host, port) = self.ssh_target()?;
        let login = &self.listed.login;
        let mut command = vec![
            "ssh".to_owned(),
            "-p".to_owned(),
            port.to_string(),
            "-o".to_owned(),
            format!(
                "UserKnownHostsFile={}",
                self.listed.dir.join("known_hosts").display()
            ),
        ];
        if let Some(identity) = &login.identity {
            command.extend(["-i".to_owned(), identity.display().to_string()]);
        }
        let user = match login.user.is_empty() {
            true => String::new(),
            false => format!("{}@", login.user),
        };
        command.push(format!("{user}{host}"));
        Some(command)
    }

    fn sample(&mut self, now: Instant) {
        let usage = cgroup(&bande_vm::unit_name(self.id())).and_then(|folder| {
            let stat = fs::read_to_string(folder.join("cpu.stat")).ok()?;
            let used = stat
                .lines()
                .find_map(|line| line.strip_prefix("usage_usec "))?
                .trim()
                .parse::<u64>()
                .ok()?;
            let memory = fs::read_to_string(folder.join("memory.current"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            let cpu = match self.cpu_sample.replace((now, used)) {
                Some((then, before)) if now > then => {
                    used.saturating_sub(before) as f32
                        / now.duration_since(then).as_micros().max(1) as f32
                        * 100.0
                }
                _ => return self.usage.map(|usage| Usage { memory, ..usage }),
            };
            Some(Usage { cpu, memory })
        });
        let disk = cgroup(&bande_vm::unit_name(self.id()))
            .and_then(|folder| fs::read_to_string(folder.join("io.stat")).ok())
            .map(|stat| io_totals(&stat))
            .unwrap_or_default();
        if let Some(usage) = usage {
            self.history.push(Sample {
                at: now,
                cpu: f64::from(usage.cpu),
                memory: usage.memory,
                read: disk.0,
                written: disk.1,
                received: self.network.0,
                sent: self.network.1,
            });
        }
        self.usage = usage;
    }
}

fn io_totals(stat: &str) -> (u64, u64) {
    let field = |line: &str, key: &str| -> u64 {
        line.split_whitespace()
            .find_map(|pair| pair.strip_prefix(key))
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    };
    stat.lines().fold((0, 0), |(read, written), line| {
        (
            read + field(line, "rbytes="),
            written + field(line, "wbytes="),
        )
    })
}

fn cgroup(unit: &str) -> Option<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let manager = PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service"
    ));
    [manager.join("app.slice").join(unit), manager.join(unit)]
        .into_iter()
        .find(|folder| folder.is_dir())
}

fn settled(entry: &Entry) -> bool {
    match entry.pending {
        Some(Act::Start) => entry.unit == ActiveState::Active,
        _ => false,
    }
}

impl Default for Fleet {
    fn default() -> Self {
        Self {
            status: Status::Probing,
            entries: BTreeMap::new(),
            jobs: BTreeMap::new(),
            created: Vec::new(),
            notices: Vec::new(),
            focus: None,
            to_open: Vec::new(),
            chosen_iso: None,
            usb_devices: Vec::new(),
            usb: BTreeMap::new(),
        }
    }
}

impl Fleet {
    pub fn ready(&self) -> bool {
        matches!(self.status, Status::Ready(_))
    }

    pub fn running(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.running())
            .count()
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    pub fn add_job(&mut self, job: u64, name: String, os: String) {
        self.jobs.insert(
            job,
            Job {
                name,
                os,
                phase: Phase::Downloading(None),
            },
        );
    }

    pub fn set_job_phase(&mut self, job: u64) {
        if let Some(entry) = self.jobs.get_mut(&job) {
            entry.phase = Phase::Creating;
        }
    }

    pub fn mark(&mut self, id: &str, act: Act) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.pending = Some(act);
        }
    }

    pub fn sample(&mut self, now: Instant) {
        for entry in self.entries.values_mut() {
            match entry.running() {
                true => entry.sample(now),
                false => {
                    entry.usage = None;
                    entry.cpu_sample = None;
                }
            }
        }
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Ready(accel) => self.status = Status::Ready(accel),
            Update::Blocked(blocker) => {
                self.status = Status::Blocked(blocker);
                self.entries.clear();
            }
            Update::Listed(listed) => {
                let mut previous = std::mem::take(&mut self.entries);
                for listed in listed {
                    let id = listed.manifest.id.clone();
                    let entry = match previous.remove(&id) {
                        Some(mut entry) => {
                            if entry.unit != listed.unit && !listed.unit.is_running() {
                                entry.paused = false;
                            }
                            entry.unit = listed.unit;
                            entry.listed = listed;
                            entry
                        }
                        None => Entry {
                            unit: listed.unit,
                            listed,
                            paused: false,
                            pending: None,
                            usage: None,
                            history: UsageHistory::default(),
                            network: (0, 0),
                            address: None,
                            cpu_sample: None,
                        },
                    };
                    self.entries.insert(id, entry);
                }
                for (job, id) in &self.created {
                    self.jobs.remove(job);
                    if let Some(entry) = self.entries.get_mut(id) {
                        entry.pending.get_or_insert(Act::Start);
                    }
                }
                let entries = &self.entries;
                self.created.retain(|(_, id)| !entries.contains_key(id));
            }
            Update::UsbDevices(devices) => self.usb_devices = devices,
            Update::Usb(id, attached) => {
                self.usb.insert(id, attached);
            }
            Update::UsbFailed(id, message) => {
                let name = self
                    .entries
                    .get(&id)
                    .map_or_else(|| id.clone(), |entry| entry.name().to_owned());
                self.notices.push(Notice {
                    id: Some(id),
                    title: tr!("Cannot connect a USB device to “{name}”", name = name),
                    message,
                });
            }
            Update::Unit(id, state) => {
                if !state.is_running() {
                    self.usb.remove(&id);
                }
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.unit = state;
                    if !state.is_running() {
                        entry.paused = false;
                        entry.usage = None;
                        entry.history = UsageHistory::default();
                        entry.address = None;
                        entry.cpu_sample = None;
                        entry.listed.since = None;
                    } else if entry.listed.since.is_none() {
                        entry.listed.since = Some(SystemTime::now());
                    }
                    if settled(entry) {
                        entry.pending = None;
                    }
                }
            }
            Update::Paused(id, paused) => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.paused = paused;
                }
            }
            Update::Address(id, address) => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.address = address;
                }
            }
            Update::Opened(id) => self.to_open.push(id),
            Update::Without3d(id) => {
                let name = self
                    .entries
                    .get(&id)
                    .map_or_else(|| id.clone(), |entry| entry.name().to_owned());
                self.notices.push(Notice {
                    id: Some(id),
                    title: tr!("“{name}” started without 3D graphics", name = name),
                    message: tr!("The machine did not start with 3D graphics, so Velox started it with the plain display."),
                });
            }
            Update::Network(id, network) => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.network = (network.received, network.sent);
                    if network.address.is_some() {
                        entry.address = network.address;
                    }
                }
            }
            Update::IsoChosen(path) => self.chosen_iso = Some(path),
            Update::FolderChosen(..) => {}
            Update::Settled(id) => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.pending = None;
                }
            }
            Update::Job(job, phase) => {
                if let Some(entry) = self.jobs.get_mut(&job) {
                    entry.phase = phase;
                }
            }
            Update::Created(job, id) => {
                self.focus = Some(id.clone());
                self.created.push((job, id));
            }
            Update::JobFailed(job, message) => {
                if let Some(entry) = self.jobs.remove(&job)
                    && !message.is_empty()
                {
                    self.notices.push(Notice {
                        id: None,
                        title: tr!("Cannot create “{name}”", name = entry.name),
                        message,
                    });
                }
            }
            Update::Failed { id, act, message } => {
                let name = self
                    .entries
                    .get(&id)
                    .map_or_else(|| id.clone(), |entry| entry.name().to_owned());
                let title = match act {
                    Act::Start => text!("Cannot start “{name}”"),
                    Act::ShutDown => text!("Cannot shut down “{name}”"),
                    Act::ForceStop => text!("Cannot force “{name}” to stop"),
                    Act::Restart => text!("Cannot restart “{name}”"),
                    Act::Delete => text!("Cannot delete “{name}”"),
                    Act::Pause => text!("Cannot pause “{name}”"),
                    Act::Resume => text!("Cannot resume “{name}”"),
                    Act::TakeSnapshot => text!("Cannot take a snapshot of “{name}”"),
                    Act::Revert => text!("Cannot revert “{name}”"),
                    Act::DeleteSnapshot => text!("Cannot delete the snapshot of “{name}”"),
                    Act::Clone => text!("Cannot clone “{name}”"),
                    Act::Suspend => text!("Cannot suspend “{name}”"),
                };
                self.notices.push(Notice {
                    id: Some(id),
                    title: title.tr_args(&[("name", &name as &dyn std::fmt::Display)]),
                    message,
                });
            }
        }
    }
}
