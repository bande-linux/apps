use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Asked<T> {
    ask: Sender<()>,
    latest: Arc<Mutex<T>>,
}

impl<T: Clone + Default + Send + 'static> Asked<T> {
    pub fn start(mut read: impl FnMut() -> Option<T> + Send + 'static) -> Self {
        let (ask, asked): (Sender<()>, Receiver<()>) = mpsc::channel();
        let latest = Arc::new(Mutex::new(T::default()));
        let shared = Arc::clone(&latest);
        thread::spawn(move || {
            while asked.recv().is_ok() {
                while asked.try_recv().is_ok() {}
                if let Some(value) = read()
                    && let Ok(mut latest) = shared.lock()
                {
                    *latest = value;
                }
            }
        });
        Self { ask, latest }
    }

    pub fn latest(&self) -> T {
        let _ = self.ask.send(());
        self.latest
            .lock()
            .map(|latest| latest.clone())
            .unwrap_or_default()
    }
}
