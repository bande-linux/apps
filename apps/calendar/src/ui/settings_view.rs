use std::os::fd::BorrowedFd;
use std::path::PathBuf;


use bande_l10n::{Text, text, tr};

use super::inspector::alert_title;
use crate::edit::Alert;
use crate::settings::{
    CalendarSettings, Change, HOURS_SHOWN, REFRESH_MINUTES, SUBSCRIPTION_MINUTES, WEEKDAYS,
    announce,
};
use crate::store::{ACCOUNTS_FILE, AccountKind, Summary, data_dir};
use accesskit::TreeUpdate;
use bande_app::View;
use bande_app_settings::{SettingsWatch, SettingsWindow, Shared, settings_folder};
use bande_datetime::DateFormats;
use bande_toolkit::ui::{
    Axis, Button, CheckState, Checkbox, Context, Form, FormLine, Label, PopUpButton, ScrollView,
    Stack, Toggle, Ui, Widget,
};
use bande_toolkit::{Appearance, Rect, Scene, TextSystem, Window, WindowEvent};
use jiff::civil::{Date, Time};

pub const SETTINGS_SIZE: (u32, u32) = (520, 560);

type Set<T> = fn(&mut CalendarSettings, T);

fn pop_up<T: Copy + PartialEq + 'static>(
    settings: &Shared<CalendarSettings>,
    message: Text,
    choices: Vec<(T, String)>,
    current: T,
    set: Set<T>,
) -> Box<dyn Widget> {
    let labels: Vec<String> = choices.iter().map(|(_, label)| label.clone()).collect();
    let chosen = choices
        .iter()
        .position(|(value, _)| *value == current)
        .unwrap_or_default();
    let shared = settings.clone();
    Box::new(PopUpButton::new(
        &message.tr(),
        &labels,
        chosen,
        move |index| {
            if let Some((value, _)) = choices.get(index) {
                set(&mut shared.borrow_mut(), *value);
            }
        },
    ))
}

fn noon(day: Date, hour: u8) -> Option<jiff::Zoned> {
    let time = Time::new(i8::try_from(hour % 24).ok()?, 0, 0, 0).ok()?;
    day.to_datetime(time).to_zoned(jiff::tz::TimeZone::UTC).ok()
}

fn hour_label(formats: &DateFormats, hour: u8) -> String {
    noon(Date::constant(2000, 1, 1), hour)
        .map(|at| formats.time(&at))
        .map_or_else(
            || format!("{hour:02}:00"),
            |(time, period)| period.map_or(time.clone(), |period| format!("{time} {period}")),
        )
}

fn every_label(minutes: u32) -> String {
    match minutes {
        0 => tr!("Manually"),
        1 => tr!("Every minute"),
        60 => tr!("Every hour"),
        1440 => tr!("Every day"),
        10080 => tr!("Every week"),
        minutes => tr!("Every {minutes} minutes", minutes = minutes),
    }
}

