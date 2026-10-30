use std::collections::HashMap;
use std::path::Path;

const KILOBYTE: u64 = 1024;
const COMM_LENGTH: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stat {
    pub(crate) comm: String,
    pub(crate) state: char,
    pub(crate) ppid: u32,
    pub(crate) cpu_ticks: u64,
    pub(crate) threads: u32,
    pub(crate) start_ticks: u64,
    pub(crate) resident_pages: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct IoBytes {
    pub(crate) read: u64,
    pub(crate) written: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct DrmUse {
    pub(crate) engine_ns: u64,
    pub(crate) memory: u64,
}

fn parse_size(value: &str) -> Option<u64> {
    let mut words = value.split_whitespace();
    let number = words.next()?.parse::<u64>().ok()?;
    let scale = match words.next() {
        None => 1,
        Some("KiB") => KILOBYTE,
        Some("MiB") => KILOBYTE * KILOBYTE,
        Some("GiB") => KILOBYTE * KILOBYTE * KILOBYTE,
        Some(_) => return None,
    };
    Some(number * scale)
}

pub(crate) fn parse_drm_fdinfo(text: &str) -> Option<(u64, DrmUse)> {
    let client = field(text, "drm-client-id")?.parse().ok()?;
    let mut engines = None;
    let (mut resident, mut memory) = (0, 0);
    for (key, value) in text.lines().filter_map(|line| line.split_once(':')) {
        let value = value.trim();
        if key.starts_with("drm-engine-capacity-") {
            continue;
        }
        if key.starts_with("drm-engine-") {
            let busy = value.strip_suffix("ns")?.trim().parse::<u64>().ok()?;
            engines = Some(engines.unwrap_or(0) + busy);
        } else if key.starts_with("drm-resident-") {
            resident += parse_size(value).unwrap_or(0);
        } else if key.starts_with("drm-memory-") {
            memory += parse_size(value).unwrap_or(0);
        }
    }
    Some((
        client,
        DrmUse {
            engine_ns: engines?,
            memory: if resident > 0 { resident } else { memory },
        },
    ))
}

pub(crate) fn parse_stat(text: &str) -> Option<Stat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_owned();
    let fields = text
        .get(close + 1..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let number = |index: usize| fields.get(index)?.parse::<u64>().ok();
    Some(Stat {
        comm,
        state: fields.first()?.chars().next()?,
        ppid: u32::try_from(number(1)?).ok()?,
        cpu_ticks: number(11)? + number(12)?,
        threads: u32::try_from(number(17)?).ok()?,
        start_ticks: number(19)?,
        resident_pages: number(21)?,
    })
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
        .map(str::trim)
}

pub(crate) fn parse_uid(status: &str) -> Option<u32> {
    field(status, "Uid")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub(crate) fn parse_io(text: &str) -> Option<IoBytes> {
    Some(IoBytes {
        read: field(text, "read_bytes")?.parse().ok()?,
        written: field(text, "write_bytes")?.parse().ok()?,
    })
}

pub(crate) fn parse_kilobytes(text: &str, key: &str) -> Option<u64> {
    field(text, key)?
        .strip_suffix("kB")?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kilobytes| kilobytes * KILOBYTE)
}

pub(crate) fn parse_cmdline(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect()
}

pub(crate) fn parse_cgroup_unit(text: &str) -> Option<String> {
    let path = text.lines().find_map(|line| line.strip_prefix("0::"))?;
    path.rsplit('/')
        .next()
        .filter(|unit| !unit.is_empty())
        .map(str::to_owned)
}

pub(crate) fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            Some((uid, name.to_owned()))
        })
        .collect()
}

pub(crate) fn display_name(comm: &str, command: &[String]) -> String {
    let program = command
        .first()
        .and_then(|first| first.split(' ').next())
        .and_then(|first| Path::new(first).file_name())
        .map(|name| name.to_string_lossy());
    match program {
        Some(program) if comm.len() >= COMM_LENGTH && program.starts_with(comm) => {
            program.into_owned()
        }
        _ => comm.to_owned(),
    }
}
