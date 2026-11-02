use bande_toolkit::Rect;
use bande_toolkit::ui::{
    Button, CheckState, Checkbox, Label, PopUpButton, TEXT_FIELD_HEIGHT, TextField, TextView,
    WidgetId,
};
use jiff::ToSpan;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;

use super::panel::{Arrow, Line, Panel, Width};
use super::{L10N, Signal, Signals, sender};
use crate::edit::{Alert, Details, EndRepeat, Repeat};
use crate::ical::Event;
use crate::store::Found;

const WIDTH: f32 = 320.0;
const LABEL_WIDTH: f32 = 70.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;
const GAP: f32 = 10.0;

pub struct Inspector {
    pub found: Found,
    pub panel: WidgetId,
    pub title_field: Option<WidgetId>,
    pub read_only: bool,
    pub original: Details,
    details: Details,
    texts: [String; 4],
    end_kind: usize,
    end_text: String,
    calendars: Vec<usize>,
    calendar_choice: usize,
    original_calendar: usize,
    repeats: Vec<Repeat>,
    alerts: Vec<Alert>,
    anchor: Option<Rect>,
    size: (f32, f32),
}

pub fn parse_time(text: &str) -> Option<Time> {
    let lower = text.trim().to_lowercase();
    let (body, afternoon) = match (lower.strip_suffix("pm"), lower.strip_suffix("am")) {
        (Some(body), _) => (body.trim(), Some(true)),
        (_, Some(body)) => (body.trim(), Some(false)),
        _ => (lower.as_str(), None),
    };
    let (hours, minutes) = match body.split_once([':', '.', 'h']) {
        Some((hours, minutes)) => (
            hours.trim().parse::<i8>().ok()?,
            minutes.trim().parse::<i8>().ok()?,
        ),
        None if body.len() > 2 => {
            let split = body.len() - 2;
            (
                body.get(..split)?.parse().ok()?,
                body.get(split..)?.parse().ok()?,
            )
        }
        None => (body.parse().ok()?, 0),
    };
    let hours = match afternoon {
        Some(true) if hours < 12 => hours + 12,
        Some(false) if hours == 12 => 0,
        Some(_) if !(1..=12).contains(&hours) => return None,
        _ => hours,
    };
    Time::new(hours, minutes, 0, 0).ok()
}

pub fn parse_date(text: &str) -> Option<Date> {
    let text = text.trim();
    let parts: Vec<&str> = text.split(['-', '.', '/']).map(str::trim).collect();
    let [a, b, c] = parts.as_slice() else {
        return None;
    };
    let (year, month, day) = match a.len() == 4 {
        true => (a.parse().ok()?, b.parse().ok()?, c.parse().ok()?),
        false => (c.parse().ok()?, b.parse().ok()?, a.parse().ok()?),
    };
    Date::new(year, month, day).ok()
}

fn date_text(day: Date) -> String {
    day.strftime("%Y-%m-%d").to_string()
}

fn time_text(at: DateTime) -> String {
    at.strftime("%H:%M").to_string()
}

fn repeat_title(repeat: Repeat) -> String {
    L10N.tr(match repeat {
        Repeat::Never => "repeat-never",
        Repeat::Daily => "repeat-daily",
        Repeat::Weekdays => "repeat-weekdays",
        Repeat::Weekly => "repeat-weekly",
        Repeat::Biweekly => "repeat-biweekly",
        Repeat::Monthly => "repeat-monthly",
        Repeat::Yearly => "repeat-yearly",
        Repeat::Custom => "repeat-custom",
    })
}

pub(crate) fn alert_title(alert: Alert) -> String {
    match alert {
        Alert::None => L10N.tr("alert-none"),
        Alert::Custom => L10N.tr("alert-custom"),
        Alert::Minutes(0) => L10N.tr("alert-at-time"),
        Alert::Minutes(60) => L10N.tr("alert-hour"),
        Alert::Minutes(1440) => L10N.tr("alert-day"),
        Alert::Minutes(minutes) if minutes % 60 == 0 => {
            L10N.tr_args("alert-hours", [("hours", (minutes / 60).into())])
        }
        Alert::Minutes(minutes) => L10N.tr_args("alert-minutes", [("minutes", minutes.into())]),
        Alert::DayAt { days_before, hour } => {
            let time = format!("{hour:02}:00");
            let id = match days_before {
                0 => "alert-on-day",
                1 => "alert-day-before",
                _ => "alert-week-before",
            };
            L10N.tr_args(id, [("time", time.into())])
        }
    }
}

