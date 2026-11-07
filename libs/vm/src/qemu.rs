use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bande_xdg::find_executable;
use serde_json::{Value, json};

use crate::library::Machine;
use crate::{Error, Result};

const FIRMWARE_CODE: &str = "/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd";
const FIRMWARE_VARS: &str = "/usr/share/AAVMF/AAVMF_VARS.fd";
const MIB: u64 = 1024 * 1024;
const OVERHEAD_MIB: u64 = 512;
const TRANSLATION_CACHE_MIB: u64 = 256;
const REQUIRED: [(&str, &str); 5] = [
    ("qemu-system-aarch64", "qemu-system-arm"),
    ("qemu-img", "qemu-utils"),
    ("passt", "passt"),
    (FIRMWARE_CODE, "qemu-efi-aarch64"),
    (FIRMWARE_VARS, "qemu-efi-aarch64"),
];
const DBUS_DISPLAY_PACKAGE: &str = "qemu-system-modules-opengl";
const SANDBOX: &str = "on,obsolete=deny,elevateprivileges=deny,spawn=deny,resourcecontrol=deny";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    Kvm,
    Tcg,
}

#[derive(Clone, Debug)]
pub struct Host {
    pub qemu: PathBuf,
    pub qemu_img: PathBuf,
    pub passt: PathBuf,
    pub firmware_code: PathBuf,
    pub firmware_vars: PathBuf,
    pub accel: Accel,
}

impl Accel {
    pub fn detect() -> Self {
        match OpenOptions::new().read(true).write(true).open("/dev/kvm") {
            Ok(_) => Self::Kvm,
            Err(_) => Self::Tcg,
        }
    }
}

fn has_dbus_display() -> bool {
    let architecture = std::env::consts::ARCH;
    Path::new(&format!(
        "/usr/lib/{architecture}-linux-gnu/qemu/ui-dbus.so"
    ))
    .is_file()
}

impl Host {
    pub fn detect() -> Result<Self> {
        let [qemu, qemu_img, passt, firmware_code, firmware_vars] =
            REQUIRED.map(|(path, package)| locate(path).ok_or(Error::Missing(package.into())));
        if !has_dbus_display() {
            return Err(Error::Missing(DBUS_DISPLAY_PACKAGE.into()));
        }
        Ok(Self {
            qemu: qemu?,
            qemu_img: qemu_img?,
            passt: passt?,
            firmware_code: firmware_code?,
            firmware_vars: firmware_vars?,
            accel: Accel::detect(),
        })
    }

