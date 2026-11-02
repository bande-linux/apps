use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use bande_packages::{Applied, Client, Error, ErrorCode, Info, Mode, Package, Progress};
use bande_xdg::DesktopEntry;

use crate::deb::{self, Deb, Machine};

const INSTALLING: [Info; 3] = [Info::Installing, Info::Updating, Info::Downgrading];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install,
    Upgrade(String),
    Downgrade(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub id: String,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub action: Action,
    pub signed: Option<Signed>,
    pub also: Vec<String>,
    pub download: u64,
    pub removes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Locked,
    SystemPackage,
    Unavailable,
    NoNetwork,
    NoSpace,
    Dependencies,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Reading,
    Invalid,
    WrongArchitecture {
        arch: String,
        native: String,
    },
    Present,
    Ready(Preview),
    Blocked {
        missing: Vec<String>,
        details: String,
    },
    Installing(Progress),
    Installed {
        apps: Vec<DesktopEntry>,
        restart: bool,
    },
    ConfirmRemove(Vec<String>),
    Removing(Progress),
    Removed,
    Failed {
        problem: Problem,
        details: String,
    },
}

fn other_packages<'a>(
    applied: &'a Applied,
    own: &'a str,
    wanted: &'a [Info],
) -> impl Iterator<Item = &'a Package> {
    applied
        .packages
        .iter()
        .filter(move |package| package.name != own && wanted.contains(&package.info))
}

fn others(applied: &Applied, own: &str, wanted: &[Info]) -> Vec<String> {
    let mut names: Vec<String> = other_packages(applied, own, wanted)
        .map(|package| package.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn download_size(client: &Client, applied: &Applied, own: &str) -> u64 {
    let ids: Vec<String> = other_packages(applied, own, &INSTALLING)
        .map(|package| package.id.clone())
        .collect();
    if ids.is_empty() {
        return 0;
    }
    client
        .details(&ids)
        .map(|details| details.iter().map(|details| details.size).sum())
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read the download size: {error}");
            0
        })
}

fn failed(error: &Error) -> State {
    let (problem, details) = match error {
        Error::Missing => (Problem::Unavailable, String::new()),
        Error::Vanished => (Problem::Other, error.to_string()),
        Error::Bus(bus) => (Problem::Other, bus.to_string()),
        Error::Failed(failure) => {
            let problem = match failure.code {
                ErrorCode::Locked => Problem::Locked,
                ErrorCode::SystemPackage => Problem::SystemPackage,
                ErrorCode::NoNetwork => Problem::NoNetwork,
                ErrorCode::NoSpace => Problem::NoSpace,
                ErrorCode::Dependencies => Problem::Dependencies,
                _ => Problem::Other,
            };
            (problem, failure.details.clone())
        }
    };
    tracing::warn!("package installer: {error}");
    State::Failed { problem, details }
}

pub fn installed(client: &Client, deb: &Deb) -> Result<Option<Package>, Error> {
    Ok(client
        .resolve_installed(&deb.name)?
        .into_iter()
        .find(|package| package.arch == deb.arch || deb.arch == "all"))
}

pub fn inspect(client: &Client, path: &Path, machine: &Machine, deb: Option<&Deb>) -> State {
    let Some(deb) = deb else {
        return State::Invalid;
    };
    if !machine.fits(&deb.arch) {
        return State::WrongArchitecture {
            arch: deb.arch.clone(),
            native: machine.native.clone(),
        };
    }
    preview(client, path, deb)
}

pub fn preview(client: &Client, path: &Path, deb: &Deb) -> State {
    let present = match installed(client, deb) {
        Ok(present) => present,
        Err(error) => return failed(&error),
    };
    let action = match present {
        None => Action::Install,
        Some(package) => match deb::compare(&deb.version, &package.version) {
            Ordering::Equal => return State::Present,
            Ordering::Greater => Action::Upgrade(package.version),
            Ordering::Less => Action::Downgrade(package.version),
        },
    };
    match client.install_files(path, Mode::Simulate, &|_| {}) {
        Ok(applied) => State::Ready(Preview {
            action,
            signed: signed(client, deb),
            also: others(&applied, &deb.name, &INSTALLING),
            download: download_size(client, &applied, &deb.name),
            removes: others(&applied, &deb.name, &[Info::Removing, Info::Obsoleting]),
        }),
        Err(Error::Failed(failure)) if failure.code == ErrorCode::Dependencies => State::Blocked {
            missing: deb::missing_requirements(&failure.details),
            details: failure.details,
        },
        Err(error) => failed(&error),
    }
}

pub fn apps(files: &[String]) -> Vec<DesktopEntry> {
    files
        .iter()
        .map(PathBuf::from)
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "desktop")
                && path
                    .parent()
                    .is_some_and(|parent| parent.ends_with("share/applications"))
        })
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let id = path.file_name()?.to_string_lossy().into_owned();
            DesktopEntry::parse(&id, &path, &text, None, &bande_xdg::current_desktops())
        })
        .collect()
}

pub enum Outcome {
    Done(State),
    Back,
}

fn signed(client: &Client, deb: &Deb) -> Option<Signed> {
    let found = client
        .resolve(std::slice::from_ref(&deb.name))
        .inspect_err(|error| tracing::warn!("cannot look in the software sources: {error}"))
        .ok()?;
    found
        .into_iter()
        .find(|package| {
            package.name == deb.name
                && package.arch == deb.arch
                && package.version == deb.version
                && package.info != Info::Installed
        })
        .map(|package| Signed {
            origin: package
                .origin
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .to_owned(),
            id: package.id,
        })
}

pub fn install(
    client: &Client,
    path: &Path,
    signed: Option<&str>,
    progress: &dyn Fn(Progress),
) -> Outcome {
    let applied = match signed {
        Some(id) => client.install(&[id.to_owned()], Mode::Apply, progress),
        None => client.install_files(path, Mode::Apply, progress),
    };
    match applied {
        Ok(applied) => {
            let files = client.files_local(path).unwrap_or_else(|error| {
                tracing::warn!("cannot list the installed files: {error}");
                Vec::new()
            });
            Outcome::Done(State::Installed {
                apps: apps(&files),
                restart: applied.restart,
            })
        }
        Err(Error::Failed(failure))
            if matches!(
                failure.code,
                ErrorCode::NotAuthorized | ErrorCode::Cancelled
            ) =>
        {
            Outcome::Back
        }
        Err(error) => Outcome::Done(failed(&error)),
    }
}

fn installed_ids(client: &Client, deb: &Deb) -> Result<Vec<String>, Error> {
    Ok(installed(client, deb)?
        .map(|package| vec![package.id])
        .unwrap_or_default())
}

pub fn removal(client: &Client, deb: &Deb) -> State {
    let simulated =
        installed_ids(client, deb).and_then(|ids| client.remove(&ids, Mode::Simulate, &|_| {}));
    match simulated {
        Ok(applied) => State::ConfirmRemove(others(
            &applied,
            &deb.name,
            &[Info::Removing, Info::Obsoleting],
        )),
        Err(error) => failed(&error),
    }
}

pub fn remove(client: &Client, deb: &Deb, progress: &dyn Fn(Progress)) -> Outcome {
    let removed =
        installed_ids(client, deb).and_then(|ids| client.remove(&ids, Mode::Apply, progress));
    match removed {
        Ok(_) => Outcome::Done(State::Removed),
        Err(Error::Failed(failure))
            if matches!(
                failure.code,
                ErrorCode::NotAuthorized | ErrorCode::Cancelled
            ) =>
        {
            Outcome::Back
        }
        Err(error) => Outcome::Done(failed(&error)),
    }
}
