use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_packages::{Client, Progress};

use crate::deb::{Deb, Machine};
use crate::flow::{self, Outcome, State};

#[derive(Debug, Clone)]
pub enum Job {
    Inspect,
    Install,
    AskRemove,
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Read(Option<Deb>),
    State(State),
    Progress(Progress),
    Back,
}

pub struct Feed {
    receiver: Receiver<Message>,
    wake: UnixStream,
}

impl Feed {
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<Message> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.receiver.try_iter().collect()
    }
}

#[derive(Clone)]
struct Messages {
    sender: Sender<Message>,
    wake: Arc<UnixStream>,
}

impl Messages {
    fn send(&self, message: Message) -> bool {
        self.sender.send(message).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

fn run(client: &Client, path: &Path, machine: &Machine, jobs: Receiver<Job>, out: &Messages) {
    let mut deb: Option<Deb> = None;
    let progress = |progress: Progress| {
        out.send(Message::Progress(progress));
    };
    for job in jobs {
        if let Job::Inspect = job {
            let (read, state) = flow::inspect(client, path, machine);
            deb.clone_from(&read);
            if !out.send(Message::Read(read)) || !out.send(Message::State(state)) {
                break;
            }
            continue;
        }
        let Some(known) = deb.as_ref() else {
            continue;
        };
        let outcome = match job {
            Job::Inspect => continue,
            Job::Install => flow::install(client, path, &progress),
            Job::AskRemove => Outcome::Done(flow::removal(client, known)),
            Job::Remove => flow::remove(client, known, &progress),
        };
        let message = match outcome {
            Outcome::Done(state) => Message::State(state),
            Outcome::Back => Message::Back,
        };
        if !out.send(message) {
            break;
        }
    }
}

pub fn start(client: Client, path: PathBuf, machine: Machine) -> io::Result<(Sender<Job>, Feed)> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    let (sender, receiver) = mpsc::channel();
    let (jobs, queue) = mpsc::channel();
    let out = Messages {
        sender,
        wake: Arc::new(writer),
    };
    std::thread::Builder::new()
        .name("installer".to_owned())
        .spawn(move || run(&client, &path, &machine, queue, &out))?;
    Ok((
        jobs,
        Feed {
            receiver,
            wake: reader,
        },
    ))
}
