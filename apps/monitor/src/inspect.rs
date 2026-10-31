use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;

use crate::L10N;
use crate::proc::{parse_kilobytes, parse_stat};

#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub tid: u32,
    pub name: String,
    pub state: char,
    pub cpu_seconds: f64,
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
}

pub fn state_name(state: char) -> String {
    L10N.tr(match state {
        'R' => "state-running",
        'S' => "state-sleeping",
        'D' => "state-waiting",
        'Z' => "state-zombie",
        'T' | 't' => "state-stopped",
        'I' => "state-idle",
        _ => "state-unknown",
    })
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
            Some(Thread {
                tid,
                name: stat.comm,
                state: stat.state,
                cpu_seconds: stat.cpu_ticks as f64 / ticks_per_second,
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
                L10N.tr_args(
                    "port-listening",
                    [
                        ("protocol", protocol.into()),
                        ("local", local.to_string().into()),
                    ],
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| format!("{:08X}", u32::from_ne_bytes(*word)))
            .collect()
    }

    #[test]
    fn sockets_read_as_listening_or_connected_ports_with_short_ipv6() {
        let loopback = hex(&[127, 0, 0, 1]);
        let local = hex(&[10, 0, 0, 2]);
        let remote = hex(&[93, 184, 216, 34]);
        let header = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";
        let tcp = format!(
            "{header}   0: {loopback}:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1\n   1: {local}:C8FA {remote}:01BB 01 00000000:00000000 00:00000000 00000000  1000        0 222 1\n"
        );
        let mut six = [0u8; 16];
        six[15] = 1;
        let udp6 = format!(
            "{header}   0: {}:14E9 {}:0000 07 00000000:00000000 00:00000000 00000000  1000        0 333 1\n",
            hex(&six),
            hex(&[0; 16])
        );

        assert_eq!(
            parse_sockets(&tcp, "TCP"),
            [
                (111, "TCP 127.0.0.1:8080 (listening)".to_owned()),
                (222, "TCP 10.0.0.2:51450 → 93.184.216.34:443".to_owned()),
            ]
        );
        assert_eq!(
            parse_sockets(&udp6, "UDP"),
            [(333, "UDP [::1]:5353".to_owned())]
        );
    }

    #[test]
    fn mappings_keep_paths_with_spaces_and_anonymous_regions() {
        let maps = parse_maps(
            "aaaadf160000-aaaadf16c000 r-xp 00000000 fe:03 2622544    /usr/bin/my tool\n\
             ffff80000000-ffff80021000 rw-p 00000000 00:00 0 \n",
        );
        assert_eq!(
            maps,
            [
                Mapping {
                    start: 0xaaaa_df16_0000,
                    end: 0xaaaa_df16_c000,
                    permissions: "r-xp".to_owned(),
                    path: "/usr/bin/my tool".to_owned(),
                },
                Mapping {
                    start: 0xffff_8000_0000,
                    end: 0xffff_8002_1000,
                    permissions: "rw-p".to_owned(),
                    path: String::new(),
                },
            ]
        );
    }
}
