use std::io;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_inbox::{Inbox, Outbox};
use bande_packages::{Client, Info, Mode, Progress, native};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Install { key: String, package: String },
    PlanRemoval { key: String, package: String },
    Remove { key: String, id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Progress(String, Progress),
    Installed(String),
    RemovalPlanned {
        key: String,
        id: String,
        also: Vec<String>,
    },
    Removed(String),
    Failed(String, String),
}

fn install(
    client: &Client,
    arch: &str,
    package: &str,
    progress: &dyn Fn(Progress),
) -> Result<(), String> {
    let found = client
        .resolve(&[package.to_owned()])
        .map_err(|error| error.to_string())?;
    let id = native(&found, package, arch)
        .map(|package| package.id.clone())
        .ok_or_else(|| format!("{package} is not available for {arch}"))?;
    client
        .install(&[id], progress)
        .map(drop)
        .map_err(|error| error.to_string())
}

fn plan_removal(client: &Client, package: &str) -> Result<(String, Vec<String>), String> {
    let id = client
        .resolve_installed(package)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|found| found.name == package)
        .map(|found| found.id)
        .ok_or_else(|| format!("{package} is not installed"))?;
    let simulated = client
        .remove(std::slice::from_ref(&id), Mode::Simulate, &|_| {})
        .map_err(|error| error.to_string())?;
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

fn run(client: &Client, arch: &str, jobs: &Receiver<Job>, out: &Outbox<Message>) {
    for job in jobs {
        let message = match job {
            Job::Install { key, package } => {
                let progress = |progress| {
                    out.send(Message::Progress(key.clone(), progress));
                };
                match install(client, arch, &package, &progress) {
                    Ok(()) => Message::Installed(key),
                    Err(error) => Message::Failed(key, error),
                }
            }
            Job::PlanRemoval { key, package } => match plan_removal(client, &package) {
                Ok((id, also)) => Message::RemovalPlanned { key, id, also },
                Err(error) => Message::Failed(key, error),
            },
            Job::Remove { key, id } => {
                let progress = |progress| {
                    out.send(Message::Progress(key.clone(), progress));
                };
                match client.remove(std::slice::from_ref(&id), Mode::Apply, &progress) {
                    Ok(_) => Message::Removed(key),
                    Err(error) => Message::Failed(key, error.to_string()),
                }
            }
        };
        if !out.send(message) {
            return;
        }
    }
}

pub fn start(client: Client, arch: String) -> io::Result<(Sender<Job>, Inbox<Message>)> {
    let inbox = Inbox::new()?;
    let out = inbox.outbox();
    let (jobs, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("software-worker".to_owned())
        .spawn(move || run(&client, &arch, &receiver, &out))?;
    Ok((jobs, inbox))
}
