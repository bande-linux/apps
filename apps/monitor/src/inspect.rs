use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;


use bande_l10n::tr;

use crate::proc::{Stat, parse_kilobytes, parse_number, parse_stat};

#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub tid: u32,
    pub name: String,
    pub state: char,
    pub cpu_seconds: f64,
    pub waiting_in: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    pub fd: u32,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub start: u64,
    pub end: u64,
    pub permissions: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Details {
    pub state: char,
    pub threads: Vec<Thread>,
    pub files: Option<Vec<OpenFile>>,
    pub maps: Option<Vec<Mapping>>,
    pub virtual_size: Option<u64>,
    pub running_seconds: Option<f64>,
    pub statistics: Statistics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Statistics {
    pub minor_faults: u64,
    pub major_faults: u64,
    pub switches: Option<u64>,
    pub involuntary_switches: Option<u64>,
    pub read_calls: Option<u64>,
    pub write_calls: Option<u64>,
}

pub fn state_name(state: char) -> String {
    match state {
        'R' => tr!("Running"),
        'S' => tr!("Sleeping"),
        'D' => tr!("Waiting for the disk"),
        'Z' => tr!("Ended, waiting for its parent"),
        'T' | 't' => tr!("Stopped"),
        'I' => tr!("Idle"),
        _ => tr!("Unknown"),
    }
}

fn parse_maps(text: &str) -> Vec<Mapping> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (start, end) = fields.next()?.split_once('-')?;
            let permissions = fields.next()?.to_owned();
            let path = fields.nth(3).map_or_else(String::new, |first| {
                std::iter::once(first)
                    .chain(fields)
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            Some(Mapping {
                start: u64::from_str_radix(start, 16).ok()?,
                end: u64::from_str_radix(end, 16).ok()?,
                permissions,
                path,
            })
        })
        .collect()
}

fn read_threads(folder: &Path, ticks_per_second: f64) -> Vec<Thread> {
    let mut threads = fs::read_dir(folder.join("task"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let tid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let stat = parse_stat(&fs::read_to_string(entry.path().join("stat")).ok()?)?;
            let waiting_in = fs::read_to_string(entry.path().join("wchan"))
                .ok()
                .map(|symbol| symbol.trim().to_owned())
                .filter(|symbol| !symbol.is_empty() && symbol != "0");
            Some(Thread {
                tid,
                name: stat.comm,
                state: stat.state,
                cpu_seconds: stat.cpu_ticks as f64 / ticks_per_second,
                waiting_in,
            })
        })
        .collect::<Vec<_>>();
    threads.sort_by_key(|thread| thread.tid);
    threads
}

const TCP_LISTEN: &str = "0A";

fn parse_socket_address(text: &str) -> Option<SocketAddr> {
    let (address, port) = text.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let words = (0..address.len() / 8)
        .map(|index| u32::from_str_radix(address.get(index * 8..index * 8 + 8)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let ip = match words.as_slice() {
        [word] => IpAddr::V4(Ipv4Addr::from(word.to_ne_bytes())),
        [a, b, c, d] => {
            let mut bytes = [0; 16];
            for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip([a, b, c, d]) {
                *chunk = word.to_ne_bytes();
            }
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

fn parse_sockets(text: &str, protocol: &str) -> Vec<(u64, String)> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let local = parse_socket_address(fields.get(1)?)?;
            let remote = parse_socket_address(fields.get(2)?)?;
            let inode = fields.get(9)?.parse::<u64>().ok()?;
            let description = if protocol == "TCP" && *fields.get(3)? == TCP_LISTEN {
                tr!(
                    "{protocol} {local} (listening)",
                    protocol = protocol,
                    local = local.to_string()
                )
            } else if remote.port() == 0 {
                format!("{protocol} {local}")
            } else {
                format!("{protocol} {local} → {remote}")
            };
            Some((inode, description))
        })
        .collect()
}

fn read_sockets(folder: &Path) -> HashMap<String, String> {
    [
        ("tcp", "TCP"),
        ("tcp6", "TCP"),
        ("udp", "UDP"),
        ("udp6", "UDP"),
    ]
    .into_iter()
    .flat_map(|(file, protocol)| {
        fs::read_to_string(folder.join("net").join(file))
            .map(|text| parse_sockets(&text, protocol))
            .unwrap_or_default()
    })
    .map(|(inode, description)| (format!("socket:[{inode}]"), description))
    .collect()
}

fn read_files(folder: &Path) -> Option<Vec<OpenFile>> {
    let mut sockets: Option<HashMap<String, String>> = None;
    let mut files = fs::read_dir(folder.join("fd"))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let target = fs::read_link(entry.path())
                .ok()?
                .to_string_lossy()
                .into_owned();
            let target = match target.starts_with("socket:[") {
                true => sockets
                    .get_or_insert_with(|| read_sockets(folder))
                    .get(&target)
                    .cloned()
                    .unwrap_or(target),
                false => target,
            };
            Some(OpenFile {
                fd: entry.file_name().to_str()?.parse().ok()?,
                target,
            })
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|file| file.fd);
    Some(files)
}

fn read_statistics(folder: &Path, stat: &Stat) -> Statistics {
    let statuses = fs::read_dir(folder.join("task"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|task| fs::read_to_string(task.path().join("status")).ok())
        .collect::<Vec<_>>();
    let summed = |key: &str| {
        statuses
            .iter()
            .map(|status| parse_number(status, key))
            .sum::<Option<u64>>()
    };
    let io = fs::read_to_string(folder.join("io")).ok();
    let calls = |key: &str| io.as_deref().and_then(|io| parse_number(io, key));
    Statistics {
        minor_faults: stat.minor_faults,
        major_faults: stat.major_faults,
        switches: summed("voluntary_ctxt_switches"),
        involuntary_switches: summed("nonvoluntary_ctxt_switches"),
        read_calls: calls("syscr"),
        write_calls: calls("syscw"),
    }
}

pub fn read_details(root: &Path, pid: u32, ticks_per_second: f64) -> Option<Details> {
    let folder = root.join("proc").join(pid.to_string());
    let stat = parse_stat(&fs::read_to_string(folder.join("stat")).ok()?)?;
    let uptime = fs::read_to_string(root.join("proc/uptime"))
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok());
    Some(Details {
        state: stat.state,
        threads: read_threads(&folder, ticks_per_second),
        files: read_files(&folder),
        maps: fs::read_to_string(folder.join("maps"))
            .ok()
            .map(|text| parse_maps(&text)),
        virtual_size: fs::read_to_string(folder.join("status"))
            .ok()
            .and_then(|status| parse_kilobytes(&status, "VmSize")),
        running_seconds: uptime
            .map(|uptime| (uptime - stat.start_ticks as f64 / ticks_per_second).max(0.0)),
        statistics: read_statistics(&folder, &stat),
    })
}
