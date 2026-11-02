use std::collections::HashSet;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

use crate::pdf::client::{Incoming, Worker};
use crate::pdf::protocol::{Ask, Glyph, Opened, Render, Reply};

const RESTARTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Opening,
    Locked { retry: bool },
    Open,
    Failed,
}

pub enum Change {
    Opened,
    Locked,
    Failed,
    Pixels(Render, Vec<u8>),
    Text(usize, Vec<Glyph>),
}

pub struct Session {
    path: PathBuf,
    worker: Worker,
    phase: Phase,
    opened: Opened,
    password: String,
    sent: Vec<Render>,
    asked: HashSet<usize>,
    poisoned: HashSet<usize>,
    restarts: usize,
}

impl Session {
    pub fn open(path: &Path) -> io::Result<Self> {
        let worker = Worker::spawn(path)?;
        worker.ask(Ask::Open {
            password: String::new(),
        });
        Ok(Self {
            path: path.to_path_buf(),
            worker,
            phase: Phase::Opening,
            opened: Opened::default(),
            password: String::new(),
            sent: Vec::new(),
            asked: HashSet::new(),
            poisoned: HashSet::new(),
            restarts: 0,
        })
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn opened(&self) -> &Opened {
        &self.opened
    }

    pub fn unlock(&mut self, password: &str) {
        password.clone_into(&mut self.password);
        self.phase = Phase::Opening;
        self.worker.ask(Ask::Open {
            password: password.to_owned(),
        });
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.worker.fd()
    }

    pub fn want(&mut self, renders: Vec<Render>) {
        let renders: Vec<Render> = renders
            .into_iter()
            .filter(|render| !self.poisoned.contains(&render.page))
            .collect();
        if renders != self.sent {
            self.worker.ask(Ask::Want {
                renders: renders.clone(),
            });
            self.sent = renders;
        }
    }

    pub fn want_text(&mut self, pages: impl IntoIterator<Item = usize>) {
        let fresh: Vec<usize> = pages
            .into_iter()
            .filter(|page| !self.poisoned.contains(page) && self.asked.insert(*page))
            .collect();
        if !fresh.is_empty() {
            self.worker.ask(Ask::Text { pages: fresh });
        }
    }

    fn restart(&mut self) -> bool {
        if self.restarts >= RESTARTS {
            return false;
        }
        self.restarts += 1;
        if let Some(render) = self.sent.first() {
            self.poisoned.insert(render.page);
        }
        let Ok(worker) = Worker::spawn(&self.path) else {
            return false;
        };
        worker.ask(Ask::Open {
            password: self.password.clone(),
        });
        self.worker = worker;
        self.sent.clear();
        self.asked.clear();
        true
    }

    pub fn take(&mut self) -> Vec<Change> {
        let mut changes = Vec::new();
        for incoming in self.worker.take() {
            match incoming {
                Incoming::Reply(Reply::Opened(opened)) => {
                    if self.phase != Phase::Open {
                        self.opened = opened;
                        self.phase = Phase::Open;
                        changes.push(Change::Opened);
                    }
                }
                Incoming::Reply(Reply::Locked { retry }) => {
                    self.phase = Phase::Locked { retry };
                    changes.push(Change::Locked);
                }
                Incoming::Reply(Reply::Text { page, glyphs }) => {
                    changes.push(Change::Text(page, glyphs));
                }
                Incoming::Pixels(render, pixels) => changes.push(Change::Pixels(render, pixels)),
                Incoming::Reply(Reply::Failed) | Incoming::Reply(Reply::Rendered { .. }) => {
                    if self.phase != Phase::Open {
                        self.phase = Phase::Failed;
                        changes.push(Change::Failed);
                    }
                }
                Incoming::Gone => {
                    if self.phase == Phase::Open && self.restart() {
                        return changes;
                    }
                    if self.phase != Phase::Failed && self.phase != Phase::Open {
                        self.phase = Phase::Failed;
                        changes.push(Change::Failed);
                    }
                    return changes;
                }
            }
        }
        changes
    }
}
