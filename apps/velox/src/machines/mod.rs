mod backend;
mod console;
mod display;
mod fleet;
mod page;
mod pane;
mod present;
mod readiness;
mod sheet;
mod snapshots;
mod widgets;
mod window;

use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use self::backend::Act;
use self::backend::{Backend, Change, Plan, SnapshotOp};
pub use self::display::DisplayWindow;
pub use self::fleet::{Fleet, Notice};
pub use self::pane::{INSPECTOR_COMMAND, MACHINES_KEY, MachinesPane};
use self::sheet::Field;
pub use self::window::{MACHINE_WINDOW_SIZE, MachineWindow};
use crate::L10N;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
pub(crate) const MACHINES_SYMBOL: &str = include_str!("symbols/machines.svg");

#[derive(Debug, Clone, PartialEq)]
enum Intent {
    Select(Vec<String>),
    Activate(Vec<String>),
    Filter(String),
    Act(Act),
    New,
    Inspector,
    Menu(String, Vec<String>),
    Emulate,
    Copy(&'static str),
    Field(Field),
    Sheet(String),
    Delete(String, bool),
    Tab(usize),
    Change(Change),
    Connect,
    RetryConsole,
    Choice(String),
    Snapshot(Vec<String>),
    SnapshotTool(snapshots::Tool),
    Text(String),
    Port(String),
    Linked(bool),
    ChooseIso(bool),
    Eject,
    Graphics3d(bool),
    StartAtLogin(bool),
    AddShare,
    RemoveShare(String),
}

type Intents = Rc<RefCell<Vec<Intent>>>;

fn push(intents: &Intents, intent: Intent) {
    intents.borrow_mut().push(intent);
}

pub enum Request {
    Act(String, Act),
    Create(Box<Plan>),
    Cancel(u64),
    Open(String),
    Connect(String),
    Emulate,
    Change(String, Change),
    Snapshot(String, SnapshotOp),
    Clone(String, bande_vm::NewClone),
    PresentClone(String),
    PresentSnapshot(String),
    ChooseIso(bool),
    ChooseFolder(String),
    Serial(String),
    Usb(String, String),
}

pub enum Effect {
    Serial(crate::shell::ShellTarget),
    Open(String),
    PresentClone(String),
    PresentSnapshot(String),
    Connect(Vec<String>),
}

pub struct Hub {
    backend: Backend,
    fleet: Rc<RefCell<Fleet>>,
    sample_at: Option<Instant>,
}

impl Hub {
    pub fn start() -> std::io::Result<Self> {
        Ok(Self {
            backend: Backend::start()?,
            fleet: Rc::default(),
            sample_at: None,
        })
    }

    pub(crate) fn fleet(&self) -> Rc<RefCell<Fleet>> {
        Rc::clone(&self.fleet)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.backend.fd()
    }

    pub fn drain(&self) -> bool {
        let updates = self.backend.drain();
        let changed = !updates.is_empty();
        let mut fleet = self.fleet.borrow_mut();
        for update in updates {
            if let backend::Update::FolderChosen(id, path) = update {
                self.backend.change(id, Change::AddShare(path));
                continue;
            }
            fleet.apply(update);
        }
        changed
    }

    pub fn take_windows_to_open(&self) -> Vec<String> {
        std::mem::take(&mut self.fleet.borrow_mut().to_open)
    }

    pub fn usb_choices(&self, id: &str) -> Option<Vec<(String, String, bool)>> {
        let fleet = self.fleet.borrow();
        if !fleet.entry(id)?.running() {
            return None;
        }
        let attached = fleet.usb.get(id);
        Some(
            fleet
                .usb_devices
                .iter()
                .map(|device| {
                    let key = device.key();
                    let on = attached.is_some_and(|keys| keys.contains(&key));
                    (key, device.name.clone(), on)
                })
                .collect(),
        )
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut self.fleet.borrow_mut().notices)
    }

    pub fn serve(&mut self, request: Request) -> Option<Effect> {
        let mut fleet = self.fleet.borrow_mut();
        match request {
            Request::Act(id, act) => {
                fleet.mark(&id, act);
                self.backend.act(id, act);
            }
            Request::Create(plan) => {
                let (name, os) = (plan.new.name.clone(), plan.new.os.clone());
                let job = self.backend.create(*plan);
                fleet.add_job(job, name, os);
                fleet.focus = Some(format!("{}{job}", present::JOB_PREFIX));
            }
            Request::Cancel(job) => self.backend.cancel(job),
            Request::Open(id) => return Some(Effect::Open(id)),
            Request::Connect(id) => {
                return fleet
                    .entry(&id)
                    .and_then(|entry| entry.ssh_command())
                    .map(Effect::Connect);
            }
            Request::Snapshot(id, op) => {
                fleet.mark(&id, op.act());
                self.backend.snapshot(id, op);
            }
            Request::Clone(id, new) => {
                let (name, os) = (
                    new.name.clone(),
                    fleet
                        .entry(&id)
                        .map(|entry| entry.listed.manifest.os.clone())
                        .unwrap_or_default(),
                );
                fleet.mark(&id, Act::Clone);
                let job = self.backend.clone_machine(id, new);
                fleet.add_job(job, name, os);
                fleet.set_job_phase(job);
            }
            Request::Serial(id) => {
                let socket = bande_vm::Library::open().ok()?.serial_socket(&id);
                let name = fleet.entry(&id)?.name().to_owned();
                return Some(Effect::Serial(crate::shell::ShellTarget::Serial {
                    machine: id,
                    name,
                    socket,
                }));
            }
            Request::PresentClone(id) => return Some(Effect::PresentClone(id)),
            Request::PresentSnapshot(id) => return Some(Effect::PresentSnapshot(id)),
            Request::ChooseIso(disk) => self.backend.choose_iso(
                L10N.tr(match disk {
                    true => "machine-disk-choose",
                    false => "machine-iso-choose",
                }),
                disk,
            ),
            Request::ChooseFolder(id) => {
                self.backend
                    .choose_folder(id, L10N.tr("machine-share-choose"));
            }
            Request::Emulate => self.backend.emulate(),
            Request::Change(id, change) => self.backend.change(id, change),
            Request::Usb(id, key) => self.backend.usb(id, key),
        }
        None
    }

    pub fn schedule(&mut self, wanted: bool) {
        let wanted = wanted && self.fleet.borrow().running() > 0;
        self.sample_at = match (wanted, self.sample_at) {
            (false, _) => None,
            (true, None) => Some(Instant::now()),
            (true, at) => at,
        };
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.sample_at
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        match self.sample_at {
            Some(at) if at <= now => {
                let running: Vec<String> = self
                    .fleet
                    .borrow()
                    .entries
                    .values()
                    .filter(|entry| entry.running())
                    .map(|entry| entry.id().to_owned())
                    .collect();
                self.backend.sample_network(running);
                self.fleet.borrow_mut().sample(now);
                self.sample_at = Some(now + SAMPLE_INTERVAL);
                true
            }
            _ => false,
        }
    }
}
