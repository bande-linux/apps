use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use unbound_notify::Urgency;
use unbound_notify::client::{Request, notify};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

const LAUNCHER_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const LAUNCHER_PATH: &str = "/org/unbound/Terminal";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Badge(u32),
    Notify { summary: String, body: String },
}

pub struct Bus {
    sender: Option<Sender<Outgoing>>,
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
        }
    }

    pub fn send(&self, outgoing: Outgoing) {
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
            Outgoing::Notify { summary, body } => notify(
                &connection,
                &Request {
                    app_name: &unbound_term::L10N.tr("title"),
                    replaces_id: 0,
                    app_icon: app_id,
                    summary: &summary,
                    body: &body,
                    actions: &[],
                    urgency: Urgency::Normal,
                    sound: None,
                },
            )
            .map(|_| ()),
        };
        if let Err(error) = result {
            tracing::warn!("cannot reach the shell: {error}");
        }
    }
}
