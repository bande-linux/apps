use std::io;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_inbox::{Inbox, Outbox};
use bande_packages::{Client, Error, ErrorCode, Info, Mode, Package, Progress, native};

use crate::media::Loaded;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Install { key: String, package: String },
    Remove { key: String, id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    PlanRemoval { key: String, package: String },
    Details { key: String, package: String },
    Updates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NoNetwork,
    Locked,
    NoSpace,
    NotFound,
    Dependencies,
    SystemPackage,
    Unsigned,
    NoPackageKit,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub problem: Problem,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    pub available: Option<String>,
    pub installed: Option<String>,
    pub size: u64,
    pub download: u64,
    pub extra: usize,
    pub origin: String,
    pub homepage: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Loaded(Loaded),
    Online(bool),
    Progress(String, Progress),
    Installed(String),
    Removed(String),
    Stopped(String),
    RemovalPlanned {
        key: String,
        id: String,
        also: Vec<String>,
    },
    Details(String, Facts),
    Updates(Vec<Package>),
    Failed(String, Failure),
}

fn failure(error: &Error) -> Option<Failure> {
    tracing::warn!("PackageKit: {error}");
    let (problem, details) = match error {
        Error::Missing => (Problem::NoPackageKit, String::new()),
        Error::Vanished | Error::Bus(_) => (Problem::Other, error.to_string()),
        Error::Failed(failure) => {
            let problem = match failure.code {
                ErrorCode::Cancelled | ErrorCode::NotAuthorized => return None,
                ErrorCode::NoNetwork => Problem::NoNetwork,
                ErrorCode::Locked => Problem::Locked,
                ErrorCode::NoSpace => Problem::NoSpace,
                ErrorCode::NotFound => Problem::NotFound,
                ErrorCode::Dependencies => Problem::Dependencies,
                ErrorCode::SystemPackage => Problem::SystemPackage,
                ErrorCode::Unsigned => Problem::Unsigned,
                _ => Problem::Other,
            };
            (problem, failure.details.clone())
        }
    };
    Some(Failure { problem, details })
}

fn settled(key: String, result: Result<Message, Error>) -> Message {
    result.unwrap_or_else(|error| match failure(&error) {
        Some(failure) => Message::Failed(key, failure),
        None => Message::Stopped(key),
    })
}

fn not_found(details: String) -> Error {
    Error::Failed(bande_packages::Failure {
        code: ErrorCode::NotFound,
        details,
    })
}

fn install(
    client: &Client,
    arch: &str,
    package: &str,
    progress: &dyn Fn(Progress),
) -> Result<(), Error> {
    let found = client.resolve(&[package.to_owned()])?;
    let id = native(&found, package, arch)
        .map(|package| package.id.clone())
        .ok_or_else(|| not_found(format!("{package} is not available for {arch}")))?;
    client.install(&[id], Mode::Apply, progress).map(drop)
}

fn plan_removal(client: &Client, package: &str) -> Result<(String, Vec<String>), Error> {
    let id = client
        .resolve_installed(package)?
        .into_iter()
        .find(|found| found.name == package)
        .map(|found| found.id)
        .ok_or_else(|| not_found(format!("{package} is not installed")))?;
    let simulated = client.remove(std::slice::from_ref(&id), Mode::Simulate, &|_| {})?;
    let mut also: Vec<String> = simulated
        .packages
        .into_iter()
        .filter(|removed| removed.info == Info::Removing && removed.name != package)
        .map(|removed| removed.name)
        .collect();
    also.sort();
    also.dedup();
    Ok((id, also))
}

fn origin(data: &str) -> String {
    data.rsplit(':').next().unwrap_or(data).to_owned()
}

fn download(client: &Client, id: &str) -> Result<(u64, usize), Error> {
    let simulated = client.install(&[id.to_owned()], Mode::Simulate, &|_| {})?;
    let ids: Vec<String> = simulated
        .packages
        .iter()
        .filter(|package| matches!(package.info, Info::Installing | Info::Updating))
        .map(|package| package.id.clone())
        .collect();
    let size = client
        .details(&ids)?
        .iter()
        .map(|details| details.size)
        .sum();
    Ok((size, ids.len().saturating_sub(1)))
}

