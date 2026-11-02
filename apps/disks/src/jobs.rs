use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use bande_volumes::udisks::{
    BLOCK, ENCRYPTED, FILESYSTEM, LOOP, MANAGER, MANAGER_PATH, Objects, PARTITION, PARTITION_TABLE,
    UDISKS,
};
use serde::Serialize;
use zbus::blocking::Connection;
use zbus::zvariant::{DynamicType, Fd, ObjectPath, OwnedObjectPath, Value};
use zeroize::Zeroizing;

use crate::L10N;
use crate::model::Scheme;
use crate::view::Choose;

const DISMISSED: &str = "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed";
const CHUNK: usize = 4 << 20;
const PARTITION_START: u64 = 1 << 20;
const FILESYSTEMS: [&str; 10] = [
    "ext2", "ext3", "ext4", "vfat", "exfat", "ntfs", "xfs", "btrfs", "f2fs", "udf",
];
const ONLINE_GROW: u64 = 1 << 4;
const ONLINE_SHRINK: u64 = 1 << 3;
const OFFLINE_GROW: u64 = 1 << 2;
const OFFLINE_SHRINK: u64 = 1 << 1;
const LINUX_DATA: &str = "0fc63daf-8483-4772-8e79-3d69d8477de4";
const BASIC_DATA: &str = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Ext4,
    Ext4Encrypted,
    ExFat,
    Fat32,
    Ntfs,
}

impl Format {
    pub const ALL: [Self; 5] = [
        Self::Ext4,
        Self::Ext4Encrypted,
        Self::ExFat,
        Self::Fat32,
        Self::Ntfs,
    ];

    pub fn title(self) -> String {
        L10N.tr(match self {
            Self::Ext4 => "format-ext4",
            Self::Ext4Encrypted => "format-ext4-encrypted",
            Self::ExFat => "format-exfat",
            Self::Fat32 => "format-fat32",
            Self::Ntfs => "format-ntfs",
        })
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Ext4 | Self::Ext4Encrypted => "ext4",
            Self::ExFat => "exfat",
            Self::Fat32 => "vfat",
            Self::Ntfs => "ntfs",
        }
    }

    pub fn encrypted(self) -> bool {
        self == Self::Ext4Encrypted
    }

    pub fn label(self, name: &str) -> String {
        let (upper, limit) = match self {
            Self::Fat32 => (true, 11),
            Self::ExFat => (false, 15),
            Self::Ext4 | Self::Ext4Encrypted => (false, 16),
            Self::Ntfs => (false, 32),
        };
        let mut label = String::new();
        for character in name.trim().chars() {
            let character = match upper {
                true => character.to_ascii_uppercase(),
                false => character,
            };
            if label.len() + character.len_utf8() > limit {
                break;
            }
            label.push(character);
        }
        label
    }

    fn partition_type(self, scheme: Scheme) -> &'static str {
        match (scheme, self) {
            (Scheme::Gpt, Self::Ext4 | Self::Ext4Encrypted) => LINUX_DATA,
            (Scheme::Gpt, _) => BASIC_DATA,
            (Scheme::Mbr, Self::Ext4 | Self::Ext4Encrypted) => "0x83",
            (Scheme::Mbr, Self::Fat32) => "0x0c",
            (Scheme::Mbr, _) => "0x07",
        }
    }
}

