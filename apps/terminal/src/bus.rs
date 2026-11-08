use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use bande_notify::Urgency;
use bande_notify::client::{Request, notify};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

const LAUNCHER_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const LAUNCHER_PATH: &str = "/org/bande/Terminal";
const BELL: &str = "bell";
const BELL_GAP: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outgoing {
    Badge(u32),
    Notify { summary: String, body: String },
    Bell,
}

pub struct Bus {
    sender: Option<Sender<Outgoing>>,
    badges: RefCell<HashMap<u32, u32>>,
    rung: Cell<Option<Instant>>,
}

impl Bus {
    pub fn start(app_id: &'static str) -> Self {
        let (sender, receiver) = mpsc::channel();
        let started = thread::Builder::new()
            .name("terminal-bus".to_owned())
            .spawn(move || serve(app_id, &receiver));
        if let Err(error) = &started {
            tracing::warn!("cannot start the session bus thread: {error}");
        }
        Self {
            sender: started.ok().map(|_| sender),
            badges: RefCell::new(HashMap::new()),
            rung: Cell::new(None),
        }
    }

    pub fn set_badge(&self, window: u32, count: u32) {
        let total = {
            let mut badges = self.badges.borrow_mut();
            let before: u32 = badges.values().sum();
            match count {
                0 => badges.remove(&window),
                count => badges.insert(window, count),
            };
            let total: u32 = badges.values().sum();
            (total != before).then_some(total)
        };
        if let Some(total) = total {
            self.send(Outgoing::Badge(total));
        }
    }

    pub fn notify(&self, summary: String, body: String) {
        self.send(Outgoing::Notify { summary, body });
    }

    pub fn ring(&self) {
        let now = Instant::now();
        if self
            .rung
            .get()
            .is_none_or(|rung| now.duration_since(rung) >= BELL_GAP)
        {
            self.rung.set(Some(now));
            self.send(Outgoing::Bell);
        }
    }

    fn send(&self, outgoing: Outgoing) {
        if let Some(sender) = &self.sender
            && sender.send(outgoing).is_err()
        {
            tracing::warn!("the session bus thread has stopped");
        }
    }
}

fn serve(app_id: &str, receiver: &Receiver<Outgoing>) {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => return tracing::warn!("cannot reach the session bus: {error}"),
    };
    let uri = format!("application://{app_id}.desktop");
    for outgoing in receiver {
        let result = match outgoing {
            Outgoing::Badge(count) => {
                let properties = HashMap::from([
                    ("count", Value::from(i64::from(count))),
                    ("count-visible", Value::from(count > 0)),
                    ("urgent", Value::from(count > 0)),
                ]);
                connection
                    .emit_signal(
                        None::<&str>,
                        LAUNCHER_PATH,
                        LAUNCHER_INTERFACE,
                        "Update",
                        &(uri.as_str(), properties),
                    )
                    .map(|_| ())
            }
            Outgoing::Bell => {
                bande_sound::play(&connection, BELL);
                Ok(())
            }
            Outgoing::Notify { summary, body } => notify(
                &connection,
                &Request {
                    app_name: &bande_terminal::L10N.tr("title"),
                    replaces_id: 0,
                    app_icon: app_id,
                    summary: &summary,
                    body: &body,
                    actions: &[],
                    urgency: Urgency::Normal,
                    sound: None,
                    desktop_entry: Some(app_id),
                },
            )
            .map(|_| ()),
        };
        if let Err(error) = result {
            tracing::warn!("cannot reach the shell: {error}");
        }
    }
}
