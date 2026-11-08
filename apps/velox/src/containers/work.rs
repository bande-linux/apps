use std::rc::Rc;

use bande_xdg::spawn_detached;

use super::compose::{Compose, Verb};
use super::inspect::{self, Inspect};
use super::logs::LogsTarget;
use crate::engine::{Engine, Informer, Update};

const TERMINAL: &str = "bande-terminal";
const WATCHER: &str = "containers";

#[derive(Debug)]
pub enum Event {
    Inspected(String, Box<Inspect>),
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
    Shell(crate::shell::ShellTarget),
    Logs(LogsTarget),
    Watch(Engine, Vec<String>),
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start {name}: {error}");
    }
}

pub struct Work {
    informer: Rc<Informer>,
}

impl Work {
    pub fn new(informer: Rc<Informer>) -> Self {
        Self { informer }
    }

    pub fn serve(
        &mut self,
        request: Request,
        token: impl FnOnce() -> Option<String>,
    ) -> Option<LogsTarget> {
        let out = self.informer.updates();
        match request {
            Request::Logs(target) => return Some(target),
            Request::Shell(_) => {}
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
                if let Err(error) =
                    spawn_detached(crate::TERMINAL_APP_ID, &argv, None, token.as_deref())
                {
                    tracing::warn!("cannot open Terminal: {error}");
                }
            }
            Request::Watch(engine, ids) => self.informer.watch(WATCHER, &engine, ids),
        }
        None
    }
}