pub type Passphrase = Option<Zeroizing<String>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erase {
    pub format: Format,
    pub name: String,
    pub passphrase: Passphrase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Mount(String),
    Unmount(String),
    Unlock(String, Zeroizing<String>),
    Eject(String),
    EraseVolume(String, Erase),
    EraseDisk(String, Scheme, Erase),
    AddPartition {
        table: String,
        scheme: Scheme,
        offset: u64,
        size: u64,
        erase: Erase,
    },
    DeletePartition(String),
    Resize {
        partition: String,
        size: u64,
        grow: bool,
    },
    FirstAid(String),
    Rename(String, String),
    CreateImage(String, PathBuf),
    Restore(String, PathBuf),
    MountImage(PathBuf),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Abilities {
    pub formats: Vec<Format>,
    pub check: HashSet<String>,
    pub repair: HashSet<String>,
    pub resize: HashMap<String, u64>,
}

impl Abilities {
    pub fn can_resize(&self, fs_type: &str, grow: bool, mounted: bool) -> bool {
        let wanted = match (grow, mounted) {
            (true, true) => ONLINE_GROW,
            (true, false) => OFFLINE_GROW,
            (false, true) => ONLINE_SHRINK,
            (false, false) => OFFLINE_SHRINK,
        };
        fs_type.is_empty()
            || self
                .resize
                .get(fs_type)
                .is_some_and(|mode| mode & wanted != 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Checked { repaired: bool },
    Loop(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Dismissed,
    Failed(String),
}

impl From<zbus::Error> for Failure {
    fn from(error: zbus::Error) -> Self {
        match error {
            zbus::Error::MethodError(name, _, _) if name.as_str() == DISMISSED => Self::Dismissed,
            zbus::Error::MethodError(_, Some(message), _) => Self::Failed(message),
            error => Self::Failed(error.to_string()),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Abilities(Abilities),
    Progress(f32),
    Finished(Result<Outcome, Failure>),
    Chosen(Choose, Option<PathBuf>),
}

#[derive(Clone)]
struct Out {
    sender: Sender<Report>,
    wake: Arc<UnixStream>,
}

impl Out {
    fn send(&self, report: Report) -> bool {
        self.sender.send(report).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

pub struct Worker {
    out: Out,
    jobs: Sender<Job>,
    reports: Receiver<Report>,
    wake: UnixStream,
    cancel: Arc<AtomicBool>,
}

type Options<'a> = HashMap<&'static str, Value<'a>>;

fn call<B: Serialize + DynamicType>(
    system: &Connection,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<zbus::Message, Failure> {
    let path = ObjectPath::try_from(path).map_err(zbus::Error::from)?;
    Ok(system.call_method(Some(UDISKS), &path, Some(interface), method, body)?)
}

fn none() -> Options<'static> {
    HashMap::new()
}

fn release(system: &Connection, blocks: &[String]) -> Result<Vec<String>, Failure> {
    let objects = Objects::load(system)?;
    let mut unmounted = Vec::new();
    for block in blocks {
        let opened = objects.path(block, ENCRYPTED, "CleartextDevice");
        for filesystem in [Some(block.clone()), opened.clone()].into_iter().flatten() {
            if !objects.mount_points(&filesystem).is_empty() {
                call(system, &filesystem, FILESYSTEM, "Unmount", &(none(),))?;
                unmounted.push(filesystem);
            }
        }
        if opened.is_some() {
            call(system, block, ENCRYPTED, "Lock", &(none(),))?;
        }
    }
    Ok(unmounted)
}

fn format_options<'a>(erase: &'a Erase, partition: bool) -> Options<'a> {
    let mut options = HashMap::from([("label", Value::from(erase.format.label(&erase.name)))]);
    if partition {
        options.insert("update-partition-type", Value::from(true));
    }
    if let Some(passphrase) = erase
        .passphrase
        .as_ref()
        .filter(|_| erase.format.encrypted())
    {
        options.insert("encrypt.passphrase", Value::from(passphrase.as_str()));
        options.insert("encrypt.type", Value::from("luks2"));
    }
    options
}

fn create_partition(
    system: &Connection,
    table: &str,
    scheme: Scheme,
    (offset, size): (u64, u64),
    erase: &Erase,
) -> Result<(), Failure> {
    let name = match scheme {
        Scheme::Gpt => erase.format.label(&erase.name),
        Scheme::Mbr => String::new(),
    };
    let body = (
        offset,
        size,
        erase.format.partition_type(scheme),
        name,
        none(),
        erase.format.kind(),
        format_options(erase, false),
    );
    call(
        system,
        table,
        PARTITION_TABLE,
        "CreatePartitionAndFormat",
        &body,
    )?;
    Ok(())
}

fn members(system: &Connection, block: &str) -> Result<Vec<String>, Failure> {
    let objects = Objects::load(system)?;
    let mut members = vec![block.to_owned()];
    members.extend(
        objects
            .paths()
            .map(|path| path.to_string())
            .filter(|path| objects.path(path, PARTITION, "Table").as_deref() == Some(block)),
    );
    Ok(members)
}

fn remount(system: &Connection, unmounted: &[String]) -> Result<(), Failure> {
    for filesystem in unmounted {
        call(system, filesystem, FILESYSTEM, "Mount", &(none(),))?;
    }
    Ok(())
}

fn resize(system: &Connection, partition: &str, size: u64, grow: bool) -> Result<(), Failure> {
    let objects = Objects::load(system)?;
    let filesystem = objects.has(partition, FILESYSTEM);
    let unmounted = release(system, &[partition.to_owned()])?;
    let partition_resize = || call(system, partition, PARTITION, "Resize", &(size, none()));
    if grow {
        partition_resize()?;
    }
    if filesystem {
        let target = match grow {
            true => 0u64,
            false => size,
        };
        call(system, partition, FILESYSTEM, "Resize", &(target, none()))?;
    }
    if !grow {
        partition_resize()?;
    }
    remount(system, &unmounted)
}

fn first_aid(system: &Connection, filesystem: &str) -> Result<Outcome, Failure> {
    let unmounted = release(system, &[filesystem.to_owned()])?;
    let consistent: bool = call(system, filesystem, FILESYSTEM, "Check", &(none(),))?
        .body()
        .deserialize()?;
    let repaired = match consistent {
        true => false,
        false => call(system, filesystem, FILESYSTEM, "Repair", &(none(),))?
            .body()
            .deserialize::<bool>()?,
    };
    remount(system, &unmounted)?;
    match consistent || repaired {
        true => Ok(Outcome::Checked { repaired }),
        false => Err(Failure::Failed(L10N.tr("first-aid-failed"))),
    }
}

fn open_device(system: &Connection, block: &str, method: &str) -> Result<File, Failure> {
    let reply = call(system, block, BLOCK, method, &(none(),))?;
    let fd: zbus::zvariant::OwnedFd = reply.body().deserialize()?;
    Ok(File::from(OwnedFd::from(fd)))
}

fn copy(
    mut from: impl Read,
    mut to: impl Write,
    total: u64,
    cancel: &AtomicBool,
    out: &Out,
) -> Result<(), Failure> {
    let mut buffer = vec![0; CHUNK];
    let mut done = 0u64;
    let mut shown = 0u64;
    loop {
        if cancel.swap(false, Ordering::Relaxed) {
            return Err(Failure::Failed(L10N.tr("copy-stopped")));
        }
        let read = from.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        to.write_all(buffer.get(..read).unwrap_or_default())?;
        done += read as u64;
        let percent = done.saturating_mul(100) / total.max(1);
        if percent != shown {
            shown = percent;
            out.send(Report::Progress(percent as f32 / 100.0));
        }
    }
}

fn create_image(
    system: &Connection,
    source: &str,
    file: &PathBuf,
    cancel: &AtomicBool,
    out: &Out,
) -> Result<(), Failure> {
    let unmounted = release(system, &members(system, source)?)?;
    let objects = Objects::load(system)?;
    let size: u64 = objects.get(source, BLOCK, "Size").unwrap_or(0);
    let device = open_device(system, source, "OpenForBackup")?;
    let image = File::create(file)?;
    let copied = copy(device, &image, size, cancel, out).and_then(|()| Ok(image.sync_all()?));
    if copied.is_err() {
        let _ = std::fs::remove_file(file);
    }
    copied?;
    remount(system, &unmounted)
}

fn restore(
    system: &Connection,
    target: &str,
    file: &PathBuf,
    cancel: &AtomicBool,
    out: &Out,
) -> Result<(), Failure> {
    let image = File::open(file)?;
    let size = image.metadata()?.len();
    let objects = Objects::load(system)?;
    let capacity: u64 = objects.get(target, BLOCK, "Size").unwrap_or(0);
    if size > capacity {
        return Err(Failure::Failed(L10N.tr("restore-too-big")));
    }
    release(system, &members(system, target)?)?;
    let device = open_device(system, target, "OpenForRestore")?;
    copy(image, &device, size, cancel, out)?;
    Ok(device.sync_all()?)
}

fn mount_image(system: &Connection, file: &PathBuf) -> Result<Outcome, Failure> {
    let iso = file
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("iso"));
    let writable = !iso && OpenOptions::new().read(true).write(true).open(file).is_ok();
    let image = OpenOptions::new().read(true).write(writable).open(file)?;
    let options = HashMap::from([("read-only", Value::from(!writable))]);
    let reply = call(
        system,
        MANAGER_PATH,
        MANAGER,
        "LoopSetup",
        &(Fd::from(image.as_fd()), options),
    )?;
    let device: OwnedObjectPath = reply.body().deserialize()?;
    call(system, &device, LOOP, "SetAutoclear", &(true, none()))?;
    Ok(Outcome::Loop(device.to_string()))
}

fn image_filter() -> bande_app::Filter {
    let patterns = ["*.img", "*.iso", "*.IMG", "*.ISO", "*.raw"];
    (
        L10N.tr("disk-images"),
        patterns
            .iter()
            .map(|pattern| (0, (*pattern).to_owned()))
            .collect(),
    )
}

fn abilities(system: &Connection) -> Abilities {
    let ask = |method: &str, kind: &str| -> Option<(bool, u64)> {
        let reply = call(system, MANAGER_PATH, MANAGER, method, &(kind,)).ok()?;
        match method {
            "CanResize" => reply
                .body()
                .deserialize::<(bool, u64, String)>()
                .ok()
                .map(|(available, mode, _)| (available, mode)),
            _ => reply
                .body()
                .deserialize::<(bool, String)>()
                .ok()
                .map(|(available, _)| (available, 0)),
        }
    };
    let able = |method: &str, kind: &str| ask(method, kind).is_some_and(|(available, _)| available);
    Abilities {
        formats: Format::ALL
            .into_iter()
            .filter(|format| able("CanFormat", format.kind()))
            .collect(),
        check: FILESYSTEMS
            .into_iter()
            .filter(|kind| able("CanCheck", kind))
            .map(str::to_owned)
            .collect(),
        repair: FILESYSTEMS
            .into_iter()
            .filter(|kind| able("CanRepair", kind))
            .map(str::to_owned)
            .collect(),
        resize: FILESYSTEMS
            .into_iter()
            .filter_map(|kind| match ask("CanResize", kind) {
                Some((true, mode)) => Some((kind.to_owned(), mode)),
                _ => None,
            })
            .collect(),
    }
}

fn perform(
    system: &Connection,
    job: Job,
    cancel: &AtomicBool,
    out: &Out,
) -> Result<Outcome, Failure> {
    match job {
        Job::Mount(filesystem) => {
            call(system, &filesystem, FILESYSTEM, "Mount", &(none(),))?;
        }
        Job::Unmount(block) => {
            release(system, &[block])?;
        }
        Job::Unlock(encrypted, passphrase) => {
            let reply = call(
                system,
                &encrypted,
                ENCRYPTED,
                "Unlock",
                &(passphrase.as_str(), none()),
            )?;
            let cleartext: OwnedObjectPath = reply.body().deserialize()?;
            call(system, &cleartext, FILESYSTEM, "Mount", &(none(),))?;
        }
        Job::Eject(disk) => {
            let objects = Objects::load(system)?;
            bande_volumes::eject_block(system, &objects, &disk, false)?;
        }
        Job::EraseVolume(block, erase) => {
            release(system, std::slice::from_ref(&block))?;
            let objects = Objects::load(system)?;
            let options = format_options(&erase, objects.has(&block, PARTITION));
            call(
                system,
                &block,
                BLOCK,
                "Format",
                &(erase.format.kind(), options),
            )?;
        }
        Job::EraseDisk(block, scheme, erase) => {
            release(system, &members(system, &block)?)?;
            let kind = match scheme {
                Scheme::Gpt => "gpt",
                Scheme::Mbr => "dos",
            };
            call(system, &block, BLOCK, "Format", &(kind, none()))?;
            create_partition(system, &block, scheme, (PARTITION_START, 0), &erase)?;
        }
        Job::AddPartition {
            table,
            scheme,
            offset,
            size,
            erase,
        } => create_partition(system, &table, scheme, (offset, size), &erase)?,
        Job::DeletePartition(block) => {
            release(system, std::slice::from_ref(&block))?;
            call(system, &block, PARTITION, "Delete", &(none(),))?;
        }
        Job::Resize {
            partition,
            size,
            grow,
        } => resize(system, &partition, size, grow)?,
        Job::FirstAid(filesystem) => return first_aid(system, &filesystem),
        Job::Rename(filesystem, label) => {
            call(
                system,
                &filesystem,
                FILESYSTEM,
                "SetLabel",
                &(label, none()),
            )?;
        }
        Job::CreateImage(source, file) => create_image(system, &source, &file, cancel, out)?,
        Job::Restore(target, file) => restore(system, &target, &file, cancel, out)?,
        Job::MountImage(file) => return mount_image(system, &file),
    }
    Ok(Outcome::Done)
}

impl Worker {
    pub fn start(system: Connection) -> io::Result<Self> {
        let (wake, ring) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let (jobs, queue) = mpsc::channel::<Job>();
        let (sender, reports) = mpsc::channel();
        let out = Out {
            sender,
            wake: Arc::new(ring),
        };
        let shared = out.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        thread::Builder::new()
            .name("disks".to_owned())
            .spawn(move || {
                if !out.send(Report::Abilities(abilities(&system))) {
                    return;
                }
                for job in queue {
                    stop.store(false, Ordering::Relaxed);
                    let result = perform(&system, job, &stop, &out);
                    if let Err(Failure::Failed(message)) = &result {
                        tracing::warn!("{message}");
                    }
                    if !out.send(Report::Finished(result)) {
                        return;
                    }
                }
            })?;
        Ok(Self {
            out: shared,
            jobs,
            reports,
            wake,
            cancel,
        })
    }

    pub fn run(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::warn!("the disk worker stopped");
        }
    }

    pub fn choose(&self, session: Connection, choose: Choose) {
        let out = self.out.clone();
        let asked = thread::Builder::new()
            .name("chooser".to_owned())
            .spawn(move || {
                let chosen = match &choose {
                    Choose::SaveImage(_, name) => bande_app::choose_save_file(
                        &session,
                        &L10N.tr("save-image"),
                        &format!("{name}.img"),
                    ),
                    Choose::Restore(_) => bande_app::choose_file(
                        &session,
                        &L10N.tr("choose-image"),
                        Some(image_filter()),
                    ),
                    Choose::MountImage => bande_app::choose_file(
                        &session,
                        &L10N.tr("open-image"),
                        Some(image_filter()),
                    ),
                };
                let file = chosen.unwrap_or_else(|error| {
                    tracing::warn!("cannot ask for a file: {error}");
                    None
                });
                out.send(Report::Chosen(choose, file));
            });
        if let Err(error) = asked {
            tracing::warn!("cannot ask for a file: {error}");
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<Report> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.reports.try_iter().collect()
    }
}
