use std::io;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_inbox::{Inbox, Outbox};
use bande_packages::{Client, Progress, native};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Install { key: String, package: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Progress(String, Progress),
    Installed(String),
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

fn run(client: &Client, arch: &str, jobs: &Receiver<Job>, out: &Outbox<Message>) {
    for job in jobs {
        match job {
            Job::Install { key, package } => {
                let progress = |progress| {
                    out.send(Message::Progress(key.clone(), progress));
                };
                let message = match install(client, arch, &package, &progress) {
                    Ok(()) => Message::Installed(key),
                    Err(error) => Message::Failed(key, error),
                };
                if !out.send(message) {
                    return;
                }
            }
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
