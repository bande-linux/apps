use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::library::{Machine, random_id};
use crate::qemu::{Host, utf8};
use crate::qmp::Qmp;
use crate::{Error, Result};

const STATE_NODE: &str = "disk";
const DEVICES: [&str; 2] = ["disk", "vars"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub created: u64,
    pub parent: Option<String>,
    pub memory: bool,
    pub memory_bytes: u64,
}

impl Machine {
    pub fn take_snapshot(&mut self, host: &Host, qmp: Option<&mut Qmp>, name: &str) -> Result<()> {
        let id = random_id()?;
        let memory = qmp.is_some();
        match qmp {
            Some(qmp) => qmp.snapshot_save(&id, STATE_NODE, &DEVICES)?,
            None => self.each_image(|image| host.snapshot_image("-c", &id, image))?,
        }
        let disk = self.disk();
        let manifest = &mut self.manifest;
        let memory_bytes = match memory {
            true => host.state_size(&disk, &id).unwrap_or_default(),
            false => 0,
        };
        manifest.snapshots.push(Snapshot {
            id: id.clone(),
            name: name.to_owned(),
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
            parent: manifest.current_snapshot.clone(),
            memory,
            memory_bytes,
        });
        manifest.current_snapshot = Some(id);
        self.save()
    }

    pub fn revert_snapshot(&mut self, host: &Host, qmp: Option<&mut Qmp>, id: &str) -> Result<()> {
        let snapshot = self.snapshot(id)?;
        match (qmp, snapshot.memory) {
            (Some(qmp), true) => qmp.snapshot_load(id, STATE_NODE, &DEVICES)?,
            (Some(_), false) => return Err(Error::NeedsShutDown),
            (None, true) => self.manifest.resume_from = Some(id.to_owned()),
            (None, false) => self.each_image(|image| host.snapshot_image("-a", id, image))?,
        }
        self.manifest.current_snapshot = Some(id.to_owned());
        self.save()
    }

    pub fn resume_snapshot(&self, qmp: &mut Qmp) -> Result<()> {
        if let Some(tag) = &self.manifest.resume_from {
            qmp.snapshot_load(tag, STATE_NODE, &DEVICES)?;
            qmp.resume()?;
        }
        Ok(())
    }

    pub fn delete_snapshot(&mut self, host: &Host, qmp: Option<&mut Qmp>, id: &str) -> Result<()> {
        let parent = self.snapshot(id)?.parent;
        match qmp {
            Some(qmp) => qmp.snapshot_delete(id, &DEVICES)?,
            None => self.each_image(|image| host.snapshot_image("-d", id, image))?,
        }
        let manifest = &mut self.manifest;
        manifest.snapshots.retain(|snapshot| snapshot.id != id);
        for snapshot in &mut manifest.snapshots {
            if snapshot.parent.as_deref() == Some(id) {
                snapshot.parent.clone_from(&parent);
            }
        }
        if manifest.current_snapshot.as_deref() == Some(id) {
            manifest.current_snapshot = parent;
        }
        if manifest.resume_from.as_deref() == Some(id) {
            manifest.resume_from = None;
        }
        self.save()
    }

    fn snapshot(&self, id: &str) -> Result<Snapshot> {
        self.manifest
            .snapshots
            .iter()
            .find(|snapshot| snapshot.id == id)
            .cloned()
            .ok_or_else(|| Error::NotListed(id.to_owned()))
    }

    fn each_image(&self, mut act: impl FnMut(&str) -> Result<()>) -> Result<()> {
        act(utf8(&self.disk())?)?;
        act(utf8(&self.vars())?)
    }
}
