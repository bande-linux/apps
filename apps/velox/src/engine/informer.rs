use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bande_watch::{Changes, FolderWatch};

use super::client::{Container, Engine, EngineInfo, Event};
use super::jobs::{self, Task};
use super::resources::{DiskUsage, Image, Network, Pulling, Sample, Volume};
use super::socket;
use crate::readiness::{self, Probe, Readiness};

const REMOVED: &str = "destroy";
const IGNORED_PREFIX: &str = "exec_";
const CONTAINER_ATTRIBUTE: &str = "container";
const RETRY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Pause,
    Resume,
    Kill,
    Remove,
}

#[derive(Debug)]
pub enum Update {
    Blocked(Readiness),
    Ready(EngineInfo),
    Listed(Vec<Container>),
    Changed(Container),
    Removed(String),
    Done {
        id: String,
        action: Action,
    },
    Containers(crate::containers::Event),
    Failed {
        id: String,
        action: Action,
        message: String,
    },
    Images(Vec<Image>),
    Volumes(Vec<Volume>),
    Networks(Vec<Network>),
    Usage(DiskUsage),
    Pulling(String, Pulling),
    Chosen(PathBuf),
    Finished(Task, Result<u64, String>),
    Stats(String, Sample),
}

#[derive(Clone)]
pub(crate) struct Updates {
    sender: Sender<Update>,
    wake: Arc<UnixStream>,
}

