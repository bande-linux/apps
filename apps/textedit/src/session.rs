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
