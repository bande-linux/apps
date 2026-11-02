use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use rustix::net::netlink::{self, SocketAddrNetlink};
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketType, recv, sendto, socket};

const SOCK_DIAG_BY_FAMILY: u16 = 20;
const NLM_F_REQUEST: u16 = 1;
const NLM_F_DUMP: u16 = 0x300;
const NLMSG_DONE: u16 = 3;
const NLMSG_ERROR: u16 = 2;
const HEADER: usize = 16;
const DIAG_REQUEST: usize = 56;
const DIAG_MESSAGE: usize = 72;
const INET_DIAG_INFO: u16 = 2;
const IPPROTO_TCP: u8 = 6;
const ALL_STATES: u32 = u32::MAX;
const BYTES_ACKED: usize = 120;
const BYTES_RECEIVED: usize = 128;
const INODE: usize = 68;
const BUFFER: usize = 65536;
const SOCKET_LINK: &str = "socket:[";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Counted {
    pub(crate) sent: u64,
    pub(crate) received: u64,
}

fn request(family: AddressFamily) -> Vec<u8> {
    let length = (HEADER + DIAG_REQUEST) as u32;
    let mut message = Vec::with_capacity(length as usize);
    message.extend(length.to_ne_bytes());
    message.extend(SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    message.extend((NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    message.extend(1u32.to_ne_bytes());
    message.extend(0u32.to_ne_bytes());
    message.push(family.as_raw() as u8);
    message.push(IPPROTO_TCP);
    message.push(1 << (INET_DIAG_INFO - 1));
    message.push(0);
    message.extend(ALL_STATES.to_ne_bytes());
    message.resize(length as usize, 0);
    message
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn aligned(length: usize) -> usize {
    length.div_ceil(4) * 4
}

fn counted(diag: &[u8]) -> Option<(u64, Counted)> {
    let inode = u64::from(u32_at(diag, INODE)?);
    let mut at = DIAG_MESSAGE;
    while at + 4 <= diag.len() {
        let length = usize::from(u16_at(diag, at)?);
        let kind = u16_at(diag, at + 2)?;
        if length < 4 {
            return None;
        }
        if kind == INET_DIAG_INFO {
            let info = diag.get(at + 4..at + length)?;
            return Some((
                inode,
                Counted {
                    sent: u64_at(info, BYTES_ACKED)?,
                    received: u64_at(info, BYTES_RECEIVED)?,
                },
            ));
        }
        at += aligned(length);
    }
    None
}

fn parse(buffer: &[u8], counts: &mut HashMap<u64, Counted>) -> Option<bool> {
    let mut at = 0;
    while at + HEADER <= buffer.len() {
        let length = u32_at(buffer, at)? as usize;
        let kind = u16_at(buffer, at + 4)?;
        if length < HEADER || at + length > buffer.len() {
            return None;
        }
        match kind {
            NLMSG_DONE | NLMSG_ERROR => return Some(true),
            SOCK_DIAG_BY_FAMILY => {
                if let Some((inode, count)) = counted(&buffer[at + HEADER..at + length]) {
                    counts.insert(inode, count);
                }
            }
            _ => {}
        }
        at += aligned(length);
    }
    Some(false)
}

pub(crate) fn tcp_counts() -> io::Result<HashMap<u64, Counted>> {
    let diag = socket(
        AddressFamily::NETLINK,
        SocketType::DGRAM,
        Some(netlink::SOCK_DIAG),
    )?;
    let kernel = SocketAddrNetlink::new(0, 0);
    let mut counts = HashMap::new();
    let mut buffer = vec![0; BUFFER];
    for family in [AddressFamily::INET, AddressFamily::INET6] {
        sendto(&diag, &request(family), SendFlags::empty(), &kernel)?;
        loop {
            let (read, _) = recv(&diag, &mut buffer, RecvFlags::empty())?;
            match parse(&buffer[..read], &mut counts) {
                Some(false) if read > 0 => continue,
                _ => break,
            }
        }
    }
    Ok(counts)
}

pub(crate) fn socket_inodes(folder: &Path) -> Vec<u64> {
    fs::read_dir(folder.join("fd"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let target = fs::read_link(entry.path()).ok()?;
            target
                .to_str()?
                .strip_prefix(SOCKET_LINK)?
                .strip_suffix(']')?
                .parse()
                .ok()
        })
        .collect()
}
