use std::path::PathBuf;

use bande_l10n::tr;

use super::client::{Engine, Error};
use super::informer::Update;
use super::resources::RunSpec;
use crate::volumes::portal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reclaim {
    pub containers: bool,
    pub images: bool,
    pub build_cache: bool,
    pub volumes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    Pull(String),
    Run(RunSpec),
    RemoveImages(Vec<String>),
    PruneImages,
    RemoveVolumes(Vec<String>),
    PruneVolumes,
    ChooseArchive,
    ImportVolume(String, PathBuf),
    ExportVolume(String),
    CreateNetwork(String, String),
    RemoveNetworks(Vec<String>),
    Usage,
    Reclaim(Reclaim),
}

fn each(names: &[String], mut act: impl FnMut(&str) -> Result<(), Error>) -> Result<u64, String> {
    let failed: Vec<String> = names
        .iter()
        .filter_map(|name| act(name).err().map(|error| error.to_string()))
        .collect();
    match failed.is_empty() {
        true => Ok(0),
        false => Err(failed.join("\n")),
    }
}

fn reclaim(engine: &Engine, chosen: Reclaim, api: &str) -> Result<u64, Error> {
    let mut total = 0;
    if chosen.containers {
        total += engine.prune_containers(None)?;
    }
    if chosen.images {
        total += engine.prune_images(None)?;
    }
    if chosen.build_cache {
        total += engine.prune_build_cache()?;
    }
    if chosen.volumes {
        total += engine.prune_volumes(None, api)?;
    }
    Ok(total)
}

pub fn perform(engine: &Engine, api: &str, task: Task, send: impl Fn(Update) -> bool) {
    let message = |error: Error| error.to_string();
    let result = match &task {
        Task::Pull(reference) => engine
            .pull(reference, |pulling| {
                send(Update::Pulling(reference.clone(), pulling));
            })
            .map(|()| 0)
            .map_err(message),
        Task::Run(spec) => engine.run(spec).map(|_| 0).map_err(message),
        Task::RemoveImages(references) => {
            each(references, |reference| engine.remove_image(reference))
        }
        Task::PruneImages => engine.prune_images(None).map_err(message),
        Task::RemoveVolumes(names) => each(names, |name| engine.remove_volume(name)),
        Task::PruneVolumes => engine.prune_volumes(None, api).map_err(message),
        Task::ChooseArchive => {
            match portal::choose_archive(&tr!("Import Volume")) {
                Ok(Some(path)) => {
                    send(Update::Chosen(path));
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot choose an archive: {error}"),
            }
            return;
        }
        Task::ImportVolume(name, archive) => engine
            .import_volume(name, archive)
            .map(|()| 0)
            .map_err(message),
        Task::ExportVolume(name) => {
            let chosen = portal::choose_save_path(&tr!("Export Volume"), &format!("{name}.tar"));
            match chosen {
                Ok(Some(path)) => engine
                    .export_volume(name, &path)
                    .map(|()| 0)
                    .map_err(message),
                Ok(None) => return,
                Err(error) => Err(error.to_string()),
            }
        }
        Task::CreateNetwork(name, subnet) => engine
            .create_network(name, subnet)
            .map(|()| 0)
            .map_err(message),
        Task::RemoveNetworks(ids) => each(ids, |id| engine.remove_network(id)),
        Task::Usage => {
            match engine.disk_usage() {
                Ok(usage) => {
                    send(Update::Usage(usage));
                }
                Err(error) => tracing::info!("cannot read the disk usage: {error}"),
            }
            return;
        }
        Task::Reclaim(chosen) => reclaim(engine, *chosen, api).map_err(message),
    };
    send(Update::Finished(task, result));
}