fn end_of(end: EndRepeat) -> (usize, String) {
    match end {
        EndRepeat::Never => (0, String::new()),
        EndRepeat::After(count) => (1, count.to_string()),
        EndRepeat::On(day) => (2, date_text(day)),
    }
}

fn details_of(found: &Found, event: &Event, local: &TimeZone) -> Details {
    let start = found.occurrence.start.datetime();
    let end = found.occurrence.end.datetime();
    let all_day = found.occurrence.all_day;
    Details {
        summary: event.summary.clone(),
        location: event.location.clone(),
        notes: event.description.clone(),
        all_day,
        start,
        end,
        repeat: Repeat::of(event.rule.as_ref()),
        end_repeat: EndRepeat::of(event.rule.as_ref(), &event.start, local),
        alert: Alert::of(&event.alarms, all_day),
    }
}

fn with_current<T: PartialEq + Copy>(choices: &[T], current: T) -> Vec<T> {
    let mut all = choices.to_vec();
    if !all.contains(&current) {
        all.push(current);
    }
    all
}

impl Inspector {
    pub fn build(
        signals: &Signals,
        found: Found,
        event: &Event,
        read_only: bool,
        calendars: Vec<(usize, String)>,
        anchor: Option<Rect>,
        local: &TimeZone,
    ) -> (Panel, Self) {
        let details = details_of(&found, event, local);
        let (end_kind, end_text) = end_of(details.end_repeat);
        let last_day = match details.all_day {
            true => details
                .end
                .date()
                .yesterday()
                .unwrap_or(details.end.date())
                .max(details.start.date()),
            false => details.end.date(),
        };
        let texts = [
            date_text(details.start.date()),
            time_text(details.start),
            date_text(last_day),
            time_text(details.end),
        ];
        let repeats = with_current(&Repeat::CHOICES, details.repeat);
        let alerts = match details.all_day {
            true => with_current(&Alert::ALL_DAY, details.alert),
            false => with_current(&Alert::TIMED, details.alert),
        };
        let mut panel = Panel::new(None, WIDTH, LABEL_WIDTH);
        let mut title_field = None;
        if read_only {
            let title = panel.add(Label::heading(&details.summary));
            panel.line(Line::new(None, vec![(title, Width::Fill)], 24.0));
            if !details.location.is_empty() {
                let location = panel.add(Label::new(&details.location));
                panel.line(Line::new(None, vec![(location, Width::Fill)], 18.0));
            }
            let when = match details.all_day {
                true => format!("{} – {}", texts[0], texts[2]),
                false => format!("{} {} – {}", texts[0], texts[1], texts[3]),
            };
            let when = panel.add(Label::new(&when));
            panel.line(Line::new(None, vec![(when, Width::Fill)], 18.0).separated());
            if !details.notes.is_empty() {
                let notes = panel.add(Label::new(&details.notes));
                panel.line(Line::new(
                    None,
                    vec![(notes, Width::Fill)],
                    18.0 * details.notes.lines().count().clamp(1, 6) as f32,
                ));
            }
            let note = panel.add(Label::new(&L10N.tr("read-only")));
            panel.line(Line::new(None, vec![(note, Width::Fill)], 18.0).separated());
        } else {
            let send = sender(signals);
            let title = panel.add(
                TextField::new(
                    &L10N.tr("title-placeholder"),
                    &details.summary,
                    move |text| send(Signal::Title(text.to_owned())),
                )
                .in_place(17.0),
            );
            title_field = panel.id_of(title);
            panel.line(Line::new(None, vec![(title, Width::Fill)], 28.0));
            let send = sender(signals);
            let location = panel.add(
                TextField::new(
                    &L10N.tr("location-placeholder"),
                    &details.location,
                    move |text| send(Signal::Location(text.to_owned())),
                )
                .borderless(13.0),
            );
            panel.line(Line::new(None, vec![(location, Width::Fill)], ROW));
            let send = sender(signals);
            let state = match details.all_day {
                true => CheckState::On,
                false => CheckState::Off,
            };
            let all_day = panel.add(Checkbox::new("", state, move |on| send(Signal::AllDay(on))));
            panel.line(
                Line::new(
                    Some(&L10N.tr("all-day-label")),
                    vec![(all_day, Width::Fixed(20.0))],
                    ROW,
                )
                .separated(),
            );
            let field = |text: &str, signal: fn(String) -> Signal| {
                let send = sender(signals);
                TextField::new("", text, move |text| send(signal(text.to_owned())))
            };
            let start_date = panel.add(field(&texts[0], Signal::StartDate));
            let start_time = panel.add(field(&texts[1], Signal::StartTime));
            panel.line(Line::new(
                Some(&L10N.tr("starts")),
                vec![(start_date, Width::Fill), (start_time, Width::Fixed(64.0))],
                ROW,
            ));
            let end_date = panel.add(field(&texts[2], Signal::EndDate));
            let end_time = panel.add(field(&texts[3], Signal::EndTime));
            panel.line(Line::new(
                Some(&L10N.tr("ends")),
                vec![(end_date, Width::Fill), (end_time, Width::Fixed(64.0))],
                ROW,
            ));
            let send = sender(signals);
            let titles: Vec<String> = repeats.iter().map(|repeat| repeat_title(*repeat)).collect();
            let selected = repeats
                .iter()
                .position(|repeat| *repeat == details.repeat)
                .unwrap_or(0);
            let repeat = panel.add(PopUpButton::new(
                &L10N.tr("repeat"),
                &titles,
                selected,
                move |index| send(Signal::Repeat(index)),
            ));
            panel.line(Line::new(
                Some(&L10N.tr("repeat")),
                vec![(repeat, Width::Fill)],
                ROW,
            ));
            let send = sender(signals);
            let ends: Vec<String> = ["end-repeat-never", "end-repeat-after", "end-repeat-on"]
                .iter()
                .map(|message| L10N.tr(message))
                .collect();
            let end_choice = panel.add(PopUpButton::new(
                &L10N.tr("end-repeat"),
                &ends,
                end_kind,
                move |index| send(Signal::EndRepeatKind(index)),
            ));
            let send = sender(signals);
            let end_field = panel.add(TextField::new(
                &L10N.tr("end-repeat-placeholder"),
                &end_text,
                move |text| send(Signal::EndRepeatText(text.to_owned())),
            ));
            panel.line(Line::new(
                Some(&L10N.tr("end-repeat")),
                vec![(end_choice, Width::Fill), (end_field, Width::Fixed(96.0))],
                ROW,
            ));
            let send = sender(signals);
            let titles: Vec<String> = alerts.iter().map(|alert| alert_title(*alert)).collect();
            let selected = alerts
                .iter()
                .position(|alert| *alert == details.alert)
                .unwrap_or(0);
            let alert = panel.add(PopUpButton::new(
                &L10N.tr("alert"),
                &titles,
                selected,
                move |index| send(Signal::Alert(index)),
            ));
            panel.line(Line::new(
                Some(&L10N.tr("alert")),
                vec![(alert, Width::Fill)],
                ROW,
            ));
            let send = sender(signals);
            let names: Vec<&str> = calendars.iter().map(|(_, name)| name.as_str()).collect();
            let selected = calendars
                .iter()
                .position(|(index, _)| *index == found.calendar)
                .unwrap_or(0);
            let calendar = panel.add(PopUpButton::new(
                &L10N.tr("calendar"),
                &names,
                selected,
                move |index| send(Signal::Calendar(index)),
            ));
            panel.line(
                Line::new(
                    Some(&L10N.tr("calendar")),
                    vec![(calendar, Width::Fill)],
                    ROW,
                )
                .separated(),
            );
            let send = sender(signals);
            let notes = panel.add(
                TextView::new(&L10N.tr("notes-placeholder"), &details.notes, move |text| {
                    send(Signal::Notes(text.to_owned()))
                })
                .visible_lines(3),
            );
            panel.line(Line::new(None, vec![(notes, Width::Fill)], 64.0).separated());
            let send = sender(signals);
            let delete = panel.add(
                Button::new(&L10N.tr("delete-event"), move || send(Signal::DeleteEvent))
                    .destructive(),
            );
            panel.line(Line::new(None, vec![(delete, Width::Fixed(120.0))], ROW).trailing());
        }
        let size = (panel.width(), panel.height());
        let selected = calendars
            .iter()
            .position(|(index, _)| *index == found.calendar)
            .unwrap_or(0);
        let inspector = Self {
            panel: bande_toolkit::ui::Widget::id(&panel),
            title_field,
            read_only,
            original: details.clone(),
            original_calendar: selected,
            details,
            texts,
            end_kind,
            end_text,
            calendars: calendars.into_iter().map(|(index, _)| index).collect(),
            calendar_choice: selected,
            repeats,
            alerts,
            anchor,
            size,
            found,
        };
        (panel, inspector)
    }

