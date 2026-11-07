use std::env;
use std::fs::{self, DirBuilder};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustix::process::getuid;
use rustix::rand::{GetRandomFlags, getrandom};

use crate::manifest::{Forward, Manifest};
use crate::qemu::{Host, utf8};
use crate::seed::CloudInit;
use crate::{Error, Result};

const NETWORK_ROOT: &str = "/tmp";
const CONTROL_SOCKET: &str = "control.sock";
const SERIAL_SOCKET: &str = "serial.sock";

pub struct Library {
    data: PathBuf,
    runtime: PathBuf,
    pub(crate) images: PathBuf,
}

pub struct NewMachine {
    pub name: String,
    pub os: String,
    pub cpus: u32,
    pub memory_mib: u64,
    pub disk_gib: u64,
    pub forwards: Vec<Forward>,
}

pub struct Machine {
    pub manifest: Manifest,
    dir: PathBuf,
    runtime: PathBuf,
    network: PathBuf,
}

impl Library {
    pub fn open() -> Result<Self> {
        let data = bande_xdg::data_home().ok_or(Error::Missing("XDG_DATA_HOME".into()))?;
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or(Error::Missing("XDG_RUNTIME_DIR".into()))?;
        Ok(Self {
            images: data.join("velox/images"),
            data: data.join("velox/vms"),
            runtime: runtime.join("velox"),
        })
    }

    pub fn at(data: PathBuf, runtime: PathBuf, images: PathBuf) -> Self {
        Self {
            data,
            runtime,
            images,
        }
    }

    pub fn machines(&self) -> Result<Vec<Machine>> {
        let entries = match fs::read_dir(&self.data) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            entries => entries?,
        };
        let mut machines = Vec::new();
        for entry in entries {
            let dir = entry?.path();
            let manifest = dir.join("manifest.json");
            if manifest.is_file() {
                machines.push(self.machine(Manifest::load(&manifest)?));
            }
        }
        machines.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        Ok(machines)
    }

    pub fn create(
        &self,
        host: &Host,
        new: NewMachine,
        base_image: &Path,
        cloud_init: &CloudInit,
    ) -> Result<Machine> {
        let id = random_uuid()?;
        let mac = random_mac()?;
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let machine = self.machine(Manifest {
            id,
            name: new.name,
            os: new.os,
            created,
            cpus: new.cpus,
            memory_mib: new.memory_mib,
            disk_gib: new.disk_gib,
            base_image: utf8(base_image)?.to_owned(),
            mac,
            forwards: new.forwards,
            snapshots: Vec::new(),
            current_snapshot: None,
            resume_from: None,
        });
        fs::create_dir_all(&machine.dir)?;
        let made = host
            .create_overlay(base_image, &machine.disk(), new.disk_gib)
            .and_then(|()| host.create_vars(&machine.vars()))
            .and_then(|()| cloud_init.write_seed(&machine.seed(), &machine.manifest.id))
            .and_then(|()| machine.save());
        if let Err(error) = made {
            let _ = fs::remove_dir_all(&machine.dir);
            return Err(error);
        }
        Ok(machine)
    }

    pub fn serial_socket(&self, id: &str) -> PathBuf {
        self.runtime.join(id).join(SERIAL_SOCKET)
    }

    pub fn control_socket(&self, id: &str) -> PathBuf {
        self.runtime.join(id).join(CONTROL_SOCKET)
    }

    pub fn delete(&self, host: &Host, machine: Machine) -> Result<()> {
        fs::remove_dir_all(&machine.dir)?;
        remove_folder(&machine.runtime)?;
        remove_folder(&machine.network)?;
        self.collect_linked(host)
    }

    pub(crate) fn machine(&self, manifest: Manifest) -> Machine {
        Machine {
            dir: self.data.join(&manifest.id),
            runtime: self.runtime.join(&manifest.id),
            network: network_base().join(&manifest.id),
            manifest,
        }
    }
}

impl Machine {
    pub fn save(&self) -> Result<()> {
        self.manifest.save(&self.dir.join("manifest.json"))
    }

    pub fn unit(&self) -> String {
        format!("velox-vm-{}.service", self.manifest.id)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn disk(&self) -> PathBuf {
        self.dir.join("disk.qcow2")
    }

    pub fn vars(&self) -> PathBuf {
        self.dir.join("vars.qcow2")
    }

    pub fn seed(&self) -> PathBuf {
        self.dir.join("seed.img")
    }

    pub fn console_log(&self) -> PathBuf {
        self.dir.join("console.log")
    }

    pub fn qmp_socket(&self) -> PathBuf {
        self.runtime.join("qmp.sock")
    }

    pub fn control_socket(&self) -> PathBuf {
        self.runtime.join(CONTROL_SOCKET)
    }

    pub fn agent_socket(&self) -> PathBuf {
        self.runtime.join("agent.sock")
    }

    pub fn serial_socket(&self) -> PathBuf {
        self.runtime.join(SERIAL_SOCKET)
    }

    pub(crate) fn runtime_dir(&self) -> &Path {
        &self.runtime
    }

    pub(crate) fn qmp_listening_marker(&self) -> PathBuf {
        self.control_socket()
    }

    pub(crate) fn passt_socket(&self) -> PathBuf {
        self.network.join("passt.sock")
    }

    pub(crate) fn passt_pid_file(&self) -> PathBuf {
        self.network.join("passt.pid")
    }

    pub(crate) fn network_unit(&self) -> String {
        format!("velox-vm-{}-network.service", self.manifest.id)
    }

    pub(crate) fn reset_runtime(&self) -> Result<()> {
        remove_folder(&self.runtime)?;
        fs::create_dir_all(&self.runtime)?;
        private_folder(&network_base())?;
        remove_folder(&self.network)?;
        fs::create_dir(&self.network)?;
        Ok(())
    }
}

fn remove_folder(folder: &Path) -> Result<()> {
    match fs::remove_dir_all(folder) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn network_base() -> PathBuf {
    PathBuf::from(NETWORK_ROOT).join(format!("velox-{}", getuid().as_raw()))
}

fn private_folder(folder: &Path) -> Result<()> {
    match DirBuilder::new().mode(0o700).create(folder) {
        Err(error) if error.kind() != ErrorKind::AlreadyExists => return Err(error.into()),
        _ => {}
    }
    let metadata = fs::symlink_metadata(folder)?;
    if !metadata.is_dir() || metadata.uid() != getuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(Error::Io(std::io::Error::other(format!(
            "{} is not a private folder",
            folder.display()
        ))));
    }
    Ok(())
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    let mut filled = 0;
    while filled < N {
        filled += getrandom(&mut bytes[filled..], GetRandomFlags::empty())?;
    }
    Ok(bytes)
}

pub(crate) fn random_id() -> Result<String> {
    let bytes = random_bytes::<8>()?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn random_uuid() -> Result<String> {
    let mut bytes = random_bytes::<16>()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

pub(crate) fn random_mac() -> Result<String> {
    let [a, b, c] = random_bytes::<3>()?;
    Ok(format!("52:54:00:{a:02x}:{b:02x}:{c:02x}"))
}