fn details(client: &Client, arch: &str, package: &str) -> Result<Facts, Error> {
    let found = client.resolve(&[package.to_owned()])?;
    let installed = found
        .iter()
        .find(|candidate| candidate.name == package && candidate.info == Info::Installed);
    let others: Vec<Package> = found
        .iter()
        .filter(|candidate| candidate.info != Info::Installed)
        .cloned()
        .collect();
    let available = native(&others, package, arch);
    let Some(chosen) = installed.or(available) else {
        return Ok(Facts::default());
    };
    let details = client.details(std::slice::from_ref(&chosen.id))?;
    let details = details.first();
    let own = details.map(|details| details.size).unwrap_or(0);
    let (size, download, extra) = match installed {
        Some(_) => (own, 0, 0),
        None => {
            let (download, extra) = download(client, &chosen.id).unwrap_or_else(|error| {
                tracing::debug!("cannot work out the download of {package}: {error}");
                (own, 0)
            });
            (0, download, extra)
        }
    };
    Ok(Facts {
        available: available.map(|package| package.version.clone()),
        installed: installed.map(|package| package.version.clone()),
        size,
        download,
        extra,
        origin: origin(&chosen.origin),
        homepage: details
            .map(|details| details.url.clone())
            .unwrap_or_default(),
    })
}

fn transact(client: &Client, arch: &str, jobs: &Receiver<Job>, out: &Outbox<Message>) {
    for job in jobs {
        let message = match job {
            Job::Install { key, package } => {
                let progress = |progress| {
                    out.send(Message::Progress(key.clone(), progress));
                };
                let installed = install(client, arch, &package, &progress)
                    .map(|()| Message::Installed(key.clone()));
                settled(key, installed)
            }
            Job::Remove { key, id } => {
                let progress = |progress| {
                    out.send(Message::Progress(key.clone(), progress));
                };
                let removed = client
                    .remove(std::slice::from_ref(&id), Mode::Apply, &progress)
                    .map(|_| Message::Removed(key.clone()));
                settled(key, removed)
            }
        };
        if !out.send(message) {
            return;
        }
    }
}

fn ask(client: &Client, arch: &str, queries: &Receiver<Query>, out: &Outbox<Message>) {
    for query in queries {
        let message = match query {
            Query::PlanRemoval { key, package } => {
                let planned =
                    plan_removal(client, &package).map(|(id, also)| Message::RemovalPlanned {
                        key: key.clone(),
                        id,
                        also,
                    });
                settled(key, planned)
            }
            Query::Details { key, package } => match details(client, arch, &package) {
                Ok(facts) => Message::Details(key, facts),
                Err(error) => {
                    tracing::warn!("cannot read the details of {package}: {error}");
                    continue;
                }
            },
            Query::Updates => match client.updates() {
                Ok(updates) => Message::Updates(updates),
                Err(error) => {
                    tracing::warn!("cannot look for updates: {error}");
                    continue;
                }
            },
        };
        if !out.send(message) {
            return;
        }
    }
}

pub struct Workers {
    pub jobs: Sender<Job>,
    pub queries: Sender<Query>,
    pub inbox: Inbox<Message>,
}

pub fn start(transactions: Client, queries: Client, arch: &str) -> io::Result<Workers> {
    let inbox = Inbox::new()?;
    let (jobs, job_receiver) = mpsc::channel();
    let (asked, query_receiver) = mpsc::channel();
    let out = inbox.outbox();
    let job_arch = arch.to_owned();
    std::thread::Builder::new()
        .name("software-transactions".to_owned())
        .spawn(move || transact(&transactions, &job_arch, &job_receiver, &out))?;
    let out = inbox.outbox();
    let query_arch = arch.to_owned();
    std::thread::Builder::new()
        .name("software-queries".to_owned())
        .spawn(move || ask(&queries, &query_arch, &query_receiver, &out))?;
    Ok(Workers {
        jobs,
        queries: asked,
        inbox,
    })
}