    pub fn missing_packages() -> Vec<&'static str> {
        let mut missing: Vec<&'static str> = REQUIRED
            .iter()
            .filter(|(path, _)| locate(path).is_none())
            .map(|(_, package)| *package)
            .collect();
        if !has_dbus_display() {
            missing.push(DBUS_DISPLAY_PACKAGE);
        }
        missing.dedup();
        missing
    }

    pub fn memory_max(&self, machine: &Machine) -> u64 {
        let overhead = match self.accel {
            Accel::Kvm => OVERHEAD_MIB,
            Accel::Tcg => OVERHEAD_MIB + TRANSLATION_CACHE_MIB,
        };
        (machine.manifest.memory_mib + overhead) * MIB
    }

    pub fn qemu_command(&self, machine: &Machine) -> Result<Vec<String>> {
        let manifest = &machine.manifest;
        let (accel, cpu) = match self.accel {
            Accel::Kvm => ("kvm".to_owned(), "host"),
            Accel::Tcg => (
                format!("tcg,thread=multi,tb-size={TRANSLATION_CACHE_MIB}"),
                "max,pauth-impdef=on",
            ),
        };
        let file = |node: &str, path: &Path, read_only: bool| -> Result<String> {
            Ok(json!({
                "driver": "file",
                "node-name": format!("{node}-file"),
                "filename": utf8(path)?,
                "read-only": read_only,
                "aio": "io_uring",
                "cache": {"direct": true},
                "discard": "unmap",
            })
            .to_string())
        };
        let format = |node: &str, driver: &str, read_only: bool| {
            json!({
                "driver": driver,
                "node-name": node,
                "file": format!("{node}-file"),
                "read-only": read_only,
                "discard": "unmap",
            })
            .to_string()
        };
        let socket = |id: &str, path: &Path| -> Result<String> {
            Ok(format!(
                "socket,id={id},path={},server=on,wait=off",
                keyval(path)?
            ))
        };
        let mut argv = vec![utf8(&self.qemu)?.to_owned()];
        let mut push = |options: &[&str]| argv.extend(options.iter().map(|&o| o.to_owned()));
        push(&[
            "-name",
            &format!("guest={},debug-threads=on", keyval_str(&manifest.name)),
        ]);
        push(&["-uuid", &manifest.id, "-nodefaults", "-no-user-config"]);
        push(&[
            "-machine",
            "virt,gic-version=3,pflash0=firmware,pflash1=vars",
        ]);
        push(&["-accel", &accel, "-cpu", cpu]);
        push(&["-smp", &manifest.cpus.to_string()]);
        push(&["-m", &format!("{}M", manifest.memory_mib)]);
        push(&["-blockdev", &file("firmware", &self.firmware_code, true)?]);
        push(&["-blockdev", &format("firmware", "raw", true)]);
        push(&["-blockdev", &file("vars", &machine.vars(), false)?]);
        push(&["-blockdev", &format("vars", "qcow2", false)]);
        push(&["-blockdev", &file("disk", &machine.disk(), false)?]);
        push(&["-blockdev", &format("disk", "qcow2", false)]);
        push(&["-device", "virtio-blk-pci,drive=disk,bootindex=1"]);
        push(&["-blockdev", &file("seed", &machine.seed(), true)?]);
        push(&["-blockdev", &format("seed", "raw", true)]);
        push(&["-device", "virtio-blk-pci,drive=seed"]);
        push(&[
            "-netdev",
            &format!(
                "stream,id=net,server=off,addr.type=unix,addr.path={}",
                keyval(&machine.passt_socket())?
            ),
        ]);
        push(&[
            "-device",
            &format!("virtio-net-pci,netdev=net,mac={}", manifest.mac),
        ]);
        push(&["-device", "virtio-rng-pci", "-device", "virtio-serial-pci"]);
        push(&["-chardev", &socket("agent", &machine.agent_socket())?]);
        push(&[
            "-device",
            "virtserialport,id=agent-port,chardev=agent,name=org.qemu.guest_agent.0",
        ]);
        push(&["-chardev", &socket("qmp", &machine.qmp_socket())?]);
        push(&["-object", "monitor-qmp,id=qmp-monitor,chardev=qmp"]);
        push(&["-chardev", &socket("control", &machine.control_socket())?]);
        push(&["-object", "monitor-qmp,id=control-monitor,chardev=control"]);
        push(&[
            "-chardev",
            &format!(
                "{},logfile={},logappend=off",
                socket("serial", &machine.serial_socket())?,
                keyval(&machine.console_log())?
            ),
        ]);
        push(&["-serial", "chardev:serial"]);
        push(&["-device", "virtio-gpu-pci"]);
        push(&["-device", "qemu-xhci,id=usb"]);
        push(&["-device", "usb-kbd,bus=usb.0"]);
        push(&["-device", "usb-tablet,bus=usb.0"]);
        push(&[
            "-chardev",
            "qemu-vdagent,id=vdagent,name=vdagent,clipboard=on,mouse=off",
        ]);
        push(&[
            "-device",
            "virtserialport,chardev=vdagent,name=com.redhat.spice.0",
        ]);
        push(&["-display", "dbus,p2p=yes,gl=off"]);
        if manifest.resume_from.is_some() {
            push(&["-S"]);
        }
        push(&["-sandbox", SANDBOX]);
        Ok(argv)
    }

    pub fn passt_command(&self, machine: &Machine) -> Result<Vec<String>> {
        let mut argv = vec![
            utf8(&self.passt)?.to_owned(),
            "--quiet".into(),
            "--one-off".into(),
            "--pid".into(),
            utf8(&machine.passt_pid_file())?.to_owned(),
            "--socket".into(),
            utf8(&machine.passt_socket())?.to_owned(),
        ];
        for forward in &machine.manifest.forwards {
            argv.push("--tcp-ports".into());
            argv.push(format!("127.0.0.1/{}:{}", forward.host, forward.guest));
        }
        Ok(argv)
    }

    pub(crate) fn create_overlay(&self, base: &Path, overlay: &Path, size_gib: u64) -> Result<()> {
        self.qemu_img(&[
            "create",
            "-q",
            "-f",
            "qcow2",
            "-F",
            "qcow2",
            "-b",
            utf8(base)?,
            utf8(overlay)?,
            &format!("{size_gib}G"),
        ])
        .map(drop)
    }

    pub(crate) fn create_vars(&self, vars: &Path) -> Result<()> {
        self.qemu_img(&[
            "convert",
            "-q",
            "-f",
            "raw",
            "-O",
            "qcow2",
            utf8(&self.firmware_vars)?,
            utf8(vars)?,
        ])
        .map(drop)
    }

    pub(crate) fn snapshot_image(&self, action: &str, tag: &str, image: &str) -> Result<()> {
        self.qemu_img(&["snapshot", action, tag, image]).map(drop)
    }

    pub(crate) fn copy_image(
        &self,
        source: &Path,
        target: &Path,
        snapshot: Option<&str>,
        base: Option<&Path>,
    ) -> Result<()> {
        let at_snapshot = snapshot.map(|tag| format!("snapshot.name={tag}"));
        let mut arguments = vec!["convert", "-q", "-O", "qcow2"];
        if let Some(at_snapshot) = &at_snapshot {
            arguments.extend(["-l", at_snapshot]);
        }
        if let Some(base) = base.filter(|base| !base.as_os_str().is_empty()) {
            arguments.extend(["-B", utf8(base)?, "-F", "qcow2"]);
        }
        arguments.extend([utf8(source)?, utf8(target)?]);
        self.qemu_img(&arguments).map(drop)
    }

    pub(crate) fn backing_of(&self, image: &Path) -> Result<Option<PathBuf>> {
        let info = self.image_info(image)?;
        Ok(info["full-backing-filename"].as_str().map(PathBuf::from))
    }

    pub(crate) fn state_size(&self, image: &Path, tag: &str) -> Result<u64> {
        let info = self.image_info(image)?;
        Ok(info["snapshots"]
            .as_array()
            .and_then(|snapshots| snapshots.iter().find(|snapshot| snapshot["name"] == tag))
            .and_then(|snapshot| snapshot["vm-state-size"].as_u64())
            .unwrap_or_default())
    }

    fn image_info(&self, image: &Path) -> Result<Value> {
        let output = self.qemu_img(&["info", "-U", "--output=json", utf8(image)?])?;
        Ok(serde_json::from_slice(&output)?)
    }

    fn qemu_img(&self, arguments: &[&str]) -> Result<Vec<u8>> {
        let output = Command::new(&self.qemu_img)
            .args(arguments)
            .stdin(Stdio::null())
            .output()?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        Err(Error::Tool {
            program: "qemu-img".into(),
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn locate(path: &str) -> Option<PathBuf> {
    match path.starts_with('/') {
        true => Some(PathBuf::from(path)).filter(|path| path.is_file()),
        false => find_executable(path),
    }
}

pub(crate) fn utf8(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| Error::NotUtf8(path.to_owned()))
}

fn keyval(path: &Path) -> Result<String> {
    Ok(keyval_str(utf8(path)?))
}

fn keyval_str(value: &str) -> String {
    value.replace(',', ",,")
}
