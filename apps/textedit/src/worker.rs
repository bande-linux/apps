use std::path::PathBuf;

use crate::document::Loaded;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    Unmappable,
    Io(String),
}

pub enum Message {
    Loaded {
        token: u64,
        loaded: Result<Loaded, String>,
    },
    Written {
        token: u64,
        revision: u64,
        path: PathBuf,
        outcome: Result<(), WriteError>,
    },
    Counted {
        token: u64,
        revision: u64,
        words: usize,
        characters: usize,
    },
    Chosen(Vec<PathBuf>),
    SaveTo {
        token: u64,
        path: Option<PathBuf>,
    },
}

pub fn spawn(name: &str, job: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.to_owned()).spawn(job) {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}
