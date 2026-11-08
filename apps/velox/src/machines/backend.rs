use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::ops::ControlFlow;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use bande_vm::{
    ActiveState, CloudImage, CloudInit, Forward, Host, ImageStore, Installer, Library, Machine,
    Manifest, NewClone, NewMachine, Qmp, Share, Units,
};
use bande_watch::{Changes, FolderWatch};
use serde::{Deserialize, Serialize};

use super::readiness::{self, Blocker};

const LOGIN_FILE: &str = "velox.json";
const SSH_PORT: u16 = 22;
const READINESS_FOLDERS: [&str; 2] = ["/var/lib/dpkg", "/etc"];
const PAUSED: &str = "STOP";
const RESUMED: &str = "RESUME";
const SHARE_TAG_LIMIT: usize = 30;
const AGENT_PORT: &str = "VSERPORT_CHANGE";
const AGENT_ID: &str = "agent-port";
const ADDRESS_WAITS: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub user: String,
    pub identity: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Listed {
    pub manifest: Manifest,
    pub login: Login,
    pub unit: ActiveState,
    pub since: Option<SystemTime>,
    pub dir: PathBuf,
    pub starts_at_login: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Start,
    ShutDown,
    ForceStop,
    Restart,
    Delete,
    Pause,
    Resume,
    Suspend,
    TakeSnapshot,
    Revert,
    DeleteSnapshot,
    Clone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotOp {
    Take(String),
    Revert(String),
    Delete(String),
}

impl SnapshotOp {
    pub fn act(&self) -> Act {
        match self {
            Self::Take(_) => Act::TakeSnapshot,
            Self::Revert(_) => Act::Revert,
            Self::Delete(_) => Act::DeleteSnapshot,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Name(String),
    Cpus(u32),
    Memory(u64),
    SshPort(u16),
    Eject,
    Graphics3d(bool),
    StartAtLogin(bool),
    AddShare(PathBuf),
    RemoveShare(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Downloading(Option<f32>),
    Creating,
}

pub enum Source {
    Cloud(CloudImage, CloudInit),
    Iso(Installer),
}

pub struct Plan {
    pub source: Source,
    pub new: NewMachine,
    pub login: Login,
}

#[derive(Debug)]
pub enum Update {
    Ready(bande_vm::Accel),
    Blocked(Blocker),
    Listed(Vec<Listed>),
    Unit(String, ActiveState),
    Paused(String, bool),
    Address(String, Option<std::net::Ipv4Addr>),
    Network(String, bande_vm::GuestNetwork),
    Opened(String),
    Without3d(String),
    IsoChosen(PathBuf),
    FolderChosen(String, PathBuf),
    Settled(String),
    Job(u64, Phase),
    Created(u64, String),
    JobFailed(u64, String),
    Failed {
        id: String,
        act: Act,
        message: String,
    },
}

#[derive(Clone)]
struct Out {
    sender: Sender<Update>,
    wake: Arc<UnixStream>,
}

impl Out {
    fn send(&self, update: Update) -> bool {
        self.sender.send(update).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

enum Recheck {
    Probe,
    Relist,
}

#[derive(Default)]
struct Shared {
    stopped: AtomicBool,
    emulate: AtomicBool,
    host: Mutex<Option<Host>>,
    units: Mutex<Option<Arc<Units>>>,
    watched: Mutex<HashSet<String>>,
    monitors: Mutex<HashMap<String, UnixStream>>,
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Backend {
    updates: Receiver<Update>,
    wake: UnixStream,
    out: Out,
    recheck: Sender<Recheck>,
    shared: Arc<Shared>,
    next_job: u64,
    _watch: Option<FolderWatch>,
}

#[derive(Clone)]
struct Worker {
    shared: Arc<Shared>,
    out: Out,
    recheck: Sender<Recheck>,
}

impl Worker {
    fn host(&self) -> Option<Host> {
        lock(&self.shared.host).clone()
    }

    fn units(&self) -> Option<Arc<Units>> {
        lock(&self.shared.units).clone()
    }

    fn library() -> Result<Library, String> {
        Library::open().map_err(|error| error.to_string())
    }

    fn machine(id: &str) -> Result<Machine, String> {
        Self::library()?
            .machines()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|machine| machine.manifest.id == id)
            .ok_or_else(|| "the machine is gone".to_owned())
    }

    fn probe(&self) {
        let emulate = self.shared.emulate.load(Ordering::SeqCst);
        let host = match readiness::probe(emulate) {
            Ok(host) => host,
            Err(blocker) => {
                *lock(&self.shared.host) = None;
                self.out.send(Update::Blocked(blocker));
                return;
            }
        };
        if self.units().is_none() {
            match Units::connect() {
                Ok(units) => *lock(&self.shared.units) = Some(Arc::new(units)),
                Err(error) => {
                    tracing::warn!("cannot reach the systemd user manager: {error}");
                    self.out.send(Update::Blocked(Blocker::NoManager));
                    return;
                }
            }
        }
        let accel = host.accel;
        *lock(&self.shared.host) = Some(host);
        if self.out.send(Update::Ready(accel)) {
            self.relist();
        }
    }

    fn relist(&self) {
        let Some(units) = self.units() else {
            return;
        };
        let machines = match Library::open().and_then(|library| library.machines()) {
            Ok(machines) => machines,
            Err(error) => {
                tracing::warn!("cannot list the virtual machines: {error}");
                return;
            }
        };
        let mut listed = Vec::new();
        for machine in machines {
            let unit = units
                .state(&machine.unit())
                .unwrap_or(ActiveState::Inactive);
            if unit == ActiveState::Active {
                self.watch(&machine, None);
            }
            listed.push(Listed {
                login: read_login(machine.dir()),
                since: unit
                    .is_running()
                    .then(|| fs::metadata(machine.qmp_socket()).and_then(|meta| meta.modified()))
                    .and_then(Result::ok),
                dir: machine.dir().to_path_buf(),
                starts_at_login: machine.starts_at_login(),
                manifest: machine.manifest,
                unit,
            });
        }
        self.out.send(Update::Listed(listed));
    }

    fn watch(&self, machine: &Machine, qmp: Option<Qmp>) {
        let id = machine.manifest.id.clone();
        let unit = machine.unit();
        if let Some(units) = self.units()
            && lock(&self.shared.watched).insert(id.clone())
        {
            let worker = self.clone();
            let id = id.clone();
            spawn("velox-vm-unit", move || {
                if let Ok((state, events)) = units.watch(&unit) {
                    let mut state = Some(state);
                    let mut events = events;
                    while let Some(current) = state.take().or_else(|| events.next()) {
                        if worker.shared.stopped.load(Ordering::SeqCst)
                            || !worker.out.send(Update::Unit(id.clone(), current))
                            || !current.is_running()
                        {
                            break;
                        }
                    }
                }
                lock(&worker.shared.watched).remove(&id);
            });
        }
        if lock(&self.shared.monitors).contains_key(&id) {
            return;
        }
        let Some(mut qmp) = qmp.or_else(|| Qmp::connect(&machine.qmp_socket()).ok()) else {
            return;
        };
        let Ok(handle) = qmp.as_fd().try_clone_to_owned().map(UnixStream::from) else {
            return;
        };
        lock(&self.shared.monitors).insert(id.clone(), handle);
        let worker = self.clone();
        let agent = machine.agent_socket();
        self.look_up_address(&id, agent.clone());
        spawn("velox-vm-qmp", move || {
            if let Ok(status) = qmp.status() {
                worker.out.send(Update::Paused(
                    id.clone(),
                    status == bande_vm::RunState::Paused,
                ));
            }
            while let Ok(event) = qmp.next_event() {
                let paused = match event.name.as_str() {
                    PAUSED => true,
                    RESUMED => false,
                    AGENT_PORT if event.data["open"] == true && event.data["id"] == AGENT_ID => {
                        worker.look_up_address(&id, agent.clone());
                        continue;
                    }
                    _ => continue,
                };
                if !worker.out.send(Update::Paused(id.clone(), paused)) {
                    break;
                }
            }
            lock(&worker.shared.monitors).remove(&id);
        });
    }

    fn look_up_address(&self, id: &str, agent: PathBuf) {
        let worker = self.clone();
        let id = id.to_owned();
        spawn("velox-vm-address", move || {
            for wait in ADDRESS_WAITS {
                std::thread::sleep(wait);
                if worker.shared.stopped.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(Some(address)) = bande_vm::guest_address(&agent) {
                    worker.out.send(Update::Address(id, Some(address)));
                    return;
                }
            }
        });
    }

    fn start(&self, machine: &Machine) -> Result<(), String> {
        let (Some(host), Some(units)) = (self.host(), self.units()) else {
            return Err("virtualization is not ready".to_owned());
        };
        let (qmp, without_3d) = units
            .boot(&host, machine)
            .map_err(|error| error.to_string())?;
        if without_3d {
            self.out
                .send(Update::Without3d(machine.manifest.id.clone()));
        }
        self.watch(machine, Some(qmp));
        Ok(())
    }

    fn snapshot(&self, id: &str, op: SnapshotOp) -> Result<(), String> {
        let (Some(host), Some(units)) = (self.host(), self.units()) else {
            return Err("virtualization is not ready".to_owned());
        };
        let text = |error: bande_vm::Error| error.to_string();
        let mut machine = Self::machine(id)?;
        let running = units.state(&machine.unit()).map_err(text)?.is_running();
        let mut qmp = match running {
            true => Some(Qmp::connect(&machine.control_socket()).map_err(text)?),
            false => None,
        };
        match op {
            SnapshotOp::Take(name) => machine
                .take_snapshot(&host, qmp.as_mut(), &name)
                .map_err(text),
            SnapshotOp::Revert(tag) => {
                machine
                    .revert_snapshot(&host, qmp.as_mut(), &tag)
                    .map_err(text)?;
                match machine.manifest.resume_from.is_some() {
                    true => self.start(&machine),
                    false => Ok(()),
                }
            }
            SnapshotOp::Delete(tag) => machine
                .delete_snapshot(&host, qmp.as_mut(), &tag)
                .map_err(text),
        }
    }

    fn until_stopped(
        &self,
        machine: &Machine,
        stop: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let units = self.units().ok_or("virtualization is not ready")?;
        let (state, events) = units
            .watch(&machine.unit())
            .map_err(|error| error.to_string())?;
        if !state.is_running() {
            return Ok(());
        }
        stop()?;
        for state in events {
            if !state.is_running() {
                break;
            }
        }
        Ok(())
    }

    fn power_down(machine: &Machine) -> Result<(), String> {
        Qmp::connect(&machine.control_socket())
            .and_then(|mut qmp| qmp.shut_down())
            .map_err(|error| error.to_string())
    }

    fn act(&self, id: &str, act: Act) -> Result<(), String> {
        let machine = Self::machine(id)?;
        match act {
            Act::Start => self.start(&machine),
            Act::ShutDown => self.until_stopped(&machine, || Self::power_down(&machine)),
            Act::ForceStop => self.until_stopped(&machine, || {
                let units = self.units().ok_or("virtualization is not ready")?;
                units
                    .stop(&machine.unit())
                    .map_err(|error| error.to_string())
            }),
            Act::Restart => {
                self.until_stopped(&machine, || Self::power_down(&machine))?;
                self.start(&machine)
            }
            Act::Delete => {
                let units = self.units().ok_or("virtualization is not ready")?;
                if units
                    .state(&machine.unit())
                    .map_err(|error| error.to_string())?
                    .is_running()
                {
                    return Err("shut the machine down first".to_owned());
                }
                let host = self.host().ok_or("virtualization is not ready")?;
                if let Err(error) = units.remove_autostart(&machine) {
                    tracing::warn!("cannot remove the login start: {error}");
                }
                Self::library()?
                    .delete(&host, machine)
                    .map_err(|error| error.to_string())
            }
            Act::Pause => Qmp::connect(&machine.control_socket())
                .and_then(|mut qmp| qmp.pause())
                .map_err(|error| error.to_string()),
            Act::Resume => Qmp::connect(&machine.control_socket())
                .and_then(|mut qmp| qmp.resume())
                .map_err(|error| error.to_string()),
            Act::Suspend => self.until_stopped(&machine, || {
                let mut suspended = Self::machine(id)?;
                let mut qmp =
                    Qmp::connect(&suspended.control_socket()).map_err(|error| error.to_string())?;
                suspended
                    .suspend(&mut qmp)
                    .map_err(|error| error.to_string())
            }),
            Act::TakeSnapshot | Act::Revert | Act::DeleteSnapshot | Act::Clone => Ok(()),
        }
    }

    fn change(&self, id: &str, change: Change) -> Result<(), String> {
        let mut machine = Self::machine(id)?;
        if let Change::StartAtLogin(on) = change {
            let units = self.units().ok_or("virtualization is not ready")?;
            let program = std::env::current_exe().map_err(|error| error.to_string())?;
            return units
                .set_starts_at_login(&machine, &program, on)
                .map_err(|error| error.to_string());
        }
        let manifest = &mut machine.manifest;
        match change {
            Change::Name(name) => manifest.name = name,
            Change::Cpus(cpus) => manifest.cpus = cpus,
            Change::Memory(memory) => manifest.memory_mib = memory,
            Change::Eject => manifest.installer = None,
            Change::Graphics3d(on) => manifest.graphics_3d = on,
            Change::StartAtLogin(_) => {}
            Change::AddShare(path) => {
                let tag = share_tag(&path, &manifest.shares);
                manifest.shares.push(Share {
                    tag,
                    path: path.display().to_string(),
                });
            }
            Change::RemoveShare(tag) => manifest.shares.retain(|share| share.tag != tag),
            Change::SshPort(port) => {
                manifest
                    .forwards
                    .retain(|forward| forward.guest != SSH_PORT);
                manifest.forwards.push(Forward {
                    host: port,
                    guest: SSH_PORT,
                });
            }
        }
        machine.save().map_err(|error| error.to_string())
    }

    fn clone_machine(&self, id: &str, new: NewClone) -> Result<String, String> {
        let host = self.host().ok_or("virtualization is not ready")?;
        let units = self.units().ok_or("virtualization is not ready")?;
        let text = |error: bande_vm::Error| error.to_string();
        let mut source = Self::machine(id)?;
        if units.state(&source.unit()).map_err(text)?.is_running() {
            return Err("shut the machine down first".to_owned());
        }
        let clone = Self::library()?
            .clone_machine(&host, &mut source, new)
            .map_err(text)?;
        write_login(clone.dir(), &read_login(source.dir())).map_err(|error| error.to_string())?;
        Ok(clone.manifest.id)
    }

    fn create(&self, job: u64, plan: Plan, cancelled: &AtomicBool) -> Result<String, String> {
        let mut host = Host::detect().map_err(|error| error.to_string())?;
        host.accel = self.host().ok_or("virtualization is not ready")?.accel;
        let (image, cloud_init) = match plan.source {
            Source::Cloud(image, cloud_init) => (image, cloud_init),
            Source::Iso(installer) => {
                self.out.send(Update::Job(job, Phase::Creating));
                let machine = Self::library()?
                    .create_from_iso(&host, plan.new, &installer)
                    .map_err(|error| error.to_string())?;
                write_login(machine.dir(), &plan.login).map_err(|error| error.to_string())?;
                self.out.send(Update::Opened(machine.manifest.id.clone()));
                return Ok(machine.manifest.id);
            }
        };
        let store = ImageStore::open().map_err(|error| error.to_string())?;
        let mut shown = None;
        let base = store
            .fetch(&image, &mut |done, total| {
                let fraction = total
                    .filter(|total| *total > 0)
                    .map(|total| (done as f64 / total as f64 * 100.0).floor() as f32 / 100.0);
                if shown != Some(fraction) {
                    shown = Some(fraction);
                    self.out
                        .send(Update::Job(job, Phase::Downloading(fraction)));
                }
                match cancelled.load(Ordering::SeqCst) {
                    true => ControlFlow::Break(()),
                    false => ControlFlow::Continue(()),
                }
            })
            .map_err(|error| match error {
                bande_vm::Error::Cancelled => String::new(),
                error => error.to_string(),
            })?;
        self.out.send(Update::Job(job, Phase::Creating));
        let machine = Self::library()?
            .create(&host, plan.new, &base, &cloud_init)
            .map_err(|error| error.to_string())?;
        write_login(machine.dir(), &plan.login).map_err(|error| error.to_string())?;
        Ok(machine.manifest.id)
    }
}

fn share_tag(path: &Path, shares: &[Share]) -> String {
    let base: String = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "share".to_owned())
        .chars()
        .map(|character| match character.is_ascii_alphanumeric() {
            true => character,
            false => '-',
        })
        .take(SHARE_TAG_LIMIT)
        .collect();
    (1..)
        .map(|index| match index {
            1 => base.clone(),
            index => format!("{base}-{index}"),
        })
        .find(|tag| !shares.iter().any(|share| share.tag == *tag))
        .unwrap_or(base)
}

fn read_login(dir: &Path) -> Login {
    fs::read(dir.join(LOGIN_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_login(dir: &Path, login: &Login) -> std::io::Result<()> {
    bande_xdg::replace_file(&dir.join(LOGIN_FILE), serde_json::to_vec_pretty(login)?)
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start {name}: {error}");
    }
}

impl Backend {
    pub fn start() -> std::io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let (sender, updates) = mpsc::channel();
        let out = Out {
            sender,
            wake: Arc::new(writer),
        };
        let (recheck, rechecks) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let vms = bande_xdg::data_home().map(|data| data.join("velox/vms"));
        if let Some(vms) = &vms
            && let Err(error) = fs::create_dir_all(vms)
        {
            tracing::warn!("cannot make {}: {error}", vms.display());
        }
        let notify = recheck.clone();
        let library = vms.clone();
        let watch = FolderWatch::spawn(Changes::Any, move |paths| {
            let relist = paths
                .iter()
                .any(|path| library.as_deref().is_some_and(|vms| path.starts_with(vms)));
            let _ = notify.send(match relist {
                true => Recheck::Relist,
                false => Recheck::Probe,
            });
        })
        .inspect_err(|error| tracing::warn!("cannot watch for virtualization: {error}"))
        .ok();
        if let Some(watch) = &watch {
            for folder in READINESS_FOLDERS.iter().map(PathBuf::from).chain(vms) {
                if let Err(error) = watch.watch(&folder) {
                    tracing::debug!("cannot watch {}: {error}", folder.display());
                }
            }
        }
        let worker = Worker {
            shared: Arc::clone(&shared),
            out: out.clone(),
            recheck: recheck.clone(),
        };
        std::thread::Builder::new()
            .name("velox-machines".to_owned())
            .spawn(move || {
                worker.probe();
                while let Ok(recheck) = rechecks.recv() {
                    if worker.shared.stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    match recheck {
                        Recheck::Probe => worker.probe(),
                        Recheck::Relist => worker.relist(),
                    }
                }
            })?;
        Ok(Self {
            updates,
            wake: reader,
            out,
            recheck,
            shared,
            next_job: 0,
            _watch: watch,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<Update> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.updates.try_iter().collect()
    }

    fn worker(&self) -> Worker {
        Worker {
            shared: Arc::clone(&self.shared),
            out: self.out.clone(),
            recheck: self.recheck.clone(),
        }
    }

    fn probe(&self) {
        let _ = self.recheck.send(Recheck::Probe);
    }

    pub fn emulate(&self) {
        self.shared.emulate.store(true, Ordering::SeqCst);
        self.probe();
    }

    pub fn act(&self, id: String, act: Act) {
        let worker = self.worker();
        spawn("velox-vm-action", move || {
            if let Err(message) = worker.act(&id, act) {
                worker.out.send(Update::Failed {
                    id: id.clone(),
                    act,
                    message,
                });
            }
            worker.out.send(Update::Settled(id));
            let _ = worker.recheck.send(Recheck::Relist);
        });
    }

    pub fn change(&self, id: String, change: Change) {
        let worker = self.worker();
        spawn("velox-vm-change", move || {
            if let Err(message) = worker.change(&id, change) {
                tracing::warn!("cannot change the machine: {message}");
            }
            let _ = worker.recheck.send(Recheck::Relist);
        });
    }

    pub fn create(&mut self, plan: Plan) -> u64 {
        let job = self.next_job;
        self.next_job += 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        lock(&self.shared.jobs).insert(job, Arc::clone(&cancelled));
        let worker = self.worker();
        spawn("velox-vm-create", move || {
            let created = worker.create(job, plan, &cancelled);
            lock(&worker.shared.jobs).remove(&job);
            match created {
                Ok(id) => {
                    worker.out.send(Update::Created(job, id.clone()));
                    worker.relist();
                    let started = Worker::machine(&id).and_then(|machine| worker.start(&machine));
                    if let Err(message) = started {
                        worker.out.send(Update::Failed {
                            id: id.clone(),
                            act: Act::Start,
                            message,
                        });
                    }
                    worker.out.send(Update::Settled(id));
                }
                Err(message) => {
                    worker.out.send(Update::JobFailed(job, message));
                }
            }
        });
        job
    }

    pub fn sample_network(&self, ids: Vec<String>) {
        let worker = self.worker();
        spawn("velox-vm-network", move || {
            for id in ids {
                let Ok(machine) = Worker::machine(&id) else {
                    continue;
                };
                if let Ok(network) = bande_vm::guest_network(&machine.agent_socket()) {
                    worker.out.send(Update::Network(id, network));
                }
            }
        });
    }

    pub fn choose_iso(&self, title: String, disk: bool) {
        let worker = self.worker();
        let choose = match disk {
            true => crate::volumes::portal::choose_disk,
            false => crate::volumes::portal::choose_iso,
        };
        spawn("velox-vm-choose", move || match choose(&title) {
            Ok(Some(path)) => {
                worker.out.send(Update::IsoChosen(path));
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("cannot ask for a disc image: {error}"),
        });
    }

    pub fn choose_folder(&self, id: String, title: String) {
        let worker = self.worker();
        spawn(
            "velox-vm-choose",
            move || match crate::volumes::portal::choose_folder(&title) {
                Ok(Some(path)) => {
                    worker.out.send(Update::FolderChosen(id, path));
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot ask for a folder: {error}"),
            },
        );
    }

    pub fn snapshot(&self, id: String, op: SnapshotOp) {
        let worker = self.worker();
        spawn("velox-vm-snapshot", move || {
            let act = op.act();
            if let Err(message) = worker.snapshot(&id, op) {
                worker.out.send(Update::Failed {
                    id: id.clone(),
                    act,
                    message,
                });
            }
            worker.out.send(Update::Settled(id));
            let _ = worker.recheck.send(Recheck::Relist);
        });
    }

    pub fn clone_machine(&mut self, id: String, new: NewClone) -> u64 {
        let job = self.next_job;
        self.next_job += 1;
        let worker = self.worker();
        spawn("velox-vm-clone", move || {
            let cloned = worker.clone_machine(&id, new);
            worker.out.send(Update::Settled(id.clone()));
            match cloned {
                Ok(clone) => {
                    worker.out.send(Update::Created(job, clone.clone()));
                    worker.relist();
                    worker.out.send(Update::Settled(clone));
                }
                Err(message) => {
                    worker.out.send(Update::JobFailed(job, message));
                    let _ = worker.recheck.send(Recheck::Relist);
                }
            }
        });
        job
    }

    pub fn cancel(&self, job: u64) {
        if let Some(cancelled) = lock(&self.shared.jobs).get(&job) {
            cancelled.store(true, Ordering::SeqCst);
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        for (_, monitor) in lock(&self.shared.monitors).drain() {
            let _ = monitor.shutdown(Shutdown::Both);
        }
        let _ = self.recheck.send(Recheck::Probe);
    }
}
