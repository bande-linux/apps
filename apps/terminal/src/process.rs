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
    Number(u32),
    Body(u32),
    BodyEscape(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Directory(PathBuf),
    CommandStarted,
    CommandFinished(Option<i32>),
    Notify { title: String, body: String },
}

const DIRECTORY: u32 = 7;
const PROMPT_MARKS: u32 = 133;
const NOTIFY: u32 = 777;
const MAX_NUMBER: u32 = 100_000;

#[derive(Debug, Default)]
pub struct Reports {
    state: State,
    body: Vec<u8>,
    host: Option<String>,
}

impl Reports {
    pub fn new(host: Option<String>) -> Self {
        Self {
            host,
            ..Self::default()
        }
    }

    pub fn scan(&mut self, bytes: &[u8]) -> Vec<Report> {
        let mut reports = Vec::new();
        for &byte in bytes {
            self.state = match (self.state, byte) {
                (State::Body(number), ESCAPE) => State::BodyEscape(number),
                (State::Body(number), BELL) | (State::BodyEscape(number), b'\\') => {
                    reports.extend(self.report(number));
                    State::Ground
                }
                (_, ESCAPE) => State::Escape,
                (State::Escape | State::BodyEscape(_), b']') => State::Number(0),
                (State::Number(number), b'0'..=b'9') if number < MAX_NUMBER => {
                    State::Number(number * 10 + u32::from(byte - b'0'))
                }
                (State::Number(number), b';') => {
                    self.body.clear();
                    State::Body(number)
                }
                (State::Body(number), _) if self.body.len() < MAX_REPORT => {
                    self.body.push(byte);
                    State::Body(number)
                }
                _ => State::Ground,
            };
        }
        reports
    }

    fn report(&self, number: u32) -> Option<Report> {
        match number {
            DIRECTORY => self.directory().map(Report::Directory),
            PROMPT_MARKS => match self.body.split_first() {
                Some((b'B' | b'C', _)) => Some(Report::CommandStarted),
                Some((b'D', rest)) => {
                    let code = std::str::from_utf8(rest)
                        .ok()
                        .and_then(|rest| rest.strip_prefix(';'))
                        .and_then(|rest| rest.split(';').next())
                        .and_then(|code| code.parse().ok());
                    Some(Report::CommandFinished(code))
                }
                _ => None,
            },
            NOTIFY => {
                let text = String::from_utf8_lossy(&self.body);
                let mut fields = text.splitn(3, ';');
                match (fields.next(), fields.next(), fields.next()) {
                    (Some("notify"), Some(title), body) => Some(Report::Notify {
                        title: title.to_owned(),
                        body: body.unwrap_or_default().to_owned(),
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn directory(&self) -> Option<PathBuf> {
        let rest = self.body.strip_prefix(b"file://")?;
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
