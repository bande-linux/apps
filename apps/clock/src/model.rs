use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp};
use rustix::time::{ClockId, clock_gettime};
use serde::{Deserialize, Serialize};

pub const PROGRAM: &str = "bande-clock";
const STATE_FILE: &str = "clock.json";
pub const LATE_LIMIT: SignedDuration = SignedDuration::from_hours(1);
pub const SNOOZE: SignedDuration = SignedDuration::from_mins(9);
pub const DEFAULT_SOUND: &str = "mallet";

pub fn boot_now() -> Duration {
    let now = clock_gettime(ClockId::Boottime);
    Duration::new(
        u64::try_from(now.tv_sec).unwrap_or(0),
        u32::try_from(now.tv_nsec).unwrap_or(0),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Days(pub u8);

impl Days {
    pub const ONCE: Self = Self(0);
    pub const WEEKDAYS: Self = Self(0b001_1111);
    pub const WEEKEND: Self = Self(0b110_0000);
    pub const EVERY_DAY: Self = Self(0b111_1111);

    fn bit(day: Weekday) -> u8 {
        1 << day.to_monday_zero_offset()
    }

    pub fn has(self, day: Weekday) -> bool {
        self.0 & Self::bit(day) != 0
    }

    pub fn toggled(self, day: Weekday) -> Self {
        Self((self.0 ^ Self::bit(day)) & Self::EVERY_DAY.0)
    }

    pub fn repeats(self) -> bool {
        self.0 != 0
    }

    fn allows(self, day: Date) -> bool {
        !self.repeats() || self.has(day.weekday())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alarm {
    pub id: u64,
    pub hour: i8,
    pub minute: i8,
    pub days: Days,
    pub label: String,
    pub sound: String,
    pub snooze: bool,
    pub enabled: bool,
    #[serde(default)]
    pub snoozed: Option<Timestamp>,
}

impl Alarm {
    pub fn new(id: u64, hour: i8, minute: i8) -> Self {
        Self {
            id,
            hour,
            minute,
            days: Days::ONCE,
            label: String::new(),
            sound: DEFAULT_SOUND.to_owned(),
            snooze: true,
            enabled: true,
            snoozed: None,
        }
    }

    pub fn next_after(&self, after: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
        let today = after.to_zoned(zone.clone()).date();
        (0..=7)
            .filter_map(|offset| today.checked_add(Span::new().days(offset)).ok())
            .filter(|day| self.days.allows(*day))
            .filter_map(|day| {
                day.at(self.hour, self.minute, 0, 0)
                    .to_zoned(zone.clone())
                    .ok()
            })
            .map(|at| at.timestamp())
            .find(|at| *at > after)
    }

    pub fn coming(&self, after: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
        let next = self.enabled.then(|| self.next_after(after, zone)).flatten();
        match (next, self.snoozed) {
            (Some(next), Some(snoozed)) => Some(next.min(snoozed)),
            (next, snoozed) => next.or(snoozed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerState {
    Idle,
    Running { ends: Duration },
    Paused { left: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer {
    pub id: u64,
    pub label: String,
    pub length: Duration,
    pub sound: String,
    pub state: TimerState,
}

impl Timer {
    pub fn left(&self, boot: Duration) -> Duration {
        match self.state {
            TimerState::Idle => self.length,
            TimerState::Running { ends } => ends.saturating_sub(boot),
            TimerState::Paused { left } => left,
        }
    }

    pub fn start(&mut self, boot: Duration) {
        self.state = TimerState::Running {
            ends: boot + self.left(boot),
        };
    }

    pub fn pause(&mut self, boot: Duration) {
        self.state = TimerState::Paused {
            left: self.left(boot),
        };
    }

    pub fn ends(&self) -> Option<Duration> {
        match self.state {
            TimerState::Running { ends } => Some(ends),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stopwatch {
    pub started: Option<Duration>,
    pub banked: Duration,
    pub laps: Vec<Duration>,
}

impl Stopwatch {
    pub fn elapsed(&self, boot: Duration) -> Duration {
        self.banked
            + self
                .started
                .map_or(Duration::ZERO, |at| boot.saturating_sub(at))
    }

    pub fn toggle(&mut self, boot: Duration) {
        match self.started.take() {
            Some(at) => self.banked += boot.saturating_sub(at),
            None => self.started = Some(boot),
        }
    }

    pub fn lap(&mut self, boot: Duration) {
        self.laps.push(self.elapsed(boot));
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedState {
    Idle,
    Running { ends: Timestamp },
    Paused { left_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedTimer {
    id: u64,
    label: String,
    seconds: u64,
    sound: String,
    state: SavedState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    alarms: Vec<Alarm>,
    timers: Vec<SavedTimer>,
    cities: Vec<String>,
    checked: Option<Timestamp>,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    Alarm(Alarm),
    Timer(Timer),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clock {
    pub alarms: Vec<Alarm>,
    pub timers: Vec<Timer>,
    pub cities: Vec<String>,
    pub stopwatch: Stopwatch,
    pub checked: Option<Timestamp>,
    next_id: u64,
}

fn wall_to_boot(at: Timestamp, now: Timestamp, boot: Duration) -> Duration {
    let ahead = at.duration_since(now);
    match ahead.is_negative() {
        true => boot.saturating_sub(ahead.unsigned_abs()),
        false => boot + ahead.unsigned_abs(),
    }
}

fn boot_to_wall(at: Duration, now: Timestamp, boot: Duration) -> Timestamp {
    SignedDuration::try_from(at.saturating_sub(boot))
        .ok()
        .and_then(|ahead| now.checked_add(ahead).ok())
        .unwrap_or(now)
}

pub fn state_file() -> PathBuf {
    bande_xdg::data_home()
        .unwrap_or_else(std::env::temp_dir)
        .join(PROGRAM)
        .join(STATE_FILE)
}

impl Clock {
    pub fn load(file: &Path) -> Self {
        let saved: Saved = std::fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let (now, boot) = (Timestamp::now(), boot_now());
        let timers = saved
            .timers
            .into_iter()
            .map(|timer| Timer {
                id: timer.id,
                label: timer.label,
                length: Duration::from_secs(timer.seconds),
                sound: timer.sound,
                state: match timer.state {
                    SavedState::Idle => TimerState::Idle,
                    SavedState::Running { ends } => TimerState::Running {
                        ends: wall_to_boot(ends, now, boot),
                    },
                    SavedState::Paused { left_ms } => TimerState::Paused {
                        left: Duration::from_millis(left_ms),
                    },
                },
            })
            .collect();
        Self {
            alarms: saved.alarms,
            timers,
            cities: saved.cities,
            stopwatch: Stopwatch::default(),
            checked: saved.checked,
            next_id: saved.next_id,
        }
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let (now, boot) = (Timestamp::now(), boot_now());
        let saved = Saved {
            alarms: self.alarms.clone(),
            timers: self
                .timers
                .iter()
                .map(|timer| SavedTimer {
                    id: timer.id,
                    label: timer.label.clone(),
                    seconds: timer.length.as_secs(),
                    sound: timer.sound.clone(),
                    state: match timer.state {
                        TimerState::Idle => SavedState::Idle,
                        TimerState::Running { ends } => SavedState::Running {
                            ends: boot_to_wall(ends, now, boot),
                        },
                        TimerState::Paused { left } => SavedState::Paused {
                            left_ms: u64::try_from(left.as_millis()).unwrap_or(u64::MAX),
                        },
                    },
                })
                .collect(),
            cities: self.cities.clone(),
            checked: self.checked,
            next_id: self.next_id,
        };
        let bytes = serde_json::to_vec_pretty(&saved).map_err(std::io::Error::other)?;
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let staging = file.with_extension("json.new");
        std::fs::write(&staging, bytes)?;
        std::fs::rename(&staging, file)
    }

    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn keeps_running(&self) -> bool {
        self.alarms
            .iter()
            .any(|alarm| alarm.enabled || alarm.snoozed.is_some())
            || self.timers.iter().any(|timer| timer.ends().is_some())
    }

    pub fn take_due_alarms(&mut self, now: Timestamp, zone: &TimeZone) -> Vec<Alarm> {
        let late = now.checked_sub(LATE_LIMIT).unwrap_or(now);
        let from = self.checked.map_or(now, |checked| checked.max(late));
        let mut due = Vec::new();
        for alarm in &mut self.alarms {
            let snoozed = alarm.snoozed.filter(|at| *at <= now);
            if snoozed.is_some() {
                alarm.snoozed = None;
            }
            let rings = alarm.enabled
                && alarm
                    .next_after(from.min(now), zone)
                    .is_some_and(|at| at <= now);
            if rings && !alarm.days.repeats() {
                alarm.enabled = false;
            }
            if rings || snoozed.is_some_and(|at| at >= late) {
                due.push(alarm.clone());
            }
        }
        self.checked = Some(now);
        due
    }

    pub fn take_due_timers(&mut self, boot: Duration) -> Vec<Timer> {
        let mut due = Vec::new();
        for timer in &mut self.timers {
            if timer.ends().is_some_and(|ends| ends <= boot) {
                timer.state = TimerState::Idle;
                due.push(timer.clone());
            }
        }
        due
    }

    pub fn next_alarm(&self, now: Timestamp, zone: &TimeZone) -> Option<(Timestamp, &Alarm)> {
        self.alarms
            .iter()
            .filter_map(|alarm| Some((alarm.coming(now, zone)?, alarm)))
            .min_by_key(|(at, _)| *at)
    }

    pub fn next_timer(&self) -> Option<Duration> {
        self.timers.iter().filter_map(Timer::ends).min()
    }

    pub fn wake_at(&self, now: Timestamp, boot: Duration, zone: &TimeZone) -> Option<Timestamp> {
        let alarm = self.next_alarm(now, zone).map(|(at, _)| at);
        let timer = self.next_timer().map(|ends| boot_to_wall(ends, now, boot));
        alarm.into_iter().chain(timer).min()
    }
}
