use std::os::fd::BorrowedFd;
use std::path::PathBuf;

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

use super::L10N;
use super::inspector::alert_title;
use crate::edit::Alert;
use crate::settings::{
    CalendarSettings, Change, HOURS_SHOWN, REFRESH_MINUTES, SUBSCRIPTION_MINUTES, WEEKDAYS,
    announce,
};
use crate::store::{ACCOUNTS_FILE, AccountKind, Summary, data_dir};

pub const SETTINGS_SIZE: (u32, u32) = (520, 560);

type Set<T> = fn(&mut CalendarSettings, T);

fn pop_up<T: Copy + PartialEq + 'static>(
    settings: &Shared<CalendarSettings>,
    message: &str,
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
        &L10N.tr(message),
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
        0 => L10N.tr("refresh-manually"),
        1 => L10N.tr("refresh-every-minute"),
        60 => L10N.tr("refresh-every-hour"),
        1440 => L10N.tr("refresh-every-day"),
        10080 => L10N.tr("refresh-every-week"),
        minutes => L10N.tr_args("refresh-every-minutes", [("minutes", minutes.into())]),
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
    let default_calendar = PopUpButton::new(
        &L10N.tr("settings-default-calendar"),
        &labels,
        chosen,
        move |index| {
            if let Some(id) = ids.get(index) {
                shared.borrow_mut().default_calendar = id.clone();
            }
        },
    );
    let weekdays = std::iter::once((None, L10N.tr("settings-week-default")))
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
    let week_numbers = Toggle::new(
        &L10N.tr("settings-week-numbers"),
        current.week_numbers,
        move |on| shared.borrow_mut().week_numbers = on,
    );
    vec![
        FormLine::Heading(L10N.tr("settings-general")),
        FormLine::Row(
            L10N.tr("settings-default-calendar"),
            Box::new(default_calendar),
        ),
        FormLine::Row(
            L10N.tr("settings-start-week"),
            pop_up(
                settings,
                "settings-start-week",
                weekdays,
                current.first_weekday,
                |settings, day| settings.first_weekday = day,
            ),
        ),
        FormLine::Row(
            L10N.tr("settings-day-starts"),
            pop_up(
                settings,
                "settings-day-starts",
                starts,
                current.day_start,
                |settings, hour| {
                    settings.day_start = hour;
                    settings.day_end = settings.day_end.max(hour + 1);
                },
            ),
        ),
        FormLine::Row(
            L10N.tr("settings-day-ends"),
            pop_up(
                settings,
                "settings-day-ends",
                ends,
                current.day_end,
                |settings, hour| {
                    settings.day_end = hour;
                    settings.day_start = settings.day_start.min(hour - 1);
                },
            ),
        ),
        FormLine::Row(
            L10N.tr("settings-hours-shown"),
            pop_up(
                settings,
                "settings-hours-shown",
                HOURS_SHOWN
                    .iter()
                    .map(|hours| {
                        (
                            *hours,
                            L10N.tr_args("settings-hours", [("hours", (*hours).into())]),
                        )
                    })
                    .collect(),
                current.hours_shown,
                |settings, hours| settings.hours_shown = hours,
            ),
        ),
        FormLine::Row(L10N.tr("settings-week-numbers"), Box::new(week_numbers)),
    ]
}

fn advanced(settings: &Shared<CalendarSettings>) -> Vec<FormLine> {
    let shared = settings.clone();
    let time_zones = Toggle::new(
        &L10N.tr("settings-time-zones"),
        settings.borrow().time_zones,
        move |on| shared.borrow_mut().time_zones = on,
    );
    vec![
        FormLine::Heading(L10N.tr("settings-advanced")),
        FormLine::Row(L10N.tr("settings-time-zones"), Box::new(time_zones)),
        FormLine::Wide(Box::new(
            Label::new(&L10N.tr("settings-time-zones-note")).secondary(),
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
    vec![
        FormLine::Heading(L10N.tr("settings-alerts")),
        FormLine::Row(
            L10N.tr("settings-event-alert"),
            pop_up(
                settings,
                "settings-event-alert",
                choices(&Alert::TIMED),
                current.timed_alert,
                |settings, alert| settings.timed_alert = alert,
            ),
        ),
        FormLine::Row(
            L10N.tr("settings-all-day-alert"),
            pop_up(
                settings,
                "settings-all-day-alert",
                choices(&Alert::ALL_DAY),
                current.all_day_alert,
                |settings, alert| settings.all_day_alert = alert,
            ),
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
        FormLine::Heading(L10N.tr("settings-accounts")),
        FormLine::Row(
            L10N.tr("settings-refresh"),
            pop_up(
                settings,
                "settings-refresh",
                every(&REFRESH_MINUTES),
                current.refresh_minutes,
                |settings, minutes| settings.refresh_minutes = minutes,
            ),
        ),
        FormLine::Row(
            L10N.tr("settings-subscription-refresh"),
            pop_up(
                settings,
                "settings-subscription-refresh",
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
                L10N.tr_args("settings-caldav-at", [("server", server.clone().into())]),
                vec![Box::new(Button::new(
                    &L10N.tr("change-password"),
                    move || announce(Change::Password(id.clone())),
                ))],
            ),
            AccountKind::Subscription {
                url,
                refresh_minutes,
                alerts,
            } => (
                L10N.tr_args("settings-subscribed-to", [("url", url.clone().into())]),
                subscription_controls(&account.id, *refresh_minutes, *alerts),
            ),
        };
        let id = account.id.clone();
        buttons.push(Box::new(
            Button::new(&L10N.tr("settings-remove"), move || {
                announce(Change::RemoveAccount(id.clone()))
            })
            .tooltip(&kind),
        ));
        lines.push(FormLine::Row(
            account.name.clone(),
            Box::new(Stack::new(Axis::Horizontal, 8.0, 0.0, buttons).trailing()),
        ));
    }
    let add = Button::new(&L10N.tr("add-account"), || announce(Change::AddAccount));
    lines.push(FormLine::Row(String::new(), Box::new(add)));
    lines.push(FormLine::Wide(Box::new(
        Label::new(&L10N.tr("settings-accounts-note")).secondary(),
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
            None => L10N.tr("refresh-default"),
            Some(minutes) => every_label(*minutes),
        })
        .collect();
    let chosen = choices
        .iter()
        .position(|choice| *choice == refresh)
        .unwrap_or_default();
    let shared = id.to_owned();
    let refresh = PopUpButton::new(
        &L10N.tr("settings-subscription-refresh"),
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
    let alerts = Checkbox::new(&L10N.tr("settings-subscription-alerts"), state, move |on| {
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
    let form = Form::new(&L10N.tr("settings-title"), lines);
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
