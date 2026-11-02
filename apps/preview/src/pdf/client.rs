use std::io::{self, BufReader, BufWriter};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use bande_inbox::{Inbox, Outbox};

use crate::pdf::protocol::{self, Ask, Reply, WORKER_FLAG};

const WORKER_VARIABLE: &str = "BANDE_PREVIEW_WORKER";
const OWN_PROGRAM: &str = "/proc/self/exe";

pub enum Incoming {
    Reply(Reply),
    Pixels(protocol::Render, Vec<u8>),
    Gone,
}

pub struct Worker {
    child: Child,
    asks: Sender<Ask>,
    incoming: Inbox<Incoming>,
}

fn program() -> PathBuf {
    std::env::var_os(WORKER_VARIABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(OWN_PROGRAM))
}

fn write_asks(stdin: ChildStdin, asks: Receiver<Ask>) {
    let mut out = BufWriter::new(stdin);
    for ask in asks {
        if protocol::write(&mut out, &ask, &[]).is_err() {
            return;
        }
    }
}

fn read_replies(stdout: ChildStdout, incoming: Outbox<Incoming>) {
    let mut input = BufReader::new(stdout);
    loop {
        let message = match protocol::read::<Reply>(&mut input) {
            Ok(Some(Reply::Rendered { render })) => match protocol::payload(&mut input, &render) {
                Ok(pixels) => Incoming::Pixels(render, pixels),
                Err(_) => Incoming::Gone,
            },
            Ok(Some(reply)) => Incoming::Reply(reply),
            Ok(None) | Err(_) => Incoming::Gone,
        };
        let gone = matches!(message, Incoming::Gone);
        if !incoming.send(message) || gone {
            return;
        }
    }
}

impl Worker {
    pub fn spawn(path: &Path) -> io::Result<Self> {
        let mut child = Command::new(program())
            .arg(WORKER_FLAG)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("the PDF renderer has no pipes"));
        };
        let incoming = Inbox::new()?;
        let (asks, receiver) = mpsc::channel();
        let outbox = incoming.outbox();
        thread::Builder::new()
            .name("pdf-asks".to_owned())
            .spawn(move || write_asks(stdin, receiver))?;
        thread::Builder::new()
            .name("pdf-replies".to_owned())
            .spawn(move || read_replies(stdout, outbox))?;
        Ok(Self {
            child,
            asks,
            incoming,
        })
    }

    pub fn ask(&self, ask: Ask) {
        let _ = self.asks.send(ask);
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.incoming.fd()
    }

    pub fn take(&mut self) -> Vec<Incoming> {
        self.incoming.drain()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
