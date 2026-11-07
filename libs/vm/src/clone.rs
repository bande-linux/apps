use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::library::{Library, Machine, random_id, random_mac, random_uuid};
use crate::manifest::Forward;
use crate::qemu::{Host, utf8};
use crate::seed::rewrite_seed;
use crate::{Error, Result};

const LINKED_PREFIX: &str = "linked-";
const SSH_PORT: u16 = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneKind {
    Full,
    Linked,
}

pub struct NewClone {
    pub name: String,
    pub ssh_port: u16,
    pub kind: CloneKind,
    pub snapshot: Option<String>,
}

fn copy_folder(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        fs::copy(entry.path(), to.join(entry.file_name()))?;
    }
    Ok(())
}

impl Library {
    pub fn clone_machine(
        &self,
        host: &Host,
        source: &mut Machine,
        new: NewClone,
    ) -> Result<Machine> {
        if new.kind == CloneKind::Linked && !source.manifest.snapshots.is_empty() {
            return Err(Error::HasSnapshots);
        }
        let mut manifest = source.manifest.clone();
        manifest.id = random_uuid()?;
        manifest.mac = random_mac()?;
        manifest.name.clone_from(&new.name);
        manifest.created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        manifest.snapshots.clear();
        manifest.current_snapshot = None;
        manifest.resume_from = None;
        manifest
            .forwards
            .retain(|forward| forward.guest != SSH_PORT);
        manifest.forwards.push(Forward {
            host: new.ssh_port,
            guest: SSH_PORT,
        });
        let mut clone = self.machine(manifest);
        fs::create_dir_all(clone.dir())?;
        let made = self
            .fill_clone(host, source, &clone, &new)
            .and_then(|base| {
                if let Some(base) = base {
                    clone.manifest.base_image = base;
                }
                clone.save()
            });
        if let Err(error) = made {
            let _ = fs::remove_dir_all(clone.dir());
            return Err(error);
        }
        Ok(clone)
    }

    fn fill_clone(
        &self,
        host: &Host,
        source: &mut Machine,
        clone: &Machine,
        new: &NewClone,
    ) -> Result<Option<String>> {
        host.copy_image(&source.vars(), &clone.vars(), new.snapshot.as_deref(), None)?;
        let tpm = source.tpm_state();
        if tpm.is_dir() {
            copy_folder(&tpm, &clone.tpm_state())?;
        }
        if source.seed().exists() {
            rewrite_seed(&source.seed(), &clone.seed(), &clone.manifest.id, &new.name)?;
        }
        match new.kind {
            CloneKind::Full => {
                let base = Path::new(&source.manifest.base_image);
                host.copy_image(
                    &source.disk(),
                    &clone.disk(),
                    new.snapshot.as_deref(),
                    Some(base),
                )?;
                Ok(None)
            }
            CloneKind::Linked => {
                let frozen = self.freeze(host, source)?;
                host.create_overlay(&frozen, &clone.disk(), source.manifest.disk_gib)?;
                Ok(Some(utf8(&frozen)?.to_owned()))
            }
        }
    }

    fn freeze(&self, host: &Host, source: &mut Machine) -> Result<PathBuf> {
        let frozen = self
            .images
            .join(format!("{LINKED_PREFIX}{}.qcow2", random_id()?));
        fs::create_dir_all(&self.images)?;
        fs::rename(source.disk(), &frozen)?;
        let previous =
            std::mem::replace(&mut source.manifest.base_image, utf8(&frozen)?.to_owned());
        let settled = host
            .create_overlay(&frozen, &source.disk(), source.manifest.disk_gib)
            .and_then(|()| source.save());
        if let Err(error) = settled {
            source.manifest.base_image = previous;
            fs::rename(&frozen, source.disk())?;
            return Err(error);
        }
        Ok(frozen)
    }

    pub(crate) fn collect_linked(&self, host: &Host) -> Result<()> {
        let entries = match fs::read_dir(&self.images) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            entries => entries?,
        };
        let mut linked: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(LINKED_PREFIX))
            })
            .collect();
        loop {
            let mut used = Vec::new();
            for machine in self.machines()? {
                used.push(PathBuf::from(machine.manifest.base_image));
            }
            for image in &linked {
                if let Some(backing) = host.backing_of(image)? {
                    used.push(backing);
                }
            }
            let free: Vec<PathBuf> = linked
                .iter()
                .filter(|image| !used.contains(image))
                .cloned()
                .collect();
            if free.is_empty() {
                return Ok(());
            }
            for image in &free {
                fs::remove_file(image)?;
            }
            linked.retain(|image| !free.contains(image));
        }
    }
}
