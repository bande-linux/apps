use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bande_xdg::replace_file;
use serde::{Deserialize, Serialize};

use crate::file::{MAX_FILE_BYTES, read_bounded};

const RECORD: &str = "json";
const TEXT: &str = "text";
const RECENT: &str = "recent";
const RECENT_COUNT: usize = 10;
const MAX_RECORD_BYTES: u64 = 64 << 10;
const VERSIONS: &str = "versions";
const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;
const MONTH: u64 = 30 * DAY;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub path: Option<PathBuf>,
    pub title: String,
    pub encoding: String,
    pub bom: bool,
    pub line_ending: String,
    pub edited: bool,
    pub monospace: bool,
    pub page: bool,
    pub size: f32,
    pub opened: u128,
    pub format: String,
    pub rich: bool,
}

#[derive(Debug, Clone)]
pub struct Session {
    folder: PathBuf,
}

pub fn new_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{now:x}-{:x}", std::process::id())
}

pub fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub fn seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn versions_to_thin(newest_first: &[u64], now: u64) -> Vec<u64> {
    let mut last_bucket = None;
    let mut thinned = Vec::new();
    for version in newest_first {
        let age = now.saturating_sub(*version);
        let bucket = match age {
            age if age < DAY => (HOUR, version / HOUR),
            age if age < MONTH => (DAY, version / DAY),
            _ => (7 * DAY, version / (7 * DAY)),
        };
        if last_bucket == Some(bucket) {
            thinned.push(*version);
        }
        last_bucket = Some(bucket);
    }
    thinned
}

impl Session {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    fn documents(&self) -> PathBuf {
        self.folder.join("documents")
    }

    fn file(&self, id: &str, kind: &str) -> PathBuf {
        self.documents().join(format!("{id}.{kind}"))
    }

    pub fn store(&self, id: &str, record: &Record, text: Option<&str>) -> io::Result<()> {
        match text {
            Some(text) => replace_file(&self.file(id, TEXT), text)?,
            None if !record.edited => match fs::remove_file(self.file(id, TEXT)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            },
            None => {}
        }
        let json = serde_json::to_vec(record).map_err(io::Error::other)?;
        replace_file(&self.file(id, RECORD), json)
    }

    pub fn forget(&self, id: &str) {
        for kind in [RECORD, TEXT] {
            if let Err(error) = fs::remove_file(self.file(id, kind))
                && error.kind() != io::ErrorKind::NotFound
            {
                tracing::warn!("cannot forget the autosaved document {id}: {error}");
            }
        }
    }

    pub fn records(&self) -> Vec<(String, Record)> {
        let Ok(entries) = fs::read_dir(self.documents()) else {
            return Vec::new();
        };
        let mut records: Vec<(String, Record)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != RECORD {
                    return None;
                }
                let id = path.file_stem()?.to_str()?.to_owned();
                let bytes = read_bounded(&path, MAX_RECORD_BYTES).ok()?;
                match serde_json::from_slice(&bytes) {
                    Ok(record) => Some((id, record)),
                    Err(error) => {
                        tracing::warn!("cannot read the autosaved document {id}: {error}");
                        None
                    }
                }
            })
            .collect();
        records.sort_by_key(|(_, record)| record.opened);
        records
    }

    fn versions_of(&self, path: &Path) -> PathBuf {
        let hash = path
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
            });
        self.folder.join(VERSIONS).join(format!("{hash:016x}"))
    }

    pub fn versions(&self, path: &Path) -> Vec<u64> {
        let mut versions: Vec<u64> = fs::read_dir(self.versions_of(path))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect();
        versions.sort_unstable_by(|left, right| right.cmp(left));
        versions
    }

    pub fn version(&self, path: &Path, version: u64) -> io::Result<Vec<u8>> {
        read_bounded(
            &self.versions_of(path).join(version.to_string()),
            MAX_FILE_BYTES,
        )
    }

    pub fn keep_version(&self, path: &Path, bytes: &[u8], at: u64) -> io::Result<()> {
        let folder = self.versions_of(path);
        if self
            .versions(path)
            .first()
            .is_some_and(|newest| self.version(path, *newest).is_ok_and(|old| old == bytes))
        {
            return Ok(());
        }
        replace_file(&folder.join(at.to_string()), bytes)?;
        for old in versions_to_thin(&self.versions(path), at) {
            fs::remove_file(folder.join(old.to_string()))?;
        }
        Ok(())
    }

    pub fn text(&self, id: &str) -> io::Result<Vec<u8>> {
        read_bounded(&self.file(id, TEXT), MAX_FILE_BYTES)
    }

    pub fn recent(&self) -> Vec<PathBuf> {
        fs::read_to_string(self.folder.join(RECENT))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with('/'))
            .take(RECENT_COUNT)
            .map(PathBuf::from)
            .collect()
    }

    pub fn remember(&self, recent: &mut Vec<PathBuf>, path: &Path) {
        recent.retain(|known| known != path);
        recent.insert(0, path.to_path_buf());
        recent.truncate(RECENT_COUNT);
        self.write_recent(recent);
    }

    pub fn write_recent(&self, recent: &[PathBuf]) {
        let lines: String = recent
            .iter()
            .filter_map(|path| path.to_str())
            .filter(|path| !path.contains('\n'))
            .map(|path| format!("{path}\n"))
            .collect();
        if let Err(error) = replace_file(&self.folder.join(RECENT), lines) {
            tracing::warn!("cannot remember the recent documents: {error}");
        }
    }
}
