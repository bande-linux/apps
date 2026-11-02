use std::sync::mpsc::{self, Receiver, Sender};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

pub const OBJECT_PATH: &str = "/org/bande/Clock";
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const WAKE_UNIT: &str = "bande-clock-wake";
const WAKE_ACCURACY_USEC: u64 = 1_000_000;
const RTC_CLASS: &str = "/sys/class/rtc";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub timers: Vec<(String, i64)>,
    pub next_alarm: (String, i64),
}

struct Published {
    summary: Summary,
}

#[zbus::interface(name = "org.bande.Clock")]
impl Published {
    #[zbus(property)]
    fn timers(&self) -> Vec<(String, i64)> {
        self.summary.timers.clone()
    }

    #[zbus(property)]
    fn next_alarm(&self) -> (String, i64) {
        self.summary.next_alarm.clone()
    }
}

pub struct Bus {
    connection: Connection,
    shown: Summary,
}

impl Bus {
    pub fn serve(connection: &Connection) -> zbus::Result<Self> {
        connection.object_server().at(
            OBJECT_PATH,
            Published {
                summary: Summary::default(),
            },
        )?;
        Ok(Self {
            connection: connection.clone(),
            shown: Summary::default(),
        })
    }

    pub fn publish(&mut self, summary: Summary) {
        if summary == self.shown {
            return;
        }
        let published = self
            .connection
            .object_server()
            .interface::<_, Published>(OBJECT_PATH)
            .and_then(|interface| {
                let emitter = interface.signal_emitter();
                let (timers, alarm) = (
                    summary.timers != self.shown.timers,
                    summary.next_alarm != self.shown.next_alarm,
                );
                interface.get_mut().summary = summary.clone();
                let published = interface.get();
                if timers {
                    zbus::block_on(published.timers_changed(emitter))?;
                }
                if alarm {
                    zbus::block_on(published.next_alarm_changed(emitter))?;
                }
                Ok(())
            });
        match published {
            Ok(()) => self.shown = summary,
            Err(error) => tracing::warn!("cannot tell the shell about Clock: {error}"),
        }
    }
}

pub struct Waker {
    wanted: Option<Timestamp>,
    sender: Sender<Option<Timestamp>>,
}

fn calendar_time(at: Timestamp) -> String {
    at.to_zoned(TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S UTC")
        .to_string()
}

fn rtc_wakes() -> bool {
    std::fs::read_dir(RTC_CLASS).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().join("wakealarm").exists())
    })
}

fn schedule(
    connection: &Connection,
    at: Timestamp,
    count: u64,
    wakes: bool,
) -> zbus::Result<String> {
    let name = format!("{WAKE_UNIT}-{}-{count}", std::process::id());
    let program = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/usr/bin/bande-clock".to_owned());
    let timer: Vec<(&str, Value)> = vec![
        ("Description", Value::from("Wake the computer for Clock")),
        (
            "TimersCalendar",
            Value::from(vec![("OnCalendar", calendar_time(at))]),
        ),
        ("WakeSystem", Value::from(wakes)),
        ("RemainAfterElapse", Value::from(false)),
        ("AccuracyUSec", Value::from(WAKE_ACCURACY_USEC)),
    ];
    let service: Vec<(&str, Value)> = vec![
        ("Description", Value::from("Clock's alarms and timers")),
        (
            "ExecStart",
            Value::from(vec![(
                program.clone(),
                vec![program.clone(), "--background".to_owned()],
                false,
            )]),
        ),
    ];
    let timer_unit = format!("{name}.timer");
    connection.call_method(
        Some(SYSTEMD),
        SYSTEMD_PATH,
        Some(MANAGER),
        "StartTransientUnit",
        &(
            timer_unit.as_str(),
            "replace",
            timer,
            vec![(format!("{name}.service"), service)],
        ),
    )?;
    Ok(timer_unit)
}

fn stop(connection: &Connection, unit: &str) {
    let stopped = connection
        .call_method(
            Some(SYSTEMD),
            SYSTEMD_PATH,
            Some(MANAGER),
            "StopUnit",
            &(unit, "replace"),
        )
        .and_then(|reply| reply.body().deserialize::<OwnedObjectPath>());
    if let Err(error) = stopped {
        tracing::debug!("cannot stop {unit}: {error}");
    }
    let reset = connection.call_method(
        Some(SYSTEMD),
        SYSTEMD_PATH,
        Some(MANAGER),
        "ResetFailedUnit",
        &(unit,),
    );
    if let Err(error) = reset {
        tracing::debug!("cannot forget {unit}: {error}");
    }
}

fn follow(wanted: &Receiver<Option<Timestamp>>) {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!("cannot wake the computer for Clock: {error}");
            return;
        }
    };
    let wakes = rtc_wakes();
    tracing::debug!("the wake-up timer wakes the computer: {wakes}");
    let mut scheduled: Option<String> = None;
    let mut count = 0;
    while let Ok(mut at) = wanted.recv() {
        while let Ok(newer) = wanted.try_recv() {
            at = newer;
        }
        if let Some(unit) = scheduled.take() {
            stop(&connection, &unit);
        }
        let Some(at) = at else {
            continue;
        };
        count += 1;
        match schedule(&connection, at, count, wakes) {
            Ok(unit) => scheduled = Some(unit),
            Err(error) => tracing::warn!("cannot set a wake-up timer for Clock: {error}"),
        }
    }
    if let Some(unit) = scheduled {
        stop(&connection, &unit);
    }
}

impl Waker {
    pub fn start() -> std::io::Result<Self> {
        let (sender, wanted) = mpsc::channel();
        std::thread::Builder::new()
            .name("wake".to_owned())
            .spawn(move || follow(&wanted))?;
        Ok(Self {
            wanted: None,
            sender,
        })
    }

    pub fn wake_at(&mut self, at: Option<Timestamp>) {
        if at == self.wanted {
            return;
        }
        self.wanted = at;
        if self.sender.send(at).is_err() {
            tracing::warn!("cannot wake the computer for Clock");
        }
    }
}
