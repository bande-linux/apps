use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::snapshot::Snapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forward {
    pub host: u16,
    pub guest: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Linux,
    Windows,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    pub tag: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub os: String,
    pub created: u64,
    pub cpus: u32,
    pub memory_mib: u64,
    pub disk_gib: u64,
    pub base_image: String,
    pub mac: String,
    pub forwards: Vec<Forward>,
    #[serde(default)]
    pub snapshots: Vec<Snapshot>,
    #[serde(default)]
    pub current_snapshot: Option<String>,
    #[serde(default)]
    pub resume_from: Option<String>,
    #[serde(default)]
    pub preset: Preset,
    #[serde(default)]
    pub installer: Option<String>,
    #[serde(default)]
    pub shares: Vec<Share>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let staging = path.with_extension("json.new");
        let mut file = File::create(&staging)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        persist(&file, &staging, path)
    }

    pub fn ssh_port(&self) -> Option<u16> {
        self.forwards
            .iter()
            .find(|forward| forward.guest == 22)
            .map(|forward| forward.host)
    }
}

pub(crate) fn persist(file: &File, staging: &Path, path: &Path) -> Result<()> {
    file.sync_all()?;
    fs::rename(staging, path)?;
    if let Some(folder) = path.parent() {
        File::open(folder)?.sync_all()?;
    }
    Ok(())
}
