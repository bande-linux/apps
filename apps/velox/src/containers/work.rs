use std::collections::HashMap;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, PoisonError};

use bande_xdg::spawn_detached;

use super::compose::{Compose, Verb};
use super::inspect::{self, Inspect};
use super::logs::LogsTarget;
use super::stats::{Raw, Sample};
use crate::engine::{Engine, Error, Informer, Update, Updates};

const TERMINAL: &str = "bande-terminal";
const TERMINAL_ID: &str = "org.bande.Terminal";

#[derive(Debug)]
pub enum Event {
    Inspected(String, Box<Inspect>),
    Sampled(String, Sample),
    Copied(String),
    Composed {
        project: String,
        verb: Verb,
        failure: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Inspect(Engine, String),
    CopyRun(Engine, String),
    Compose(Compose, Verb),
    Terminal(Vec<String>),
    Logs(LogsTarget),
    Watch(Option<Engine>, Vec<String>),
}

#[derive(Default)]
struct Held {
    stopped: bool,
    socket: Option<UnixStream>,
}

struct Stream(Arc<Mutex<Held>>);

impl Drop for Stream {
    fn drop(&mut self) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.stopped = true;
        if let Some(socket) = held.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start {name}: {error}");
    }
}

fn sample(engine: &Engine, id: &str, held: &Mutex<Held>, out: &Updates) -> Result<(), Error> {
    let (socket, body) = engine.stream(&format!("/containers/{id}/stats?stream=1"))?;
    {
        let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.stopped {
            return Ok(());
        }
        held.socket = Some(socket);
    }
    for raw in serde_json::Deserializer::from_reader(body).into_iter::<Raw>() {
        let sampled = Event::Sampled(id.to_owned(), raw?.sample());
        if !out.send(Update::Containers(sampled)) {
            break;
        }
    }
    Ok(())
}

pub struct Work {
    updates: Updates,
    streams: HashMap<String, Stream>,
}

impl Work {
    pub fn new(informer: &Informer) -> Self {
        Self {
            updates: informer.updates(),
            streams: HashMap::new(),
        }
    }

    pub fn serve(
        &mut self,
        request: Request,
        token: impl FnOnce() -> Option<String>,
    ) -> Option<LogsTarget> {
        let out = self.updates.clone();
        match request {
            Request::Logs(target) => return Some(target),
            Request::Inspect(engine, id) => {
                spawn("velox-inspect", move || {
                    match inspect::inspect(&engine, &id) {
                        Ok(found) => {
                            out.send(Update::Containers(Event::Inspected(id, Box::new(found))));
                        }
                        Err(error) => tracing::info!("cannot inspect {id}: {error}"),
                    }
                })
            }
            Request::CopyRun(engine, id) => spawn("velox-copy", move || {
                match inspect::inspect(&engine, &id).and_then(|found| found.run_command(&engine)) {
                    Ok(command) => {
                        out.send(Update::Containers(Event::Copied(command)));
                    }
                    Err(error) => tracing::warn!("cannot describe {id}: {error}"),
                }
            }),
            Request::Compose(compose, verb) => spawn("velox-compose", move || {
                let failure = compose.run(verb).err();
                out.send(Update::Containers(Event::Composed {
                    project: compose.project,
                    verb,
                    failure,
                }));
            }),
            Request::Terminal(command) => {
                let token = token();
                let mut argv = vec![TERMINAL.to_owned(), "-e".to_owned()];
                argv.extend(command);
                if let Err(error) = spawn_detached(TERMINAL_ID, &argv, None, token.as_deref()) {
                    tracing::warn!("cannot open Terminal: {error}");
                }
            }
            Request::Watch(engine, ids) => self.watch(engine, ids),
        }
        None
    }

    fn watch(&mut self, engine: Option<Engine>, ids: Vec<String>) {
        self.streams.retain(|id, _| ids.contains(id));
        let Some(engine) = engine else {
            return;
        };
        for id in ids {
            if self.streams.contains_key(&id) {
                continue;
            }
            let held = Arc::new(Mutex::new(Held::default()));
            let shared = Arc::clone(&held);
            let (engine, out, key) = (engine.clone(), self.updates.clone(), id.clone());
            spawn("velox-stats", move || {
                if let Err(error) = sample(&engine, &key, &shared, &out)
                    && !shared
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .stopped
                {
                    tracing::info!("the stats of {key} ended: {error}");
                }
            });
            self.streams.insert(id, Stream(held));
        }
    }
}
