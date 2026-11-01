use std::hash::{BuildHasher, RandomState};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);
const SRV: u16 = 33;
const MX: u16 = 15;
const MAX_POINTERS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub host: String,
}

fn nameservers(resolv_conf: &str) -> Vec<IpAddr> {
    resolv_conf
        .lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|address| address.trim().split('%').next()?.parse().ok())
        .collect()
}

fn query(id: u16, name: &str, kind: u16) -> Option<Vec<u8>> {
    let mut packet = Vec::with_capacity(64);
    packet.extend(id.to_be_bytes());
    packet.extend([0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        let length = u8::try_from(label.len())
            .ok()
            .filter(|length| (1..64).contains(length))?;
        packet.push(length);
        packet.extend(label.as_bytes());
    }
    packet.push(0);
    packet.extend(kind.to_be_bytes());
    packet.extend(1u16.to_be_bytes());
    Some(packet)
}

fn read_u16(packet: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*packet.get(at)?, *packet.get(at + 1)?]))
}

fn read_name(packet: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..MAX_POINTERS {
        let length = *packet.get(at)?;
        match length {
            0 => {
                return Some((labels.join("."), end.unwrap_or(at + 1)));
            }
            length if length & 0xc0 == 0xc0 => {
                end.get_or_insert(at + 2);
                at = usize::from(read_u16(packet, at)? & 0x3fff);
            }
            length => {
                let start = at + 1;
                let label = packet.get(start..start + usize::from(length))?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                at = start + usize::from(length);
            }
        }
    }
    None
}

fn answers(packet: &[u8], id: u16, kind: u16) -> Option<Vec<Target>> {
    if read_u16(packet, 0)? != id || packet.get(2)? & 0x80 == 0 {
        return None;
    }
    if packet.get(3)? & 0x0f != 0 {
        return Some(Vec::new());
    }
    let questions = read_u16(packet, 4)?;
    let count = read_u16(packet, 6)?;
    let mut at = 12;
    for _ in 0..questions {
        at = read_name(packet, at)?.1 + 4;
    }
    let mut targets = Vec::new();
    for _ in 0..count {
        at = read_name(packet, at)?.1;
        let record = read_u16(packet, at)?;
        let length = usize::from(read_u16(packet, at + 8)?);
        let data = at + 10;
        if record == kind && kind == SRV {
            targets.push(Target {
                priority: read_u16(packet, data)?,
                weight: read_u16(packet, data + 2)?,
                port: read_u16(packet, data + 4)?,
                host: read_name(packet, data + 6)?.0,
            });
        } else if record == kind && kind == MX {
            targets.push(Target {
                priority: read_u16(packet, data)?,
                weight: 0,
                port: 25,
                host: read_name(packet, data + 2)?.0,
            });
        }
        at = data + length;
    }
    targets.sort_by_key(|target| (target.priority, u16::MAX - target.weight));
    targets.retain(|target| !target.host.is_empty());
    Some(targets)
}

fn resolve(name: &str, kind: u16) -> Vec<Target> {
    let servers = std::fs::read_to_string("/etc/resolv.conf")
        .map(|text| nameservers(&text))
        .unwrap_or_default();
    let id = RandomState::new().hash_one(name) as u16;
    let Some(packet) = query(id, name, kind) else {
        return Vec::new();
    };
    for server in servers.into_iter().take(3) {
        let local: SocketAddr = match server {
            IpAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
            IpAddr::V6(_) => ([0u16; 8], 0).into(),
        };
        let Ok(socket) = UdpSocket::bind(local) else {
            continue;
        };
        if socket.set_read_timeout(Some(TIMEOUT)).is_err()
            || socket.send_to(&packet, (server, 53)).is_err()
        {
            continue;
        }
        let mut answer = [0u8; 4096];
        while let Ok((length, from)) = socket.recv_from(&mut answer) {
            if from.ip() != server {
                continue;
            }
            if let Some(targets) = answers(&answer[..length], id, kind) {
                return targets;
            }
        }
    }
    Vec::new()
}

pub fn services(name: &str) -> Vec<Target> {
    resolve(name, SRV)
}

pub fn exchangers(domain: &str) -> Vec<Target> {
    resolve(domain, MX)
}