fn general(settings: &Shared<CalendarSettings>, summary: &Summary) -> Vec<FormLine> {
    let current = settings.borrow().clone();
    let formats = DateFormats::from_env();
    let calendars: Vec<(usize, String)> = summary
        .writable()
        .enumerate()
        .map(|(index, (calendar, account))| {
            let name = match account {
                Some(account) => format!("{} ({account})", calendar.name),
                None => calendar.name.clone(),
            };
            (index, name)
        })
        .collect();
    let ids: Vec<String> = summary
        .writable()
        .map(|(calendar, _)| calendar.id.clone())
        .collect();
    let chosen = ids
        .iter()
        .position(|id| *id == current.default_calendar)
        .or_else(|| summary.automatic())
        .unwrap_or_default();
    let shared = settings.clone();
    let labels: Vec<String> = calendars.into_iter().map(|(_, name)| name).collect();
    let default_calendar =
        PopUpButton::new(&tr!("Default calendar:"), &labels, chosen, move |index| {
            if let Some(id) = ids.get(index) {
                shared.borrow_mut().default_calendar = id.clone();
            }
        });
    let weekdays = std::iter::once((None, tr!("Default for Region")))
        .chain(WEEKDAYS.iter().map(|day| {
            let name = Date::constant(2000, 1, 2)
                .nth_weekday_of_month(1, *day)
                .ok()
                .and_then(|sample| noon(sample, 12))
                .map(|sample| formats.weekday(&sample));
            (Some(*day), name.unwrap_or_default())
        }))
        .collect();
    let starts = (0..24)
        .map(|hour| (hour, hour_label(&formats, hour)))
        .collect();
    let ends = (1..=24)
        .map(|hour| (hour, hour_label(&formats, hour)))
        .collect();
    let shared = settings.clone();
    let week_numbers = Toggle::new(&tr!("Show week numbers"), current.week_numbers, move |on| {
        shared.borrow_mut().week_numbers = on
    });
    vec![
        FormLine::Heading(tr!("General")),
        FormLine::Row(tr!("Default calendar:"), Box::new(default_calendar)),
        FormLine::Row(
            tr!("Start week on:"),
            pop_up(
                settings,
                text!("Start week on:"),
                weekdays,
                current.first_weekday,
                |settings, day| settings.first_weekday = day,
            ),
        ),
        FormLine::Row(
            tr!("Day starts at:"),
            pop_up(
                settings,
                text!("Day starts at:"),
                starts,
                current.day_start,
                |settings, hour| {
                    settings.day_start = hour;
                    settings.day_end = settings.day_end.max(hour + 1);
                },
            ),
        ),
        FormLine::Row(
            tr!("Day ends at:"),
            pop_up(
                settings,
                text!("Day ends at:"),
                ends,
                current.day_end,
                |settings, hour| {
                    settings.day_end = hour;
                    settings.day_start = settings.day_start.min(hour - 1);
                },
            ),
        ),
        FormLine::Row(
            tr!("Show:"),
            pop_up(
                settings,
                text!("Show:"),
                HOURS_SHOWN
                    .iter()
                    .map(|hours| (*hours, tr!("{hours} hours at a time", hours = (*hours))))
                    .collect(),
                current.hours_shown,
                |settings, hours| settings.hours_shown = hours,
            ),
        ),
        FormLine::Row(tr!("Show week numbers"), Box::new(week_numbers)),
    ]
}

fn advanced(settings: &Shared<CalendarSettings>) -> Vec<FormLine> {
    let shared = settings.clone();
    let time_zones = Toggle::new(
        &tr!("Time zone support"),
        settings.borrow().time_zones,
        move |on| shared.borrow_mut().time_zones = on,
    );
    vec![
        FormLine::Heading(tr!("Advanced")),
        FormLine::Row(tr!("Time zone support"), Box::new(time_zones)),
        FormLine::Wide(Box::new(
            Label::new(&tr!("Shows each event’s time zone in its details, so times can be set in another zone, such as a flight’s departure and arrival.")).secondary(),
        )),
    ]
}

fn alerts(settings: &Shared<CalendarSettings>) -> Vec<FormLine> {
    let current = settings.borrow().clone();
    let choices = |all: &[Alert]| -> Vec<(Alert, String)> {
        all.iter()
            .map(|alert| (*alert, alert_title(*alert)))
            .collect()
    };
    let shared = settings.clone();
    let when_closed = Toggle::new(
        &tr!("Show alerts when Calendar is closed"),
        current.alerts_when_closed,
        move |on| shared.borrow_mut().alerts_when_closed = on,
    );
    vec![
        FormLine::Heading(tr!("Default Alerts")),
        FormLine::Row(
            tr!("Events:"),
            pop_up(
                settings,
                text!("Events:"),
                choices(&Alert::TIMED),
                current.timed_alert,
                |settings, alert| settings.timed_alert = alert,
            ),
        ),
        FormLine::Row(
            tr!("All-day events:"),
            pop_up(
                settings,
                text!("All-day events:"),
                choices(&Alert::ALL_DAY),
                current.all_day_alert,
                |settings, alert| settings.all_day_alert = alert,
            ),
        ),
        FormLine::Row(
            tr!("Show alerts when Calendar is closed"),
            Box::new(when_closed),
        ),
    ]
}

