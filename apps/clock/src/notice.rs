use std::collections::HashMap;
use std::os::fd::BorrowedFd;

use bande_l10n::tr;
use bande_inbox::{Inbox, Outbox};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

const APP_ID: &str = "org.bande.Clock";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NO_TIMEOUT: i32 = -1;
const CRITICAL: u8 = 2;
pub const OPEN: &str = "default";
pub const SNOOZE: &str = "snooze";
pub const STOP: &str = "stop";
pub const REPEAT: &str = "repeat";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Alarm(u64),
    Timer(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Shown(u32, Subject),
    Invoked(u32, String),
    Closed(u32),
}

pub struct Ring {
    pub subject: Subject,
    pub title: String,
    pub body: String,
    pub sound: String,
    pub snooze: bool,
}

pub fn sound_name(keyword: &str) -> String {
    format!("alert-{keyword}")
}

pub struct Notices {
    inbox: Inbox<Notice>,
}

fn listen(outbox: &Outbox<Notice>) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(NOTIFICATIONS)?
        .path(NOTIFICATIONS_PATH)?
        .build();
    for signal in MessageIterator::for_match_rule(rule, &connection, None)?.flatten() {
        let header = signal.header();
        let body = signal.body();
        let notice = match header.member().map(|member| member.as_str()) {
            Some("ActionInvoked") => body
                .deserialize::<(u32, String)>()
                .ok()
                .map(|(id, key)| Notice::Invoked(id, key)),
            Some("NotificationClosed") => body
                .deserialize::<(u32, u32)>()
                .ok()
                .map(|(id, _)| Notice::Closed(id)),
            _ => None,
        };
        if let Some(notice) = notice
            && !outbox.send(notice)
        {
            break;
        }
    }
    Ok(())
}

fn show(ring: &Ring) -> zbus::Result<u32> {
    let (second, second_label) = match ring.subject {
        Subject::Alarm(_) if ring.snooze => (SNOOZE, tr!("Snooze")),
        Subject::Alarm(_) => (STOP, tr!("Stop")),
        Subject::Timer(_) => (REPEAT, tr!("Repeat")),
    };
    let stop = tr!("Stop");
    let mut actions = vec![OPEN, "", second, second_label.as_str()];
    if second != STOP {
        actions.extend([STOP, stop.as_str()]);
    }
    let hints: HashMap<&str, Value> = HashMap::from([
        ("desktop-entry", Value::from(APP_ID)),
        ("category", Value::from("x-clock.alarm")),
        ("urgency", Value::from(CRITICAL)),
        ("sound-name", Value::from(sound_name(&ring.sound))),
    ]);
    Connection::session()?
        .call_method(
            Some(NOTIFICATIONS),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS),
            "Notify",
            &(
                tr!("Clock"),
                0u32,
                APP_ID,
                ring.title.as_str(),
                ring.body.as_str(),
                actions,
                hints,
                NO_TIMEOUT,
            ),
        )?
        .body()
        .deserialize::<u32>()
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

impl Notices {
    pub fn start() -> std::io::Result<Self> {
        let inbox = Inbox::new()?;
        let outbox = inbox.outbox();
        std::thread::Builder::new()
            .name("notices".to_owned())
            .spawn(move || {
                if let Err(error) = listen(&outbox) {
                    tracing::warn!("cannot follow Clock's notifications: {error}");
                }
            })?;
        Ok(Self { inbox })
    }

    pub fn ring(&self, ring: Ring) {
        let outbox = self.inbox.outbox();
        in_background("ring", move || match show(&ring) {
            Ok(id) => {
                tracing::info!("rang {:?}", ring.subject);
                outbox.send(Notice::Shown(id, ring.subject));
            }
            Err(error) => tracing::warn!("cannot ring {:?}: {error}", ring.subject),
        });
    }

    pub fn withdraw(&self, id: u32) {
        in_background("withdraw", move || {
            let closed = Connection::session().and_then(|connection| {
                connection.call_method(
                    Some(NOTIFICATIONS),
                    NOTIFICATIONS_PATH,
                    Some(NOTIFICATIONS),
                    "CloseNotification",
                    &id,
                )
            });
            if let Err(error) = closed {
                tracing::debug!("cannot withdraw a notification: {error}");
            }
        });
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn take(&mut self) -> Vec<Notice> {
        self.inbox.drain()
    }
}
