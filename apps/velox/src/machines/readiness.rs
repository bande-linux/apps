use std::fs::{self, OpenOptions};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bande_vm::{Accel, Host};

use crate::readiness::listed_in_group;

const KVM: &str = "/dev/kvm";
const ACL_XATTR: &str = "system.posix_acl_access";
const ACL_USER: u16 = 2;
const ACL_READ_WRITE: u16 = 6;
pub const INSTALL: &str = "sudo apt install qemu-system-arm qemu-efi-aarch64 qemu-utils passt";
pub const KVM_GROUP: &str = "sudo usermod -aG kvm $USER";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    Missing(Vec<&'static str>),
    NoKvm,
    Relogin,
    NoManager,
}

pub fn probe(emulate: bool) -> Result<Host, Blocker> {
    let missing = Host::missing_packages();
    if !missing.is_empty() {
        return Err(Blocker::Missing(missing));
    }
    let mut host = Host::detect().map_err(|_| Blocker::Missing(Host::missing_packages()))?;
    host.accel = match kvm() {
        Ok(()) => Accel::Kvm,
        Err(_) if emulate => Accel::Tcg,
        Err(blocker) => return Err(blocker),
    };
    Ok(host)
}

fn kvm() -> Result<(), Blocker> {
    let device = Path::new(KVM);
    let Ok(metadata) = fs::metadata(device) else {
        return Err(Blocker::NoKvm);
    };
    let gid = metadata.gid();
    let opened = OpenOptions::new()
        .read(true)
        .write(true)
        .open(device)
        .is_ok();
    let manager = manager_groups();
    let by_user = granted_to_user(device) || metadata.uid() == uid();
    let manager_opens = by_user
        || manager
            .as_ref()
            .map_or(opened, |groups| groups.contains(&gid));
    match manager_opens {
        true => Ok(()),
        false if listed_in_group(gid) => Err(Blocker::Relogin),
        false => Err(Blocker::NoKvm),
    }
}

fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

fn granted_to_user(device: &Path) -> bool {
    let mut acl = [0; 256];
    let Ok(length) = rustix::fs::getxattr(device, ACL_XATTR, &mut acl) else {
        return false;
    };
    let uid = uid();
    acl.get(4..length)
        .unwrap_or_default()
        .as_chunks::<8>()
        .0
        .iter()
        .any(|entry| {
            let tag = u16::from_le_bytes([entry[0], entry[1]]);
            let permissions = u16::from_le_bytes([entry[2], entry[3]]);
            let id = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
            tag == ACL_USER && id == uid && permissions & ACL_READ_WRITE == ACL_READ_WRITE
        })
}

fn manager_groups() -> Option<Vec<u32>> {
    let uid = uid();
    let procs = fs::read_to_string(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/init.scope/cgroup.procs"
    ))
    .ok()?;
    let pid = procs.lines().next()?.trim().to_owned();
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let groups = status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:"))?;
    Some(
        groups
            .split_whitespace()
            .filter_map(|group| group.parse().ok())
            .collect(),
    )
}