    pub fn frame(&self, sheet: Rect) -> Rect {
        let (width, height) = self.size;
        let anchor = self.anchor.unwrap_or(Rect::new(
            sheet.x + sheet.width / 2.0,
            sheet.y + sheet.height / 3.0,
            0.0,
            0.0,
        ));
        let right = anchor.x + anchor.width + GAP;
        let x = match right + width <= sheet.x + sheet.width {
            true => right,
            false => (anchor.x - GAP - width).max(sheet.x + GAP),
        };
        let center = anchor.y + anchor.height.min(80.0) / 2.0;
        let y = (center - height / 3.0).clamp(
            sheet.y + GAP,
            (sheet.y + sheet.height - height - GAP).max(sheet.y + GAP),
        );
        Rect::new(x, y, width, height)
    }

    pub fn arrow(&self, sheet: Rect) -> Arrow {
        let frame = self.frame(sheet);
        match self.anchor {
            Some(anchor) if frame.x > anchor.x => {
                Arrow::Left(anchor.y + anchor.height.min(80.0) / 2.0)
            }
            Some(anchor) => Arrow::Right(anchor.y + anchor.height.min(80.0) / 2.0),
            None => Arrow::None,
        }
    }

    pub fn apply(&mut self, signal: &Signal) {
        match signal {
            Signal::Title(text) => self.details.summary = text.clone(),
            Signal::Location(text) => self.details.location = text.clone(),
            Signal::Notes(text) => self.details.notes = text.clone(),
            Signal::AllDay(on) => {
                self.details.all_day = *on;
                if *on && !self.original.all_day {
                    self.details.alert = Alert::None;
                }
            }
            Signal::StartDate(text) => self.texts[0] = text.clone(),
            Signal::StartTime(text) => self.texts[1] = text.clone(),
            Signal::EndDate(text) => self.texts[2] = text.clone(),
            Signal::EndTime(text) => self.texts[3] = text.clone(),
            Signal::Repeat(index) => {
                self.details.repeat = self.repeats.get(*index).copied().unwrap_or(Repeat::Never)
            }
            Signal::Alert(index) => {
                self.details.alert = self.alerts.get(*index).copied().unwrap_or(Alert::None)
            }
            Signal::Calendar(index) => self.calendar_choice = *index,
            Signal::EndRepeatKind(index) => self.end_kind = *index,
            Signal::EndRepeatText(text) => self.end_text = text.clone(),
            _ => {}
        }
    }

