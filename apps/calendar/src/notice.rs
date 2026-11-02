use std::collections::HashMap;
use std::os::fd::BorrowedFd;

use bande_datetime::DateFormats;
use bande_inbox::{Inbox, Outbox};
use jiff::tz::TimeZone;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

use crate::alerts::Due;
use crate::ui::L10N;

const APP_ID: &str = "org.bande.Calendar";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NO_TIMEOUT: i32 = -1;
pub const OPEN: &str = "default";
pub const SNOOZE: &str = "snooze";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Shown(u32, Due),
    Invoked(u32, String),
    Closed(u32),
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

fn show(due: &Due, title: &str, body: &str) -> zbus::Result<u32> {
    let hints: HashMap<&str, Value> = HashMap::from([
        ("desktop-entry", Value::from(APP_ID)),
        ("category", Value::from("x-calendar.alert")),
    ]);
    let snooze = L10N.tr("snooze");
    let actions = [OPEN, "", SNOOZE, snooze.as_str()];
    Connection::session()?
        .call_method(
            Some(NOTIFICATIONS),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS),
            "Notify",
            &(
                L10N.tr("app-name"),
                0u32,
                APP_ID,
                title,
                body,
                actions.as_slice(),
                hints,
                NO_TIMEOUT,
            ),
        )?
        .body()
        .deserialize::<u32>()
        .inspect(|_| tracing::debug!("alert for {}", due.title))
}

impl Notices {
    pub fn start() -> std::io::Result<Self> {
        let inbox = Inbox::new()?;
        let outbox = inbox.outbox();
        std::thread::Builder::new()
            .name("notices".to_owned())
            .spawn(move || {
                if let Err(error) = listen(&outbox) {
                    tracing::warn!("cannot follow alert notifications: {error}");
                }
            })?;
        Ok(Self { inbox })
    }

    pub fn notify(&self, due: Due, zone: &TimeZone, formats: &DateFormats) {
        let starts = due.starts.to_zoned(zone.clone());
        let when = match due.all_day {
            true => formats.date(&starts, false),
            false => formats.clock(&starts),
        };
        let body = match due.location.is_empty() {
            true => when,
            false => format!("{when}\n{}", due.location),
        };
        let outbox = self.inbox.outbox();
        let spawned =
            std::thread::Builder::new()
                .name("alert".to_owned())
                .spawn(move || match show(&due, &due.title, &body) {
                    Ok(id) => {
                        outbox.send(Notice::Shown(id, due));
                    }
                    Err(error) => tracing::warn!("cannot show an alert: {error}"),
                });
        if let Err(error) = spawned {
            tracing::warn!("cannot show an alert: {error}");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn take(&mut self) -> Vec<Notice> {
        self.inbox.drain()
    }
}
