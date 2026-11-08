use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, IoSlice, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

const USB_DEVICES: &str = "/sys/bus/usb/devices";
const DRIVERS_PROBE: &str = "/sys/bus/usb/drivers_probe";
const NET: &str = "/sys/class/net";
const TUN: &str = "/dev/net/tun";
const HUB_CLASS: &str = "09";
const HELD: &str = "held";
const TAP_PREFIX: &str = "vx";
const IFF_TAP: libc::c_short = 0x0002;
const IFF_NO_PI: libc::c_short = 0x1000;
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
const TUNSETPERSIST: libc::c_ulong = 0x4004_54cb;
const TUNSETOWNER: libc::c_ulong = 0x4004_54cc;
const SIOCBRADDIF: libc::c_ulong = 0x89a2;
const SIOCBRDELIF: libc::c_ulong = 0x89a3;

fn refuse(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message.into())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message.into())
}

fn caller() -> io::Result<u32> {
    if !rustix::process::geteuid().is_root() {
        return Err(refuse("the helper runs only through pkexec"));
    }
    std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|uid| uid.parse().ok())
        .filter(|uid| *uid != 0)
        .ok_or_else(|| refuse("the helper runs only through pkexec for a user"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

fn number<T: std::str::FromStr>(text: &str) -> io::Result<T> {
    text.parse()
        .map_err(|_| invalid(format!("{text} is not a number")))
}

fn usb_device(bus: u8, address: u8) -> io::Result<PathBuf> {
    for entry in fs::read_dir(USB_DEVICES)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.contains(':') || name.starts_with("usb") {
            continue;
        }
        if read(&path.join("busnum")) == bus.to_string()
            && read(&path.join("devnum")) == address.to_string()
        {
            return Ok(path);
        }
    }
    Err(invalid(format!("no USB device {bus}-{address}")))
}

fn interfaces(device: &Path) -> io::Result<Vec<PathBuf>> {
    let name = device
        .file_name()
        .map(|name| format!("{}:", name.to_string_lossy()))
        .unwrap_or_default();
    Ok(fs::read_dir(device)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|file| file.to_string_lossy().starts_with(&name))
        })
        .collect())
}

fn block_devices(folder: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if path.parent().and_then(Path::file_name) == Some("block".as_ref())
            || path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                == Some("block".as_ref())
        {
            let numbers = read(&path.join("dev"));
            if !numbers.is_empty() {
                found.push(numbers);
            }
            let held = fs::read_dir(path.join("holders"))
                .map(|mut holders| holders.next().is_some())
                .unwrap_or(false);
            if held {
                found.push(HELD.to_owned());
            }
        }
        block_devices(&path, found);
    }
}

fn check_usb(device: &Path) -> io::Result<()> {
    if read(&device.join("bDeviceClass")) == HUB_CLASS {
        return Err(refuse("a USB hub cannot be passed to a virtual machine"));
    }
    for interface in interfaces(device)? {
        if read(&interface.join("bInterfaceClass")) == HUB_CLASS {
            return Err(refuse("a USB hub cannot be passed to a virtual machine"));
        }
    }
    let mut blocks = Vec::new();
    block_devices(device, &mut blocks);
    if blocks.iter().any(|block| block == HELD) {
        return Err(refuse("the device is in use by this computer"));
    }
    let mounts = fs::read_to_string("/proc/self/mountinfo")?;
    let mounted = mounts
        .lines()
        .filter_map(|line| line.split(' ').nth(2))
        .any(|numbers| blocks.iter().any(|block| block == numbers));
    if mounted {
        return Err(refuse("the device holds a mounted file system"));
    }
    Ok(())
}

fn owner_of(pid: i32) -> io::Result<u32> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_whitespace().next())
        .and_then(|uid| uid.parse().ok())
        .ok_or_else(|| invalid(format!("process {pid} has no owner")))
}

fn send_fd(fd: BorrowedFd<'_>) -> io::Result<()> {
    let stdout = unsafe { BorrowedFd::borrow_raw(libc::STDOUT_FILENO) };
    if !rustix::fs::fstat(stdout)
        .map(|stat| rustix::fs::FileType::from_raw_mode(stat.st_mode).is_socket())
        .unwrap_or(false)
    {
        return Err(invalid("the helper's output must be a socket"));
    }
    let fds = [fd];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    control.push(SendAncillaryMessage::ScmRights(&fds));
    sendmsg(
        stdout,
        &[IoSlice::new(b"1")],
        &mut control,
        SendFlags::empty(),
    )?;
    Ok(())
}

fn give_back(device: &Path, address: &str) {
    if read(&device.join("devnum")) != address {
        return;
    }
    let Ok(interfaces) = interfaces(device) else {
        return;
    };
    for interface in interfaces {
        if interface.join("driver").exists() {
            continue;
        }
        let Some(name) = interface.file_name() else {
            continue;
        };
        if let Ok(mut probe) = OpenOptions::new().write(true).open(DRIVERS_PROBE) {
            let _ = probe.write_all(name.to_string_lossy().as_bytes());
        }
    }
}

fn detach_stdio() {
    if let Ok(null) = File::options().read(true).write(true).open("/dev/null") {
        for fd in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            unsafe { libc::dup2(null.as_raw_fd(), fd) };
        }
    }
}

