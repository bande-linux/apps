use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const TERMINAL_GROUP_FIELD: usize = 5;
const MAX_REPORT: usize = 4096;
const BELL: u8 = 0x07;
const ESCAPE: u8 = 0x1b;

pub fn foreground_group(shell: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{shell}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    let group: i64 = fields
        .split_whitespace()
        .nth(TERMINAL_GROUP_FIELD)?
        .parse()
        .ok()?;
    u32::try_from(group).ok().filter(|group| *group > 0)
}

pub fn name(pid: u32) -> Option<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_owned())
}

pub fn working_directory(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

pub fn host_name() -> Option<String> {
    let name = fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    Some(name.trim().to_owned())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    Introducer(u8),
    Report,
    ReportEscape,
}

#[derive(Debug, Default)]
pub struct DirectoryReports {
    state: State,
    report: Vec<u8>,
    host: Option<String>,
}

impl DirectoryReports {
    pub fn new(host: Option<String>) -> Self {
        Self {
            host,
            ..Self::default()
        }
    }

    pub fn scan(&mut self, bytes: &[u8]) -> Option<PathBuf> {
        let mut latest = None;
        for &byte in bytes {
            self.state = match (self.state, byte) {
                (_, ESCAPE) if self.state != State::Report => State::Escape,
                (State::Escape, b']') => State::Introducer(0),
                (State::Introducer(0), b'7') => State::Introducer(1),
                (State::Introducer(1), b';') => {
                    self.report.clear();
                    State::Report
                }
                (State::Report, BELL) | (State::ReportEscape, b'\\') => {
                    latest = self.directory().or(latest);
                    State::Ground
                }
                (State::Report, ESCAPE) => State::ReportEscape,
                (State::Report, _) if self.report.len() < MAX_REPORT => {
                    self.report.push(byte);
                    State::Report
                }
                _ => State::Ground,
            };
        }
        latest
    }

    fn directory(&self) -> Option<PathBuf> {
        let rest = self.report.strip_prefix(b"file://")?;
        let slash = rest.iter().position(|byte| *byte == b'/')?;
        let (host, path) = rest.split_at(slash);
        let local = host.is_empty()
            || host == b"localhost"
            || self
                .host
                .as_deref()
                .is_some_and(|name| host == name.as_bytes());
        if !local {
            return None;
        }
        let decoded = percent_decode(path)?;
        let path = Path::new(OsStr::from_bytes(&decoded));
        path.is_absolute().then(|| path.to_path_buf())
    }
}

fn percent_decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}
