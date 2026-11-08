use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};

use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};
use serde_json::json;

use crate::qmp::Qmp;
use crate::{Error, Result};

pub const HELPER: &str = "/usr/libexec/bande-velox/velox-helper";
const PKEXEC: &str = "pkexec";
const DISMISSED: i32 = 126;
const USB_DEVICES: &str = "/sys/bus/usb/devices";
const NET: &str = "/sys/class/net";
const HUB_CLASS: &str = "09";
const USB_ID_PREFIX: &str = "usbpass-";
const PERIPHERALS: &str = "/machine/peripheral";
const TAP_PREFIX: &str = "vx";
const TAP_ID_LENGTH: usize = 8;
const IFF_TAP: libc::c_short = 0x0002;
const IFF_NO_PI: libc::c_short = 0x1000;
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
const TUNSETPERSIST: libc::c_ulong = 0x4004_54cb;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbDevice {
    pub bus: u8,
    pub address: u8,
    pub vendor: String,
    pub product: String,
    pub name: String,
}

impl UsbDevice {
    pub fn key(&self) -> String {
        format!("{}-{}", self.bus, self.address)
    }

    fn qdev_id(&self) -> String {
        format!("{USB_ID_PREFIX}{}", self.key())
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

fn usb_device(path: &Path) -> Option<UsbDevice> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if name.contains(':')
        || name.starts_with("usb")
        || read(&path.join("bDeviceClass")) == HUB_CLASS
    {
        return None;
    }
    let vendor = read(&path.join("idVendor"));
    let product = read(&path.join("idProduct"));
    let label = [
        read(&path.join("manufacturer")),
        read(&path.join("product")),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    Some(UsbDevice {
        bus: read(&path.join("busnum")).parse().ok()?,
        address: read(&path.join("devnum")).parse().ok()?,
        name: match label.is_empty() {
            true => format!("{vendor}:{product}"),
            false => label,
        },
        vendor,
        product,
    })
}

pub fn usb_devices() -> Vec<UsbDevice> {
    let Ok(entries) = fs::read_dir(USB_DEVICES) else {
        return Vec::new();
    };
    let mut devices: Vec<UsbDevice> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| usb_device(&entry.path()))
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name).then(a.key().cmp(&b.key())));
    devices
}

fn run_helper(arguments: &[String]) -> Result<UnixStream> {
    let (ours, theirs) = UnixStream::pair()?;
    let output = Command::new(PKEXEC)
        .arg(HELPER)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => Error::Missing(PKEXEC.into()),
            _ => error.into(),
        })?;
    match output.status.code() {
        Some(0) => Ok(ours),
        Some(DISMISSED) => Err(Error::Cancelled),
        _ => Err(Error::Tool {
            program: HELPER.into(),
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
    }
}

fn receive_fd(stream: &UnixStream) -> Result<OwnedFd> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let mut byte = [0u8; 1];
    recvmsg(
        stream,
        &mut [IoSliceMut::new(&mut byte)],
        &mut control,
        RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
    )?;
    control
        .drain()
        .find_map(|message| match message {
            RecvAncillaryMessage::ScmRights(mut fds) => fds.next(),
            _ => None,
        })
        .ok_or_else(|| Error::Protocol("the helper sent no device".into()))
}

impl Qmp {
    pub fn attach_usb(&mut self, pid: i32, device: &UsbDevice) -> Result<()> {
        let stream = run_helper(&[
            "usb".into(),
            device.bus.to_string(),
            device.address.to_string(),
            pid.to_string(),
        ])?;
        let fd = receive_fd(&stream)?;
        let added = self.execute_with_fd(
            "add-fd",
            json!({"opaque": format!("USB {} {}", device.key(), device.name)}),
            fd.as_fd(),
        )?;
        drop(fd);
        let set = added["fdset-id"]
            .as_i64()
            .ok_or_else(|| Error::Protocol("add-fd returned no set".into()))?;
        let plugged = self.execute(
            "device_add",
            json!({
                "driver": "usb-host",
                "id": device.qdev_id(),
                "bus": "usb.0",
                "hostdevice": format!("/dev/fdset/{set}"),
            }),
        );
        self.execute("remove-fd", json!({"fdset-id": set}))?;
        plugged.map(|_| ())
    }

    pub fn detach_usb(&mut self, key: &str) -> Result<()> {
        self.execute("device_del", json!({"id": format!("{USB_ID_PREFIX}{key}")}))
            .map(|_| ())
    }

    pub fn attached_usb(&mut self) -> Result<Vec<String>> {
        let listed = self.execute("qom-list", json!({"path": PERIPHERALS}))?;
        Ok(listed
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|property| property["name"].as_str())
            .filter_map(|name| name.strip_prefix(USB_ID_PREFIX))
            .map(str::to_owned)
            .collect())
    }
}

pub fn tap_name(id: &str) -> String {
    let short: String = id
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(TAP_ID_LENGTH)
        .collect();
    format!("{TAP_PREFIX}{}", short.to_ascii_lowercase())
}

pub fn bridges() -> Vec<String> {
    let Ok(entries) = fs::read_dir(NET) else {
        return Vec::new();
    };
    let mut bridges: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("bridge").is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    bridges.sort();
    bridges
}

fn tap_ready(tap: &str, bridge: &str) -> bool {
    let path = Path::new(NET).join(tap);
    let uid = rustix::process::getuid().as_raw();
    read(&path.join("owner")) == uid.to_string()
        && fs::read_link(path.join("master"))
            .ok()
            .is_some_and(|master| master.file_name() == Some(bridge.as_ref()))
}

pub fn prepare_tap(tap: &str, bridge: &str) -> Result<()> {
    if tap_ready(tap, bridge) {
        return Ok(());
    }
    run_helper(&["tap".into(), tap.into(), bridge.into()]).map(|_| ())
}

pub fn release_tap(tap: &str) -> Result<()> {
    let path = Path::new(NET).join(tap);
    let uid = rustix::process::getuid().as_raw();
    if read(&path.join("owner")) != uid.to_string() {
        return Ok(());
    }
    let bytes = tap.as_bytes();
    if bytes.len() >= libc::IFNAMSIZ {
        return Err(Error::Protocol(format!("{tap} is too long")));
    }
    let tun = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/dev/net/tun")?;
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    request.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;
    if unsafe { libc::ioctl(tun.as_raw_fd(), TUNSETIFF as _, &raw mut request) } == -1
        || unsafe { libc::ioctl(tun.as_raw_fd(), TUNSETPERSIST as _, 0 as libc::c_ulong) } == -1
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
