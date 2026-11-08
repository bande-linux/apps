use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId};
use bande_datetime::DateFormats;
use bande_toolkit::ui::{
    Button, CheckState, Checkbox, PopUpButton, TEXT_FIELD_HEIGHT, TextField, Ui, WidgetId,
};
use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;

use super::panel::{Line, Panel, Width};
use super::{L10N, Signal, Signals, sender};
use crate::ical::{DaySpec, Frequency, Rule};

const WIDTH: f32 = 540.0;
const LABEL_WIDTH: f32 = 70.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;
const FREQUENCIES: [Frequency; 4] = [
    Frequency::Daily,
    Frequency::Weekly,
    Frequency::Monthly,
    Frequency::Yearly,
];

pub struct Custom {
    frequency: usize,
    interval: String,
    days: [bool; 7],
    order: [Weekday; 7],
    on_weekday: [bool; 2],
    start: Date,
    lines: [usize; 3],
    panel: WidgetId,
    cancel: WidgetId,
    dismissed: Rc<Cell<bool>>,
}

fn nth(day: Date) -> i16 {
    match day.day() + 7 > day.days_in_month() {
        true if day.day() > 28 => -1,
        _ => i16::from((day.day() - 1) / 7 + 1),
    }
}

fn nth_title(nth: i16) -> String {
    L10N.tr(match nth {
        1 => "nth-first",
        2 => "nth-second",
        3 => "nth-third",
        4 => "nth-fourth",
        _ => "nth-last",
    })
}

fn weekday_name(formats: &DateFormats, day: Date) -> String {
    day.to_zoned(TimeZone::UTC)
        .map(|at| formats.weekday(&at))
        .unwrap_or_default()
}

impl Custom {
    pub fn build(
        signals: &Signals,
        rule: Option<&Rule>,
        start: Date,
        formats: &DateFormats,
        first_weekday: Weekday,
    ) -> (Panel, Self) {
        let frequency = rule
            .and_then(|rule| {
                FREQUENCIES
                    .iter()
                    .position(|known| *known == rule.frequency)
            })
            .unwrap_or(1);
        let interval = rule.map_or(1, |rule| rule.interval).to_string();
        let order: [Weekday; 7] = std::array::from_fn(|index| {
            first_weekday.wrapping_add(i64::try_from(index).unwrap_or(0))
        });
        let chosen: Vec<Weekday> = rule
            .filter(|rule| rule.frequency == Frequency::Weekly)
            .map(|rule| rule.by_day.iter().map(|spec| spec.weekday).collect())
            .filter(|days: &Vec<Weekday>| !days.is_empty())
            .unwrap_or_else(|| vec![start.weekday()]);
        let days = order.map(|day| chosen.contains(&day));
        let by_weekday =
            rule.is_some_and(|rule| rule.by_day.iter().any(|spec| spec.ordinal.is_some()));
        let on_weekday = [by_weekday && frequency == 2, by_weekday && frequency == 3];
        let mut panel = Panel::new(Some(&L10N.tr("custom-title")), WIDTH, LABEL_WIDTH).sheet();
        let send = sender(signals);
        let count = panel.add(
            TextField::new("1", &interval, move |text| {
                send(Signal::CustomInterval(text.to_owned()))
            })
            .without_text_checking(),
        );
        let units: Vec<String> = [
            "custom-days",
            "custom-weeks",
            "custom-months",
            "custom-years",
        ]
        .iter()
        .map(|message| L10N.tr(message))
        .collect();
        let send = sender(signals);
        let unit = panel.add(PopUpButton::new(
            &L10N.tr("custom-every"),
            &units,
            frequency,
            move |index| send(Signal::CustomFrequency(index)),
        ));
        panel.line(Line::new(
            Some(&L10N.tr("custom-every")),
            vec![(count, Width::Fixed(56.0)), (unit, Width::Fill)],
            ROW,
        ));
        let cells = order
            .iter()
            .enumerate()
            .map(|(index, day)| {
                let send = sender(signals);
                let sample = start
                    .saturating_add(jiff::Span::new().days(i64::from(day.since(start.weekday()))));
                let state = match days[index] {
                    true => CheckState::On,
                    false => CheckState::Off,
                };
                let check = Checkbox::new(&formats.short_weekday(sample), state, move |on| {
                    send(Signal::CustomDay(index, on))
                });
                (panel.add(check), Width::Fill)
            })
            .collect();
        let weekly = panel.line(Line::new(Some(&L10N.tr("custom-on")), cells, ROW));
        let weekday = weekday_name(formats, start);
        let on_the = L10N.tr_args(
            "custom-on-the",
            [
                ("nth", nth_title(nth(start)).into()),
                ("weekday", weekday.clone().into()),
            ],
        );
        let monthly_choices = [
            L10N.tr_args("custom-on-day", [("day", start.day().into())]),
            on_the.clone(),
        ];
        let send = sender(signals);
        let monthly = panel.add(PopUpButton::new(
            &L10N.tr("custom-on"),
            &monthly_choices,
            usize::from(on_weekday[0]),
            move |index| send(Signal::CustomOnWeekday(0, index == 1)),
        ));
        let monthly = panel.line(Line::new(
            Some(&L10N.tr("custom-on")),
            vec![(monthly, Width::Fill)],
            ROW,
        ));
        let month = start.strftime("%B").to_string();
        let yearly_choices = [
            L10N.tr_args(
                "custom-on-date",
                [("day", start.day().into()), ("month", month.clone().into())],
            ),
            L10N.tr_args(
                "custom-on-the-of",
                [("on", on_the.into()), ("month", month.into())],
            ),
        ];
        let send = sender(signals);
        let yearly = panel.add(PopUpButton::new(
            &L10N.tr("custom-on"),
            &yearly_choices,
            usize::from(on_weekday[1]),
            move |index| send(Signal::CustomOnWeekday(1, index == 1)),
        ));
        let yearly = panel.line(Line::new(
            Some(&L10N.tr("custom-on")),
            vec![(yearly, Width::Fill)],
            ROW,
        ));
        let send = sender(signals);
        let cancel =
            panel.add(Button::new(&L10N.tr("cancel"), move || send(Signal::CustomCancel)).cancel());
        let send = sender(signals);
        let done = panel
            .add(Button::new(&L10N.tr("custom-done"), move || send(Signal::CustomDone)).primary());
        panel.line(
            Line::new(
                None,
                vec![(cancel, Width::Fixed(96.0)), (done, Width::Fixed(96.0))],
                ROW,
            )
            .separated()
            .trailing(),
        );
        let lines = [weekly, monthly, yearly];
        for (index, line) in lines.iter().enumerate() {
            panel.show_line(*line, index + 1 == frequency);
        }
        let custom = Self {
            frequency,
            interval,
            days,
            order,
            on_weekday,
            start,
            lines,
            panel: bande_toolkit::ui::Widget::id(&panel),
            cancel: panel.id_of(cancel).unwrap_or_else(WidgetId::next),
            dismissed: panel.dismisser(),
        };
        (panel, custom)
    }