fn usb(uid: u32, arguments: &[String]) -> io::Result<()> {
    let [bus, address, pid] = arguments else {
        return Err(invalid("usage: usb <bus> <address> <qemu pid>"));
    };
    let bus: u8 = number(bus)?;
    let address: u8 = number(address)?;
    let pid: i32 = number(pid)?;
    let process = Pid::from_raw(pid).ok_or_else(|| invalid("no such process"))?;
    let pidfd = pidfd_open(process, PidfdFlags::empty())?;
    if owner_of(pid)? != uid {
        return Err(refuse("the virtual machine belongs to another user"));
    }
    let device = usb_device(bus, address)?;
    check_usb(&device)?;
    let node = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(format!("/dev/bus/usb/{bus:03}/{address:03}"))?;
    send_fd(node.as_fd())?;
    drop(node);
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => {
            unsafe { libc::setsid() };
            detach_stdio();
            wait_for_exit(&pidfd);
            give_back(&device, &address.to_string());
            std::process::exit(0)
        }
        _ => Ok(()),
    }
}

fn wait_for_exit(pidfd: &OwnedFd) {
    loop {
        let mut fds = [PollFd::new(pidfd, PollFlags::IN)];
        match poll(&mut fds, None) {
            Ok(_) => return,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return,
        }
    }
}

fn interface_name(name: &str) -> io::Result<[libc::c_char; libc::IFNAMSIZ]> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() >= libc::IFNAMSIZ {
        return Err(invalid(format!("{name} is not an interface name")));
    }
    let mut out = [0; libc::IFNAMSIZ];
    for (slot, byte) in out.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    Ok(out)
}

fn request(name: &str) -> io::Result<libc::ifreq> {
    let mut ifreq: libc::ifreq = unsafe { std::mem::zeroed() };
    ifreq.ifr_name = interface_name(name)?;
    Ok(ifreq)
}

fn ioctl(fd: BorrowedFd<'_>, op: libc::c_ulong, argument: *mut libc::c_void) -> io::Result<()> {
    match unsafe { libc::ioctl(fd.as_raw_fd(), op as _, argument) } {
        -1 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

fn ioctl_value(fd: BorrowedFd<'_>, op: libc::c_ulong, value: libc::c_ulong) -> io::Result<()> {
    match unsafe { libc::ioctl(fd.as_raw_fd(), op as _, value) } {
        -1 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

fn valid_tap(name: &str) -> bool {
    name.strip_prefix(TAP_PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() + TAP_PREFIX.len() < libc::IFNAMSIZ
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
    })
}

fn tap(uid: u32, arguments: &[String]) -> io::Result<()> {
    let [name, bridge] = arguments else {
        return Err(invalid("usage: tap <name> <bridge>"));
    };
    if !valid_tap(name) {
        return Err(invalid(format!("{name} is not a Velox network name")));
    }
    let bridge_path = Path::new(NET).join(bridge);
    if bridge.contains('/') || !bridge_path.join("bridge").is_dir() {
        return Err(invalid(format!("{bridge} is not a network bridge")));
    }
    let existing = Path::new(NET).join(name);
    if existing.exists()
        && (!existing.join("tun_flags").exists()
            || read(&existing.join("owner")) != uid.to_string())
    {
        return Err(refuse(format!("{name} belongs to someone else")));
    }
    let tun = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(TUN)?;
    let mut ifreq = request(name)?;
    ifreq.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;
    ioctl(tun.as_fd(), TUNSETIFF, (&raw mut ifreq).cast())?;
    ioctl_value(tun.as_fd(), TUNSETOWNER, libc::c_ulong::from(uid))?;
    ioctl_value(tun.as_fd(), TUNSETPERSIST, 1)?;
    drop(tun);
    let socket: OwnedFd = rustix::net::socket(
        rustix::net::AddressFamily::INET,
        rustix::net::SocketType::DGRAM,
        None,
    )?;
    let mut index = request(name)?;
    ioctl(socket.as_fd(), libc::SIOCGIFINDEX, (&raw mut index).cast())?;
    let tap_index = unsafe { index.ifr_ifru.ifru_ifindex };
    let master = fs::read_link(existing.join("master"))
        .ok()
        .and_then(|link| {
            link.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
    if master.as_deref() != Some(bridge.as_str()) {
        if let Some(old) = master {
            let mut leave = request(&old)?;
            leave.ifr_ifru.ifru_ifindex = tap_index;
            ioctl(socket.as_fd(), SIOCBRDELIF, (&raw mut leave).cast())?;
        }
        let mut join = request(bridge)?;
        join.ifr_ifru.ifru_ifindex = tap_index;
        ioctl(socket.as_fd(), SIOCBRADDIF, (&raw mut join).cast())?;
    }
    let mut flags = request(name)?;
    ioctl(socket.as_fd(), libc::SIOCGIFFLAGS, (&raw mut flags).cast())?;
    unsafe { flags.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short };
    ioctl(socket.as_fd(), libc::SIOCSIFFLAGS, (&raw mut flags).cast())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let done = caller().and_then(|uid| match arguments.split_first() {
        Some((action, rest)) if action == "usb" => usb(uid, rest),
        Some((action, rest)) if action == "tap" => tap(uid, rest),
        _ => Err(invalid("usage: velox-helper usb|tap …")),
    });
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "{error}");
            ExitCode::FAILURE
        }
    }
}
