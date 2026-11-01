use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use bande_packages::{Applied, Client, Error, ErrorCode, Info, Mode, Package, Progress};
use bande_xdg::DesktopEntry;

use crate::deb::{self, Deb, Machine};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install,
    Upgrade(String),
    Downgrade(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub action: Action,
    pub also: Vec<String>,
    pub removes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Locked,
    SystemPackage,
    Unavailable,
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

fn others(applied: &Applied, own: &str, wanted: &[Info]) -> Vec<String> {
    let mut names: Vec<String> = applied
        .packages
        .iter()
        .filter(|package| package.name != own && wanted.contains(&package.info))
        .map(|package| package.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
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

pub fn inspect(client: &Client, path: &Path, machine: &Machine) -> (Option<Deb>, State) {
    let Some(deb) = deb::read(path) else {
        return (None, State::Invalid);
    };
    if !machine.fits(&deb.arch) {
        let state = State::WrongArchitecture {
            arch: deb.arch.clone(),
            native: machine.native.clone(),
        };
        return (Some(deb), state);
    }
    let state = preview(client, path, &deb);
    (Some(deb), state)
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
            also: others(
                &applied,
                &deb.name,
                &[Info::Installing, Info::Updating, Info::Downgrading],
            ),
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

pub fn install(client: &Client, path: &Path, progress: &dyn Fn(Progress)) -> Outcome {
    match client.install_files(path, Mode::Apply, progress) {
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