fn accounts(settings: &Shared<CalendarSettings>, summary: &Summary) -> Vec<FormLine> {
    let current = settings.borrow().clone();
    let every = |all: &[u32]| -> Vec<(u32, String)> {
        all.iter()
            .map(|minutes| (*minutes, every_label(*minutes)))
            .collect()
    };
    let mut lines = vec![
        FormLine::Heading(tr!("Accounts")),
        FormLine::Row(
            tr!("Refresh calendars:"),
            pop_up(
                settings,
                text!("Refresh calendars:"),
                every(&REFRESH_MINUTES),
                current.refresh_minutes,
                |settings, minutes| settings.refresh_minutes = minutes,
            ),
        ),
        FormLine::Row(
            tr!("Refresh subscriptions:"),
            pop_up(
                settings,
                text!("Refresh subscriptions:"),
                every(&SUBSCRIPTION_MINUTES),
                current.subscription_minutes,
                |settings, minutes| settings.subscription_minutes = minutes,
            ),
        ),
    ];
    for account in &summary.accounts {
        let id = account.id.clone();
        let (kind, mut buttons): (String, Vec<Box<dyn Widget>>) = match &account.kind {
            AccountKind::Local => continue,
            AccountKind::CalDav { server, .. } => (
                tr!("CalDAV on {server}", server = server.clone()),
                vec![Box::new(Button::new(&tr!("Password…"), move || {
                    announce(Change::Password(id.clone()))
                }))],
            ),
            AccountKind::Subscription {
                url,
                refresh_minutes,
                alerts,
            } => (
                tr!("Subscribed to {url}", url = url.clone()),
                subscription_controls(&account.id, *refresh_minutes, *alerts),
            ),
        };
        let id = account.id.clone();
        buttons.push(Box::new(
            Button::new(&tr!("Remove…"), move || {
                announce(Change::RemoveAccount(id.clone()))
            })
            .tooltip(&kind),
        ));
        lines.push(FormLine::Row(
            account.name.clone(),
            Box::new(Stack::new(Axis::Horizontal, 8.0, 0.0, buttons).trailing()),
        ));
    }
    let add = Button::new(&tr!("Add Account…"), || announce(Change::AddAccount));
    lines.push(FormLine::Row(String::new(), Box::new(add)));
    lines.push(FormLine::Wide(Box::new(
        Label::new(&tr!("iCloud and Fastmail sign in with an app password, Nextcloud with your password or an app password. Google needs OAuth, which Calendar cannot do yet.")).secondary(),
    )));
    lines
}

fn subscription_controls(id: &str, refresh: Option<u32>, alerts: bool) -> Vec<Box<dyn Widget>> {
    let choices: Vec<Option<u32>> = std::iter::once(None)
        .chain(SUBSCRIPTION_MINUTES.iter().copied().map(Some))
        .collect();
    let labels: Vec<String> = choices
        .iter()
        .map(|choice| match choice {
            None => tr!("As in Settings"),
            Some(minutes) => every_label(*minutes),
        })
        .collect();
    let chosen = choices
        .iter()
        .position(|choice| *choice == refresh)
        .unwrap_or_default();
    let shared = id.to_owned();
    let refresh = PopUpButton::new(
        &tr!("Refresh subscriptions:"),
        &labels,
        chosen,
        move |index| {
            if let Some(choice) = choices.get(index) {
                announce(Change::SubscriptionRefresh(shared.clone(), *choice));
            }
        },
    );
    let shared = id.to_owned();
    let state = match alerts {
        true => CheckState::On,
        false => CheckState::Off,
    };
    let alerts = Checkbox::new(&tr!("Alerts"), state, move |on| {
        announce(Change::SubscriptionAlerts(shared.clone(), on))
    });
    vec![Box::new(refresh), Box::new(alerts)]
}

fn settings_form(appearance: Appearance, settings: &Shared<CalendarSettings>) -> Ui {
    let summary = Summary::load(&data_dir());
    let mut lines = general(settings, &summary);
    lines.extend(alerts(settings));
    lines.extend(accounts(settings, &summary));
    lines.extend(advanced(settings));
    let form = Form::new(&tr!("Calendar Settings"), lines);
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form)),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}

pub struct SettingsView {
    window: SettingsWindow<CalendarSettings>,
    accounts: Option<SettingsWatch>,
    folder: Option<PathBuf>,
}

impl SettingsView {
    pub fn new(appearance: Appearance) -> Self {
        let folder = settings_folder();
        Self {
            window: SettingsWindow::new(appearance, folder.clone(), settings_form),
            accounts: SettingsWatch::start(&data_dir(), ACCOUNTS_FILE),
            folder,
        }
    }
}

impl View for SettingsView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.window.scene(window)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        self.window.event(window, event)
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.window.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.window.set_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = self.window.sources();
        sources.extend(self.accounts.iter().map(SettingsWatch::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) -> bool {
        let own = self.window.sources().len();
        if index < own {
            return self.window.source_ready(window, index);
        }
        if let Some(accounts) = &mut self.accounts {
            accounts.drain();
        }
        self.window = SettingsWindow::new(window.appearance(), self.folder.clone(), settings_form);
        true
    }

    fn animating(&self) -> bool {
        self.window.animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        self.window.accessibility()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.window.text_input_area()
    }
}