    fn times(&self) -> Option<(DateTime, DateTime)> {
        let start_day = parse_date(&self.texts[0])?;
        let end_day = parse_date(&self.texts[2])?;
        match self.details.all_day {
            true => {
                let end_day = end_day.max(start_day);
                Some((
                    start_day.to_datetime(Time::midnight()),
                    end_day
                        .checked_add(1.day())
                        .ok()?
                        .to_datetime(Time::midnight()),
                ))
            }
            false => {
                let start = start_day.to_datetime(parse_time(&self.texts[1])?);
                let end = end_day.to_datetime(parse_time(&self.texts[3])?);
                let end = match end <= start {
                    true => start.checked_add(1.hour()).ok()?,
                    false => end,
                };
                Some((start, end))
            }
        }
    }

    pub fn edited_details(&self) -> Option<Details> {
        let (start, end) = self
            .times()
            .unwrap_or((self.original.start, self.original.end));
        let (start, end) = match (self.details.all_day, self.original.all_day) {
            (false, true) if self.times().is_none() || self.texts[1] == "00:00" => {
                let start = start.date().to_datetime(Time::constant(9, 0, 0, 0));
                (start, start.checked_add(1.hour()).unwrap_or(start))
            }
            _ => (start, end),
        };
        let end_repeat = match self.end_kind {
            1 => EndRepeat::After(
                self.end_text
                    .trim()
                    .parse()
                    .ok()
                    .filter(|count| *count > 0)
                    .unwrap_or(1),
            ),
            2 => EndRepeat::On(parse_date(&self.end_text).unwrap_or(start.date())),
            _ => EndRepeat::Never,
        };
        let details = Details {
            start,
            end,
            end_repeat,
            ..self.details.clone()
        };
        (details != self.original || self.calendar_choice != self.original_calendar)
            .then_some(details)
    }

    pub fn chosen_calendar(&self) -> Option<usize> {
        self.calendars.get(self.calendar_choice).copied()
    }
}
