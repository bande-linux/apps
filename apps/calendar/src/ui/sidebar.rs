use jiff::civil::Date;
use unbound_datetime::Month;
use unbound_toolkit::{Point, Radius, Rect, TextAlign, TextRole, TextStyle};

use super::paint::calendar_color;
use super::painter::Painter;
use super::{Hit, L10N, ViewModel};
use crate::store::AccountKind;

const INSET: f32 = 10.0;
const ROW: f32 = 26.0;
const HEADING: f32 = 28.0;
const CHECK: f32 = 14.0;
const MINI_CELL: f32 = 26.0;
const MINI_HEADER: f32 = 28.0;

pub fn paint(
    model: &ViewModel,
    mini_month: Date,
    status: Option<&str>,
    area: Rect,
    painter: &mut Painter<Hit>,
) {
    let palette = model.palette;
    let mini_height = MINI_HEADER + 18.0 + MINI_CELL * 6.0;
    let mini = Rect::new(
        area.x + INSET,
        area.y + area.height - mini_height - 24.0,
        area.width - 2.0 * INSET,
        mini_height,
    );
    let mut y = area.y + 6.0;
    for account in &model.library.accounts {
        let calendars: Vec<usize> = model
            .library
            .calendars
            .iter()
            .enumerate()
            .filter(|(_, calendar)| calendar.info.account == account.id)
            .map(|(index, _)| index)
            .collect();
        if calendars.is_empty() || y + HEADING > mini.y {
            continue;
        }
        let heading = match account.kind {
            AccountKind::Local => L10N.tr("on-my-computer"),
            AccountKind::Subscription { .. } => L10N.tr("subscriptions"),
            AccountKind::CalDav { .. } => account.name.clone(),
        };
        let heading_rect = Rect::new(
            area.x + INSET + 8.0,
            y + 4.0,
            area.width - 2.0 * INSET - 8.0,
            HEADING - 4.0,
        );
        painter.label(
            &heading,
            TextStyle::role(TextRole::Label, palette.ink3),
            heading_rect,
            TextAlign::Start,
        );
        painter.hit(heading_rect, Hit::Account(account.id.clone()));
        y += HEADING;
        for index in calendars {
            if y + ROW > mini.y {
                break;
            }
            let calendar = &model.library.calendars[index];
            let row = Rect::new(area.x + INSET, y, area.width - 2.0 * INSET, ROW);
            if model.hover == Some(&Hit::Calendar(index)) {
                painter.fill(row, Radius::S, palette.well.with_opacity(0.5));
            }
            let color = calendar_color(model, index);
            let check = Rect::new(row.x + 8.0, row.y + (ROW - CHECK) / 2.0, CHECK, CHECK);
            match calendar.info.visible {
                true => {
                    painter.fill(check, Radius::XS, color);
                    painter.items.push(unbound_toolkit::Item::StrokePath {
                        path: unbound_toolkit::Path::through(&[
                            Point::new(check.x + 3.5, check.y + 7.5),
                            Point::new(check.x + 6.0, check.y + 10.0),
                            Point::new(check.x + 10.5, check.y + 4.5),
                        ]),
                        width: 1.8,
                        paint: palette.on_accent.into(),
                    });
                }
                false => painter.stroke(check.inset(0.75), Radius::XS, 1.5, color),
            }
            painter.label(
                &calendar.info.name,
                TextStyle::role(TextRole::Body, palette.ink),
                Rect::new(check.x + CHECK + 8.0, row.y, row.width - CHECK - 24.0, ROW),
                TextAlign::Start,
            );
            painter.hit(row, Hit::Calendar(index));
            y += ROW;
        }
        y += 6.0;
    }
    paint_mini_month(model, mini_month, mini, painter);
    if let Some(status) = status {
        painter.label(
            status,
            TextStyle::role(TextRole::Callout, palette.ink3),
            Rect::new(
                area.x + INSET + 8.0,
                area.y + area.height - 22.0,
                area.width - 2.0 * INSET - 8.0,
                18.0,
            ),
            TextAlign::Start,
        );
    }
}

fn paint_mini_month(model: &ViewModel, first: Date, frame: Rect, painter: &mut Painter<Hit>) {
    let palette = model.palette;
    let month = Month::containing(first, model.formats.first_weekday());
    let title_rect = Rect::new(frame.x + 6.0, frame.y, frame.width - 64.0, MINI_HEADER);
    painter.label(
        &model.formats.month_title(first),
        TextStyle::role(TextRole::BodyStrong, palette.ink),
        title_rect,
        TextAlign::Start,
    );
    for (offset, label, shift) in [(52.0, "‹", -1), (26.0, "›", 1)] {
        let button = Rect::new(
            frame.x + frame.width - offset,
            frame.y + 2.0,
            24.0,
            MINI_HEADER - 4.0,
        );
        if model.hover == Some(&Hit::MiniShift(shift)) {
            painter.fill(button, Radius::S, palette.well);
        }
        painter.label(
            label,
            TextStyle::role(TextRole::Heading, palette.ink2),
            button,
            TextAlign::Center,
        );
        painter.hit(button, Hit::MiniShift(shift));
    }
    let cell_width = frame.width / 7.0;
    let (focus_first, focus_last) = super::paint::period(model.mode, model.focus, model.formats);
    if let Some(week) = month.weeks.first() {
        for (column, day) in week.days.iter().enumerate() {
            let initial: String = model.formats.short_weekday(*day).chars().take(2).collect();
            painter.label(
                &initial,
                TextStyle::new(10.0, palette.ink3).weight(TextStyle::SEMIBOLD),
                Rect::new(
                    frame.x + cell_width * column as f32,
                    frame.y + MINI_HEADER,
                    cell_width,
                    18.0,
                ),
                TextAlign::Center,
            );
        }
    }
    let today = model.now.date();
    for (row, week) in month.weeks.iter().enumerate() {
        let top = frame.y + MINI_HEADER + 18.0 + MINI_CELL * row as f32;
        let in_focus = |day: &Date| *day >= focus_first && *day < focus_last;
        if matches!(model.mode, super::Mode::Week) && week.days.iter().any(in_focus) {
            painter.fill(
                Rect::new(frame.x, top + 1.0, frame.width, MINI_CELL - 2.0),
                Radius::S,
                palette.well,
            );
        }
        for (column, day) in week.days.iter().enumerate() {
            let cell = Rect::new(
                frame.x + cell_width * column as f32,
                top,
                cell_width,
                MINI_CELL,
            );
            let size = MINI_CELL - 4.0;
            let badge = Rect::new(cell.x + (cell.width - size) / 2.0, cell.y + 2.0, size, size);
            let ink = match (
                *day == today,
                *day == model.focus && !matches!(model.mode, super::Mode::Week | super::Mode::Year),
            ) {
                (true, _) => {
                    painter.fill(badge, size / 2.0, palette.danger);
                    palette.on_danger
                }
                (false, true) => {
                    painter.fill(badge, size / 2.0, palette.well);
                    palette.ink
                }
                _ if month.contains(*day) => palette.ink,
                _ => palette.ink3,
            };
            painter.label(
                &day.day().to_string(),
                TextStyle::new(11.5, ink).tabular(),
                badge,
                TextAlign::Center,
            );
            painter.hit(cell, Hit::MiniDay(*day));
        }
    }
}
