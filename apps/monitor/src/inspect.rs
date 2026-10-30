use std::fs;
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

fn read_files(folder: &Path) -> Option<Vec<OpenFile>> {
    let mut files = fs::read_dir(folder.join("fd"))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            Some(OpenFile {
                fd: entry.file_name().to_str()?.parse().ok()?,
                target: fs::read_link(entry.path())
                    .ok()?
                    .to_string_lossy()
                    .into_owned(),
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
