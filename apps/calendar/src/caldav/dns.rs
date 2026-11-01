use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

const RESOLV_CONF: &str = "/etc/resolv.conf";
const SRV: u16 = 33;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

pub fn query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = Vec::with_capacity(32 + name.len());
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        packet.push(label.len().min(63) as u8);
        packet.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    packet.push(0);
    packet.extend_from_slice(&SRV.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet
}

fn read_u16(packet: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*packet.get(at)?, *packet.get(at + 1)?]))
}

fn read_name(packet: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let length = *packet.get(at)?;
        match length {
            0 => {
                return Some((labels.join("."), end.unwrap_or(at + 1)));
            }
            pointer if pointer & 0xC0 == 0xC0 => {
                let target = usize::from(read_u16(packet, at)? & 0x3FFF);
                end.get_or_insert(at + 2);
                at = target;
            }
            length => {
                let label = packet.get(at + 1..at + 1 + usize::from(length))?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + usize::from(length);
            }
        }
    }
    None
}

pub fn parse_answer(packet: &[u8], id: u16) -> Option<Vec<Service>> {
    if read_u16(packet, 0)? != id || packet.get(3)? & 0x0F != 0 {
        return None;
    }
    let questions = read_u16(packet, 4)?;
    let answers = read_u16(packet, 6)?;
    let mut at = 12;
    for _ in 0..questions {
        at = read_name(packet, at)?.1 + 4;
    }
    let mut services = Vec::new();
    for _ in 0..answers {
        at = read_name(packet, at)?.1;
        let kind = read_u16(packet, at)?;
        let length = usize::from(read_u16(packet, at + 8)?);
        let data = at + 10;
        if kind == SRV {
            services.push(Service {
                priority: read_u16(packet, data)?,
                weight: read_u16(packet, data + 2)?,
                port: read_u16(packet, data + 4)?,
                target: read_name(packet, data + 6)?.0,
            });
        }
        at = data + length;
    }
    services.sort_by_key(|service| (service.priority, u16::MAX - service.weight));
    Some(services)
}

fn nameservers() -> Vec<IpAddr> {
    std::fs::read_to_string(RESOLV_CONF)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|address| address.trim().split('%').next()?.parse().ok())
        .collect()
}

pub fn lookup(name: &str) -> Vec<Service> {
    let id = (std::process::id() as u16) ^ 0x5A5A;
    let packet = query(id, name);
    for server in nameservers() {
        let local: SocketAddr = match server {
            IpAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
            IpAddr::V6(_) => (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
        };
        let Ok(socket) = UdpSocket::bind(local) else {
            continue;
        };
        if socket.set_read_timeout(Some(TIMEOUT)).is_err()
            || socket.send_to(&packet, (server, 53)).is_err()
        {
            continue;
        }
        let mut answer = [0u8; 1500];
        if let Ok(length) = socket.recv(&mut answer)
            && let Some(services) = parse_answer(&answer[..length], id)
        {
            return services
                .into_iter()
                .filter(|service| service.target != "." && !service.target.is_empty())
                .collect();
        }
    }
    Vec::new()
}