    pub fn apply(&mut self, signal: &Signal, ui: &mut Ui) {
        match signal {
            Signal::CustomFrequency(index) => {
                self.frequency = (*index).min(FREQUENCIES.len() - 1);
                let (lines, frequency) = (self.lines, self.frequency);
                ui.update_widget::<Panel>(self.panel, |panel, _| {
                    for (index, line) in lines.iter().enumerate() {
                        panel.show_line(*line, index + 1 == frequency);
                    }
                });
            }
            Signal::CustomInterval(text) => self.interval = text.clone(),
            Signal::CustomDay(index, on) => {
                if let Some(day) = self.days.get_mut(*index) {
                    *day = *on;
                }
            }
            Signal::CustomOnWeekday(which, on) => {
                if let Some(chosen) = self.on_weekday.get_mut(*which) {
                    *chosen = *on;
                }
            }
            _ => {}
        }
    }

    pub fn rule(&self) -> Rule {
        let frequency = FREQUENCIES
            .get(self.frequency)
            .copied()
            .unwrap_or(Frequency::Weekly);
        let mut rule = Rule {
            interval: self
                .interval
                .trim()
                .parse()
                .ok()
                .filter(|count| *count > 0)
                .unwrap_or(1),
            ..Rule::new(frequency)
        };
        let on_the = DaySpec {
            ordinal: Some(nth(self.start)),
            weekday: self.start.weekday(),
        };
        match frequency {
            Frequency::Weekly => {
                rule.by_day = self
                    .order
                    .iter()
                    .zip(self.days)
                    .filter(|(_, on)| *on)
                    .map(|(weekday, _)| DaySpec {
                        ordinal: None,
                        weekday: *weekday,
                    })
                    .collect();
            }
            Frequency::Monthly if self.on_weekday[0] => rule.by_day = vec![on_the],
            Frequency::Yearly if self.on_weekday[1] => {
                rule.by_day = vec![on_the];
                rule.by_month = vec![self.start.month()];
            }
            _ => {}
        }
        rule
    }

    pub fn close(&self, ui: &mut Ui) {
        self.dismissed.set(true);
        ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: self.cancel.into(),
            data: None,
        });
    }
}
