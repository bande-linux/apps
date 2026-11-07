use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_mail::portal::choose_files;

pub const MAX_ATTACHMENT: u64 = 25 << 20;

pub enum Picked {
    File(PathBuf, Vec<u8>),
    TooLarge(PathBuf),
}

pub struct Picker {
    sender: Sender<(u64, Vec<Picked>)>,
    received: Receiver<(u64, Vec<Picked>)>,
    awake: UnixStream,
    wake: Arc<UnixStream>,
}

impl Picker {
    pub fn new() -> std::io::Result<Self> {
        let (awake, wake) = UnixStream::pair()?;
        awake.set_nonblocking(true)?;
        let (sender, received) = mpsc::channel();
        Ok(Self {
            sender,
            received,
            awake,
            wake: Arc::new(wake),
        })
    }

    pub fn pick(&self, token: u64, title: String) {
        self.gather(token, move || {
            choose_files(&title)
                .inspect_err(|error| tracing::warn!("cannot choose files: {error}"))
                .unwrap_or_default()
        });
    }

    pub fn read(&self, token: u64, paths: Vec<PathBuf>) {
        self.gather(token, move || paths);
    }

    fn gather(&self, token: u64, paths: impl FnOnce() -> Vec<PathBuf> + Send + 'static) {
        let (sender, wake) = (self.sender.clone(), Arc::clone(&self.wake));
        let spawned = std::thread::Builder::new()
            .name("attach".to_owned())
            .spawn(move || {
                let picked = paths()
                    .into_iter()
                    .filter_map(|path| {
                        let size = std::fs::metadata(&path).ok()?.len();
                        if size > MAX_ATTACHMENT {
                            return Some(Picked::TooLarge(path));
                        }
                        let mut data = Vec::new();
                        std::fs::File::open(&path)
                            .and_then(|mut file| file.read_to_end(&mut data))
                            .inspect_err(|error| {
                                tracing::warn!("cannot read {}: {error}", path.display())
                            })
                            .ok()?;
                        Some(Picked::File(path, data))
                    })
                    .collect();
                if sender.send((token, picked)).is_ok() {
                    let _ = (&*wake).write(&[1]);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot read attachments: {error}");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.awake.as_fd()
    }

    pub fn take(&mut self) -> Vec<(u64, Vec<Picked>)> {
        let mut buffer = [0u8; 64];
        while matches!((&self.awake).read(&mut buffer), Ok(count) if count > 0) {}
        self.received.try_iter().collect()
    }
}
