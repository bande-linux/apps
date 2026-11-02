use std::sync::OnceLock;
use std::time::Duration;

use bande_app_settings::{Stored, flag, pairs};
use bande_inbox::Outbox;
use jiff::civil::Weekday;

use crate::edit::Alert;

pub const WEEKDAYS: [Weekday; 7] = [
    Weekday::Sunday,
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
];

pub const HOURS_SHOWN: [u8; 5] = [6, 8, 12, 16, 24];

pub const REFRESH_MINUTES: [u32; 6] = [1, 5, 15, 30, 60, 0];

pub const SUBSCRIPTION_MINUTES: [u32; 6] = [5, 15, 60, 24 * 60, 7 * 24 * 60, 0];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarSettings {
    pub default_calendar: String,
    pub first_weekday: Option<Weekday>,
    pub day_start: u8,
    pub day_end: u8,
    pub hours_shown: u8,
    pub week_numbers: bool,
    pub time_zones: bool,
    pub timed_alert: Alert,
    pub all_day_alert: Alert,
    pub refresh_minutes: u32,
    pub subscription_minutes: u32,
    pub alerts_when_closed: bool,
}

impl Default for CalendarSettings {
    fn default() -> Self {
        Self {
            default_calendar: String::new(),
            first_weekday: None,
            day_start: 8,
            day_end: 18,
            hours_shown: 12,
            week_numbers: false,
            time_zones: false,
            timed_alert: Alert::None,
            all_day_alert: Alert::None,
            refresh_minutes: 5,
            subscription_minutes: 60,
            alerts_when_closed: true,
        }
    }
}

fn every(minutes: u32) -> Option<Duration> {
    (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

impl CalendarSettings {
    pub fn refresh(&self) -> Option<Duration> {
        every(self.refresh_minutes)
    }

    pub fn subscription_refresh(&self) -> Option<Duration> {
        every(self.subscription_minutes)
    }

    pub fn work_hours(&self) -> (u8, u8) {
        (self.day_start, self.day_end)
    }
}

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

fn alert_text(alert: Alert) -> String {
    match alert {
        Alert::Minutes(minutes) => minutes.to_string(),
        Alert::DayAt { days_before, .. } => format!("{days_before}d"),
        Alert::None | Alert::Custom => "none".to_owned(),
    }
}

fn parse_alert(value: &str, choices: &[Alert]) -> Option<Alert> {
    choices
        .iter()
        .copied()
        .find(|choice| alert_text(*choice) == value)
}

fn parse_choice(value: &str, choices: &[u32]) -> Option<u32> {
    value
        .parse()
        .ok()
        .filter(|minutes| choices.contains(minutes))
}

impl Stored for CalendarSettings {
    const FILE: &'static str = "calendar.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            match key {
                "default-calendar" => settings.default_calendar = value.to_owned(),
                "start-week-on" => {
                    settings.first_weekday =
                        WEEKDAYS.into_iter().find(|day| weekday_name(*day) == value);
                }
                "day-starts" => {
                    settings.day_start = value
                        .parse()
                        .ok()
                        .filter(|hour| *hour < 24)
                        .unwrap_or(settings.day_start);
                }
                "day-ends" => {
                    settings.day_end = value
                        .parse()
                        .ok()
                        .filter(|hour| (1..=24).contains(hour))
                        .unwrap_or(settings.day_end);
                }
                "hours-shown" => {
                    settings.hours_shown = value
                        .parse()
                        .ok()
                        .filter(|hours| HOURS_SHOWN.contains(hours))
                        .unwrap_or(settings.hours_shown);
                }
                "week-numbers" => {
                    settings.week_numbers = flag(value).unwrap_or(settings.week_numbers);
                }
                "time-zone-support" => {
                    settings.time_zones = flag(value).unwrap_or(settings.time_zones);
                }
                "event-alert" => {
                    settings.timed_alert =
                        parse_alert(value, &Alert::TIMED).unwrap_or(settings.timed_alert);
                }
                "all-day-alert" => {
                    settings.all_day_alert =
                        parse_alert(value, &Alert::ALL_DAY).unwrap_or(settings.all_day_alert);
                }
                "refresh-minutes" => {
                    settings.refresh_minutes =
                        parse_choice(value, &REFRESH_MINUTES).unwrap_or(settings.refresh_minutes);
                }
                "subscription-minutes" => {
                    settings.subscription_minutes = parse_choice(value, &SUBSCRIPTION_MINUTES)
                        .unwrap_or(settings.subscription_minutes);
                }
                "alerts-when-closed" => {
                    settings.alerts_when_closed =
                        flag(value).unwrap_or(settings.alerts_when_closed);
                }
                _ => {}
            }
        }
        if settings.day_end <= settings.day_start {
            settings.day_end = (settings.day_start + 1).min(24);
        }
        settings
    }

    fn to_text(&self) -> String {
        format!(
            "default-calendar = {}\nstart-week-on = {}\nday-starts = {}\nday-ends = {}\nhours-shown = {}\n\
             week-numbers = {}\ntime-zone-support = {}\nevent-alert = {}\nall-day-alert = {}\nrefresh-minutes = {}\n\
             subscription-minutes = {}\nalerts-when-closed = {}\n",
            self.default_calendar,
            self.first_weekday.map_or("default", weekday_name),
            self.day_start,
            self.day_end,
            self.hours_shown,
            self.week_numbers,
            self.time_zones,
            alert_text(self.timed_alert),
            alert_text(self.all_day_alert),
            self.refresh_minutes,
            self.subscription_minutes,
            self.alerts_when_closed,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    AddAccount,
    RemoveAccount(String),
    Password(String),
    SubscriptionRefresh(String, Option<u32>),
    SubscriptionAlerts(String, bool),
}

static CHANGES: OnceLock<Outbox<Change>> = OnceLock::new();

pub fn listen(outbox: Outbox<Change>) {
    if CHANGES.set(outbox).is_err() {
        tracing::warn!("the settings already have a listener");
    }
}

pub fn announce(change: Change) {
    if let Some(outbox) = CHANGES.get() {
        outbox.send(change);
    }
}
