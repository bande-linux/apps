use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_inbox::{Inbox, Outbox};

use bande_content::{Decoded, Failure, Kind};

pub enum Opened {
    Image(Box<Decoded>),
    Document,
}

pub struct Loaded {
    pub generation: u64,
    pub path: PathBuf,
    pub result: Result<Opened, Failure>,
}

#[derive(Default)]
struct State {
    wanted: Option<(u64, PathBuf)>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next(&self) -> Option<(u64, PathBuf)> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            if let Some(wanted) = state.wanted.take() {
                return Some(wanted);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

pub struct Loader {
    shared: Arc<Shared>,
    results: Inbox<Loaded>,
    generation: u64,
}

fn load(path: &Path) -> Result<Opened, Failure> {
    let (kind, bytes) = bande_content::open(path)?;
    if kind == Kind::Pdf {
        return Ok(Opened::Document);
    }
    let mut decoded = bande_content::decode(&bytes, kind)?;
    decoded.facts.kind = Some(kind);
    Ok(Opened::Image(Box::new(decoded)))
}

fn work(shared: &Shared, results: &Outbox<Loaded>) {
    while let Some((generation, path)) = shared.next() {
        let result = load(&path);
        if !results.send(Loaded {
            generation,
            path,
            result,
        }) {
            return;
        }
    }
}

impl Loader {
    pub fn new() -> io::Result<Self> {
        let results = Inbox::new()?;
        let shared = Arc::new(Shared::default());
        let (worker, outbox) = (Arc::clone(&shared), results.outbox());
        thread::Builder::new()
            .name("decode".to_owned())
            .spawn(move || work(&worker, &outbox))?;
        Ok(Self {
            shared,
            results,
            generation: 0,
        })
    }

    pub fn load(&mut self, path: &Path) -> u64 {
        self.generation += 1;
        self.shared.lock().wanted = Some((self.generation, path.to_path_buf()));
        self.shared.ready.notify_all();
        self.generation
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.results.fd()
    }

    pub fn take(&mut self) -> Option<Loaded> {
        let generation = self.generation;
        self.results
            .drain()
            .into_iter()
            .rfind(|loaded| loaded.generation == generation)
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.ready.notify_all();
    }
}