impl Updates {
    pub(crate) fn send(&self, update: Update) -> bool {
        self.sender.send(update).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

type Stream = Arc<Mutex<Option<UnixStream>>>;

#[derive(Default)]
struct Watch {
    stream: Stream,
    stopped: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        shut(&self.stream);
    }
}

#[derive(Default)]
struct Watches {
    owners: HashMap<&'static str, BTreeSet<String>>,
    streams: HashMap<String, Watch>,
}

pub struct Informer {
    updates: Receiver<Update>,
    wake: UnixStream,
    sender: Updates,
    recheck: Sender<()>,
    stopped: Arc<AtomicBool>,
    events: Stream,
    watches: RefCell<Watches>,
    _watch: Option<FolderWatch>,
}

fn shut(events: &Stream) {
    if let Some(stream) = events.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

fn follow(engine: &Engine, events: &Stream, stopped: &AtomicBool, out: &Updates) -> io::Result<()> {
    let (handle, body) = engine.events().map_err(io::Error::other)?;
    *events.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
    if stopped.load(Ordering::SeqCst) {
        return Ok(());
    }
    let listed = engine.containers().map_err(io::Error::other)?;
    if !out.send(Update::Listed(listed)) {
        return Ok(());
    }
    for kind in [Kind::Images, Kind::Volumes, Kind::Networks] {
        if !relist(engine, kind, out) {
            return Ok(());
        }
    }
    for event in serde_json::Deserializer::from_reader(body).into_iter::<Event>() {
        let event = event.map_err(io::Error::other)?;
        let Some(actor) = event.actor else {
            continue;
        };
        if event.action.starts_with(IGNORED_PREFIX) {
            continue;
        }
        let container = match (event.kind.as_str(), event.action.as_str()) {
            ("image", "push" | "save") | ("volume", "mount" | "unmount") => continue,
            ("image", _) => Err(Kind::Images),
            ("volume", _) => Err(Kind::Volumes),
            ("network", "connect" | "disconnect") => {
                match actor.attributes.get(CONTAINER_ATTRIBUTE) {
                    Some(id) => Ok(id.clone()),
                    None => continue,
                }
            }
            ("network", _) => Err(Kind::Networks),
            (_, REMOVED) => {
                if !out.send(Update::Removed(actor.id)) {
                    return Ok(());
                }
                continue;
            }
            _ => Ok(actor.id),
        };
        let sent = match container {
            Ok(id) => out.send(match engine.container(&id).map_err(io::Error::other)? {
                Some(container) => Update::Changed(container),
                None => Update::Removed(id),
            }),
            Err(kind) => relist(engine, kind, out),
        };
        if !sent {
            return Ok(());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Images,
    Volumes,
    Networks,
}

fn relist(engine: &Engine, kind: Kind, out: &Updates) -> bool {
    let listed = match kind {
        Kind::Images => engine.images().map(Update::Images),
        Kind::Volumes => engine.volumes().map(Update::Volumes),
        Kind::Networks => engine.networks().map(Update::Networks),
    };
    match listed {
        Ok(update) => out.send(update),
        Err(error) => {
            tracing::info!("cannot list the {kind:?}: {error}");
            true
        }
    }
}

fn run(
    candidates: &[PathBuf],
    recheck: &Receiver<()>,
    events: &Stream,
    stopped: &AtomicBool,
    out: &Updates,
) {
    while !stopped.load(Ordering::SeqCst) {
        while recheck.try_recv().is_ok() {}
        match readiness::probe(candidates) {
            Probe::Ready(info) => {
                let engine = Engine::new(&info.socket);
                if !out.send(Update::Ready(info)) {
                    return;
                }
                if let Err(error) = follow(&engine, events, stopped, out) {
                    tracing::info!("lost the engine's events: {error}");
                }
                shut(events);
                if recheck.recv_timeout(RETRY) == Err(RecvTimeoutError::Disconnected) {
                    return;
                }
            }
            Probe::Blocked(state) => {
                if !out.send(Update::Blocked(state)) || recheck.recv().is_err() {
                    return;
                }
            }
        }
    }
}

impl Informer {
    pub fn start() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let (sender, updates) = mpsc::channel();
        let out = Updates {
            sender,
            wake: Arc::new(writer),
        };
        let (recheck, rechecks) = mpsc::channel();
        let candidates = socket::candidates();
        let notify = recheck.clone();
        let watch = FolderWatch::spawn(Changes::Any, move |_| {
            let _ = notify.send(());
        })
        .inspect_err(|error| tracing::warn!("cannot watch for an engine: {error}"))
        .ok();
        if let Some(watch) = &watch {
            for folder in socket::watched_folders(&candidates) {
                if let Err(error) = watch.watch(&folder) {
                    tracing::debug!("cannot watch {}: {error}", folder.display());
                }
            }
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let events = Stream::default();
        let thread = (Arc::clone(&stopped), Arc::clone(&events), out.clone());
        std::thread::Builder::new()
            .name("velox-informer".to_owned())
            .spawn(move || {
                let (stopped, events, out) = thread;
                run(&candidates, &rechecks, &events, &stopped, &out);
            })?;
        Ok(Self {
            updates,
            wake: reader,
            sender: out,
            recheck,
            stopped,
            events,
            watches: RefCell::default(),
            _watch: watch,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<Update> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.updates.try_iter().collect()
    }

    pub(crate) fn updates(&self) -> Updates {
        self.sender.clone()
    }

    pub fn act(&self, engine: Engine, action: Action, ids: Vec<String>) {
        for id in ids {
            let out = self.sender.clone();
            let engine = engine.clone();
            let spawned = std::thread::Builder::new()
                .name("velox-action".to_owned())
                .spawn(move || {
                    let update = match engine.act(&id, action) {
                        Ok(()) => Update::Done { id, action },
                        Err(error) => Update::Failed {
                            id,
                            action,
                            message: error.to_string(),
                        },
                    };
                    out.send(update);
                });
            if let Err(error) = spawned {
                tracing::warn!("cannot reach the engine: {error}");
            }
        }
    }

    pub fn relist(&self, engine: Engine) {
        let out = self.sender.clone();
        let spawned = std::thread::Builder::new()
            .name("velox-list".to_owned())
            .spawn(move || match engine.containers() {
                Ok(containers) => {
                    out.send(Update::Listed(containers));
                }
                Err(error) => tracing::info!("cannot list the containers: {error}"),
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot reach the engine: {error}");
        }
    }
}

impl Informer {
    pub fn run(&self, engine: Engine, api: String, task: Task) {
        let out = self.sender.clone();
        let spawned = std::thread::Builder::new()
            .name("velox-task".to_owned())
            .spawn(move || jobs::perform(&engine, &api, task, |update| out.send(update)));
        if let Err(error) = spawned {
            tracing::warn!("cannot reach the engine: {error}");
        }
    }

    pub fn watch(&self, owner: &'static str, engine: &Engine, ids: Vec<String>) {
        let mut watches = self.watches.borrow_mut();
        let Watches { owners, streams } = &mut *watches;
        match ids.is_empty() {
            true => owners.remove(owner),
            false => owners.insert(owner, ids.into_iter().collect()),
        };
        let wanted: BTreeSet<&String> = owners.values().flatten().collect();
        streams.retain(|id, watch| wanted.contains(id) && !watch.ended.load(Ordering::SeqCst));
        for id in wanted {
            if !streams.contains_key(id) {
                streams.insert(id.clone(), self.stream_stats(engine.clone(), id.clone()));
            }
        }
    }

    fn stream_stats(&self, engine: Engine, id: String) -> Watch {
        let watch = Watch::default();
        let out = self.sender.clone();
        let (stream, stopped, ended) = (
            Arc::clone(&watch.stream),
            Arc::clone(&watch.stopped),
            Arc::clone(&watch.ended),
        );
        let spawned = std::thread::Builder::new()
            .name("velox-stats".to_owned())
            .spawn(move || {
                match engine.stats(&id) {
                    Ok((handle, samples)) => {
                        *stream.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
                        for sample in samples {
                            if stopped.load(Ordering::SeqCst)
                                || !out.send(Update::Stats(id.clone(), sample))
                            {
                                break;
                            }
                        }
                    }
                    Err(error) => tracing::info!("cannot follow the stats of {id}: {error}"),
                }
                ended.store(true, Ordering::SeqCst);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot reach the engine: {error}");
            watch.ended.store(true, Ordering::SeqCst);
        }
        watch
    }
}

impl Drop for Informer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        shut(&self.events);
        let _ = self.recheck.send(());
    }
}
