use std::collections::HashSet;

use bande_datetime::{DateFormats, Month};
use bande_toolkit::{Color, Point, Radius, Rect, Scheme, Shape, TextAlign, TextRole, TextStyle};
use jiff::Timestamp;
use jiff::ToSpan;
use jiff::civil::{Date, DateTime, Time};

use super::geometry::{
    self, DAY_HEADER, GUTTER, Slot, TimeGrid, arrange, at_minutes, minutes_of, month_cell,
    stack_rows,
};
use super::painter::{Painter, parse_color};
use super::{Drag, Hit, L10N, Mode, Part, ViewModel};
use crate::store::Found;

const MONTH_HEADER: f32 = 28.0;
const MONTH_LINE: f32 = 18.0;
const MONTH_DAY_TOP: f32 = 26.0;
const RESIZE_HANDLE: f32 = 6.0;
const RESULTS_WIDTH: f32 = 300.0;
const RESULT_ROW: f32 = 52.0;
const YEAR_COLUMNS: usize = 4;
const YEAR_ROWS: usize = 3;

type ViewPainter<'a> = Painter<'a, Hit>;

pub fn period(mode: Mode, focus: Date, formats: &DateFormats) -> (Date, Date) {
    match mode {
        Mode::Day => (focus, focus.saturating_add(1.day())),
        Mode::Week => {
            let back = i64::from(focus.weekday().since(formats.first_weekday()));
            let first = focus.saturating_sub(back.days());
            (first, first.saturating_add(7.days()))
        }
        Mode::Month => {
            let month = Month::containing(focus, formats.first_weekday());
            let first = month.weeks.first().map_or(focus, |week| week.days[0]);
            (
                first,
                first.saturating_add((7 * month.weeks.len() as i64).days()),
            )
        }
        Mode::Year => {
            let first = focus
                .first_of_month()
                .with()
                .month(1)
                .build()
                .unwrap_or(focus);
            (first, first.saturating_add(1.year()))
        }
    }
}

fn content_area(model: &ViewModel) -> Rect {
    let sheet = model.sheet;
    let results = match model.search.trim().is_empty() {
        true => 0.0,
        false => RESULTS_WIDTH.min(sheet.width / 2.0),
    };
    Rect::new(
        sheet.x,
        sheet.y,
        (sheet.width - results).max(0.0),
        sheet.height,
    )
}

pub fn time_grid(
    mode: Mode,
    focus: Date,
    formats: &DateFormats,
    area: Rect,
    scroll: f32,
    all_day_rows: usize,
) -> Option<TimeGrid> {
    let days = match mode {
        Mode::Day => 1,
        Mode::Week => 7,
        Mode::Month | Mode::Year => return None,
    };
    Some(TimeGrid {
        area,
        first: period(mode, focus, formats).0,
        days,
        all_day_rows,
        scroll,
    })
}

pub fn calendar_color(model: &ViewModel, calendar: usize) -> Color {
    model
        .library
        .calendars
        .get(calendar)
        .and_then(|calendar| parse_color(&calendar.info.color))
        .unwrap_or(model.palette.accent)
}

fn text_on(model: &ViewModel, color: Color) -> Color {
    match model.palette.scheme {
        Scheme::Light => color.mix(model.palette.ink, 0.45),
        Scheme::Dark => color.mix(Color::WHITE, 0.45),
    }
}

fn fill_of(model: &ViewModel, color: Color, selected: bool) -> Color {
    match (selected, model.palette.scheme) {
        (true, _) => color,
        (false, Scheme::Light) => color.with_opacity(0.18),
        (false, Scheme::Dark) => color.with_opacity(0.32),
    }
}

fn instant(day: Date, model: &ViewModel) -> Timestamp {
    day.to_zoned(model.zone.clone())
        .map(|at| at.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

fn summary(model: &ViewModel, found: &Found) -> String {
    let summary = model
        .library
        .event(found)
        .map(|event| event.summary.clone())
        .unwrap_or_default();
    match summary.trim().is_empty() {
        true => L10N.tr("new-event"),
        false => summary,
    }
}

fn time_label(model: &ViewModel, at: &jiff::Zoned) -> String {
    let (time, period) = model.formats.time(at);
    match period {
        Some(period) => format!("{time} {period}"),
        None => time,
    }
}

fn read_only(model: &ViewModel, found: &Found) -> bool {
    model
        .library
        .calendars
        .get(found.calendar)
        .is_none_or(|calendar| calendar.info.read_only)
}

pub fn view(model: &ViewModel, painter: &mut ViewPainter) -> (Vec<Found>, Option<TimeGrid>) {
    let area = content_area(model);
    let (first, last) = period(model.mode, model.focus, model.formats);
    let shown = model
        .library
        .occurrences(instant(first, model), instant(last, model), model.zone);
    let grid = match model.mode {
        Mode::Day | Mode::Week => paint_week(model, &shown, area, painter),
        Mode::Month => {
            paint_month(model, &shown, area, painter);
            None
        }
        Mode::Year => {
            paint_year(model, &shown, area, painter);
            None
        }
    };
    if !model.search.trim().is_empty() {
        paint_results(model, area, painter);
    }
    (shown, grid)
}

fn preview_of(model: &ViewModel, found: &Found) -> Option<(DateTime, DateTime, bool)> {
    match model.drag? {
        Drag::Move {
            found: moving,
            grab,
            current,
            all_day,
            moved: true,
        } if moving == found => {
            let start = found.occurrence.start.datetime();
            let end = found.occurrence.end.datetime();
            let (start, end) = crate::edit::shifted(*grab, *current, start, end);
            match (found.occurrence.all_day, *all_day) {
                (true, true) | (false, false) => Some((start, end, *all_day)),
                (false, true) => Some((
                    start.date().to_datetime(Time::midnight()),
                    start.date().tomorrow().ok()?.to_datetime(Time::midnight()),
                    true,
                )),
                (true, false) => Some((*current, current.checked_add(1.hour()).ok()?, false)),
            }
        }
        Drag::Resize {
            found: resizing,
            end,
        } if resizing == found => {
            let start = found.occurrence.start.datetime();
            Some((
                start,
                (*end).max(start.checked_add(15.minutes()).ok()?),
                false,
            ))
        }
        _ => None,
    }
}

fn day_header(model: &ViewModel, grid: &TimeGrid, painter: &mut ViewPainter) {
    let palette = model.palette;
    let today = model.now.date();
    for index in 0..grid.days {
        let day = grid.day(index);
        let column = grid.column(index);
        let frame = Rect::new(column.x, grid.area.y, column.width, DAY_HEADER);
        let weekday = model.formats.short_weekday(day);
        let number = day.day().to_string();
        let weekday_style = TextStyle::role(TextRole::Callout, palette.ink2);
        let number_style =
            TextStyle::role(TextRole::Heading, palette.ink).weight(TextStyle::MEDIUM);
        let weekday_width = painter.measure(&weekday, weekday_style);
        let number_width = painter.measure(&number, number_style);
        let total = weekday_width + 6.0 + number_width.max(26.0);
        let x = match grid.days {
            1 => frame.x + 12.0,
            _ => frame.x + ((frame.width - total) / 2.0).round(),
        };
        painter.label(
            &weekday,
            weekday_style,
            Rect::new(x, frame.y, weekday_width, frame.height),
            TextAlign::Start,
        );
        let badge = Rect::new(
            x + weekday_width + 6.0,
            frame.y + (frame.height - 26.0) / 2.0,
            number_width.max(26.0),
            26.0,
        );
        let number_color = match day == today {
            true => {
                painter.fill(badge, 13.0, palette.danger);
                palette.on_danger
            }
            false => palette.ink,
        };
        painter.label(
            &number,
            TextStyle {
                color: number_color,
                ..number_style
            },
            badge,
            TextAlign::Center,
        );
        painter.hit(frame, Hit::MonthDay(day));
    }
    painter.line(
        Point::new(grid.area.x, grid.area.y + DAY_HEADER - 0.5),
        Point::new(
            grid.area.x + grid.area.width,
            grid.area.y + DAY_HEADER - 0.5,
        ),
        1.0,
        palette.hairline,
    );
}

fn event_block(
    model: &ViewModel,
    painter: &mut ViewPainter,
    rect: Rect,
    color: Color,
    selected: bool,
    title: &str,
    time: Option<&str>,
) {
    painter.fill(rect, Radius::XS, fill_of(model, color, selected));
    painter.fill(Rect::new(rect.x, rect.y, 3.0, rect.height), 1.5, color);
    let ink = match selected {
        true => Color::WHITE,
        false => text_on(model, color),
    };
    let title_style = TextStyle::new(12.0, ink).weight(TextStyle::SEMIBOLD);
    let time_style = TextStyle::new(11.0, ink.with_opacity(0.85));
    if rect.height < 30.0 {
        let line = Rect::new(
            rect.x + 7.0,
            rect.y,
            (rect.width - 9.0).max(0.0),
            rect.height,
        );
        let title_width = painter.measure(title, title_style).min(line.width);
        painter.label(
            title,
            title_style,
            Rect::new(line.x, line.y, title_width, line.height),
            TextAlign::Start,
        );
        if let Some(time) = time {
            let rest = Rect::new(
                line.x + title_width + 5.0,
                line.y,
                (line.width - title_width - 5.0).max(0.0),
                line.height,
            );
            painter.label(time, time_style, rest, TextAlign::Start);
        }
        return;
    }
    let inner = Rect::new(
        rect.x + 7.0,
        rect.y + 2.0,
        (rect.width - 9.0).max(0.0),
        (rect.height - 4.0).max(0.0),
    );
    painter.clipped(Shape::rect(inner), |painter| {
        painter.wrapped(title, title_style, inner);
        if let Some(time) = time {
            let lines = painter
                .text
                .paragraph(title, title_style, Some(inner.width), TextAlign::Start)
                .height();
            let y = (inner.y + lines).min(inner.y + inner.height - 14.0);
            painter.label(
                time,
                time_style,
                Rect::new(inner.x, y, inner.width, 15.0),
                TextAlign::Start,
            );
        }
    });
}

fn paint_week(
    model: &ViewModel,
    shown: &[Found],
    area: Rect,
    painter: &mut ViewPainter,
) -> Option<TimeGrid> {
    let palette = model.palette;
    let base = time_grid(
        model.mode,
        model.focus,
        model.formats,
        area,
        model.scroll,
        1,
    )?;
    let mut bars: Vec<(usize, usize, usize)> = Vec::new();
    for (index, found) in shown.iter().enumerate() {
        let (first, last, all_day) = match preview_of(model, found) {
            Some((start, end, true)) => (
                start.date(),
                end.date()
                    .yesterday()
                    .unwrap_or(start.date())
                    .max(start.date()),
                true,
            ),
            Some(_) => continue,
            None => (
                found.occurrence.first_day,
                found.occurrence.last_day,
                found.occurrence.all_day,
            ),
        };
        if !all_day {
            continue;
        }
        let from = (first - base.first).get_days().max(0) as usize;
        let to = ((last - base.first).get_days().min(base.days as i32 - 1)).max(0) as usize;
        if (last - base.first).get_days() >= 0 && (first - base.first).get_days() < base.days as i32
        {
            bars.push((index, from, to));
        }
    }
    let creating_all_day = matches!(model.drag, Some(Drag::Create { all_day: true, .. }));
    let (rows, row_count) = stack_rows(
        &bars
            .iter()
            .map(|(_, from, to)| (*from, *to))
            .collect::<Vec<_>>(),
    );
    let grid = TimeGrid {
        all_day_rows: row_count + usize::from(creating_all_day),
        ..base
    };
    day_header(model, &grid, painter);
    let band = grid.all_day_area();
    painter.label(
        &L10N.tr("all-day"),
        TextStyle::role(TextRole::Callout, palette.ink3),
        Rect::new(grid.area.x, band.y, GUTTER - 8.0, geometry::ALL_DAY_ROW),
        TextAlign::End,
    );
    for index in 0..grid.days {
        let column = grid.column(index);
        painter.hit(
            Rect::new(column.x, band.y, column.width, band.height),
            Hit::AllDay(grid.day(index)),
        );
    }
    for ((index, from, to), row) in bars.iter().zip(&rows) {
        let found = &shown[*index];
        let width = grid.column_width();
        let rect = Rect::new(
            band.x + width * *from as f32 + 2.0,
            band.y + 3.0 + geometry::ALL_DAY_ROW * *row as f32,
            width * (to - from + 1) as f32 - 4.0,
            geometry::ALL_DAY_ROW - 2.0,
        );
        let color = calendar_color(model, found.calendar);
        let selected = model.is_selected(found);
        event_block(
            model,
            painter,
            rect,
            color,
            selected,
            &summary(model, found),
            None,
        );
        painter.hit(rect, Hit::Event(*index, Part::Body));
    }
    if let Some(Drag::Create {
        anchor,
        current,
        all_day: true,
    }) = model.drag
    {
        let (from, to) = (
            (*anchor).min(*current).date(),
            (*anchor).max(*current).date(),
        );
        if let (Some(from), Some(to)) = (grid.day_index(from), grid.day_index(to)) {
            let width = grid.column_width();
            let rect = Rect::new(
                band.x + width * from as f32 + 2.0,
                band.y + 3.0 + geometry::ALL_DAY_ROW * row_count as f32,
                width * (to - from + 1) as f32 - 4.0,
                geometry::ALL_DAY_ROW - 2.0,
            );
            event_block(
                model,
                painter,
                rect,
                palette.accent,
                false,
                &L10N.tr("new-event"),
                None,
            );
        }
    }
    painter.line(
        Point::new(grid.area.x, band.y + band.height - 0.5),
        Point::new(grid.area.x + grid.area.width, band.y + band.height - 0.5),
        1.0,
        palette.hairline,
    );
    let hours = grid.hours_area();
    let visible = Rect::new(grid.area.x, hours.y, grid.area.width, hours.height);
    painter.clipped(Shape::rect(visible), |painter| {
        paint_hours(model, &grid, shown, painter);
    });
    Some(grid)
}

fn paint_hours(model: &ViewModel, grid: &TimeGrid, shown: &[Found], painter: &mut ViewPainter) {
    let palette = model.palette;
    let hours = grid.hours_area();
    let today = model.now.date();
    if let Some(index) = grid.day_index(today).filter(|_| grid.days > 1) {
        painter.fill(
            grid.column(index),
            0.0,
            palette.accent_soft.with_opacity(0.35),
        );
    }
    for hour in 0..=24 {
        let y = grid.y_for_minutes(hour * 60).round() + 0.5;
        if hour > 0 && hour < 24 {
            let label = at_minutes(today, hour * 60)
                .to_zoned(model.zone.clone())
                .map(|at| time_label(model, &at))
                .unwrap_or_default();
            painter.label(
                &label,
                TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
                Rect::new(grid.area.x, y - 8.0, GUTTER - 8.0, 16.0),
                TextAlign::End,
            );
        }
        painter.line(
            Point::new(hours.x, y),
            Point::new(hours.x + hours.width, y),
            1.0,
            palette.hairline,
        );
    }
    for index in 1..grid.days {
        let x = grid.column(index).x.round() + 0.5;
        painter.line(
            Point::new(x, hours.y),
            Point::new(x, hours.y + grid.content_height()),
            1.0,
            palette.hairline,
        );
    }
    for index in 0..grid.days {
        let day = grid.day(index);
        let mut spans: Vec<(usize, i32, i32)> = Vec::new();
        for (found_index, found) in shown.iter().enumerate() {
            let (start, end, all_day) = preview_of(model, found).unwrap_or((
                found.occurrence.start.datetime(),
                found.occurrence.end.datetime(),
                found.occurrence.all_day,
            ));
            if all_day
                || start.date() > day
                || end.date() < day
                || (end.date() == day && end.time() == Time::midnight() && end > start)
            {
                continue;
            }
            let from = match start.date() < day {
                true => 0,
                false => minutes_of(start),
            };
            let to = match end.date() > day {
                true => geometry::MINUTES_PER_DAY,
                false => minutes_of(end),
            };
            spans.push((found_index, from, to));
        }
        let (slots, more) = arrange(
            &spans
                .iter()
                .map(|(_, from, to)| (*from, *to))
                .collect::<Vec<_>>(),
            grid.lanes_that_fit(),
        );
        for ((found_index, from, to), slot) in spans.iter().zip(slots) {
            let Some(slot) = slot else {
                continue;
            };
            let found = &shown[*found_index];
            let rect = grid.block(index, *from, *to, slot);
            let color = calendar_color(model, found.calendar);
            let selected = model.is_selected(found);
            let (start, _, _) = preview_of(model, found).unwrap_or((
                found.occurrence.start.datetime(),
                found.occurrence.end.datetime(),
                false,
            ));
            let time = start
                .to_zoned(model.zone.clone())
                .map(|at| time_label(model, &at))
                .ok();
            event_block(
                model,
                painter,
                rect,
                color,
                selected,
                &summary(model, found),
                time.as_deref(),
            );
            painter.hit(rect, Hit::Event(*found_index, Part::Body));
            if !read_only(model, found) && rect.height > 3.0 * RESIZE_HANDLE {
                painter.hit(
                    Rect::new(
                        rect.x,
                        rect.y + rect.height - RESIZE_HANDLE,
                        rect.width,
                        RESIZE_HANDLE,
                    ),
                    Hit::Event(*found_index, Part::End),
                );
            }
        }
        for chip in more {
            let rect = grid.block(index, chip.start, chip.end, chip.slot);
            painter.fill(rect, Radius::XS, palette.well);
            painter.stroke(rect, Radius::XS, 1.0, palette.hairline);
            painter.label(
                &format!("+{}", chip.count),
                TextStyle::new(12.0, palette.ink2).weight(TextStyle::SEMIBOLD),
                Rect::new(rect.x, rect.y + 2.0, rect.width, 16.0),
                TextAlign::Center,
            );
            painter.hit(rect, Hit::More(day));
        }
    }
    if let Some(Drag::Create {
        anchor,
        current,
        all_day: false,
    }) = model.drag
    {
        let (start, end) = ((*anchor).min(*current), (*anchor).max(*current));
        let end = end.max(start.checked_add(15.minutes()).unwrap_or(end));
        if let Some(index) = grid.day_index(start.date()) {
            let to = match end.date() > start.date() {
                true => geometry::MINUTES_PER_DAY,
                false => minutes_of(end),
            };
            let rect = grid.block(
                index,
                minutes_of(start),
                to,
                Slot {
                    lane: 0,
                    span: 1,
                    lanes: 1,
                },
            );
            let label = start
                .to_zoned(model.zone.clone())
                .map(|at| time_label(model, &at))
                .ok();
            event_block(
                model,
                painter,
                rect,
                palette.accent,
                false,
                &L10N.tr("new-event"),
                label.as_deref(),
            );
        }
    }
    if let Some(index) = grid.day_index(today) {
        let y = grid.y_for_minutes(minutes_of(model.now.datetime()));
        let column = grid.column(index);
        let (from, to) = match grid.days {
            1 => (hours.x, hours.x + hours.width),
            _ => (column.x, column.x + column.width),
        };
        painter.line(
            Point::new(hours.x, y),
            Point::new(hours.x + hours.width, y),
            1.0,
            palette.danger.with_opacity(0.35),
        );
        painter.line(Point::new(from, y), Point::new(to, y), 2.0, palette.danger);
        painter.fill(
            Rect::new(from - 4.0, y - 4.0, 8.0, 8.0),
            4.0,
            palette.danger,
        );
    }
}

fn paint_month(model: &ViewModel, shown: &[Found], area: Rect, painter: &mut ViewPainter) {
    let palette = model.palette;
    let month = Month::containing(model.focus, model.formats.first_weekday());
    let weeks = month.weeks.len();
    let today = model.now.date();
    if let Some(week) = month.weeks.first() {
        for (index, day) in week.days.iter().enumerate() {
            let cell = month_cell(area, MONTH_HEADER, weeks, 0, index);
            painter.label(
                &model.formats.short_weekday(*day),
                TextStyle::role(TextRole::Callout, palette.ink2),
                Rect::new(cell.x + 8.0, area.y, cell.width - 16.0, MONTH_HEADER),
                TextAlign::End,
            );
        }
    }
    let lines =
        |cell: Rect| (((cell.height - MONTH_DAY_TOP - 4.0) / MONTH_LINE).floor() as usize).max(1);
    for (week_index, week) in month.weeks.iter().enumerate() {
        let first = week.days[0];
        let mut bars: Vec<(usize, usize, usize)> = Vec::new();
        let mut timed: Vec<Vec<usize>> = vec![Vec::new(); 7];
        for (index, found) in shown.iter().enumerate() {
            let (start_day, end_day, all_day) = match preview_of(model, found) {
                Some((start, end, all_day)) => (
                    start.date(),
                    match all_day {
                        true => end
                            .date()
                            .yesterday()
                            .unwrap_or(start.date())
                            .max(start.date()),
                        false => start.date(),
                    },
                    all_day,
                ),
                None => (
                    found.occurrence.first_day,
                    found.occurrence.last_day,
                    found.occurrence.all_day,
                ),
            };
            let from = (start_day - first).get_days();
            let to = (end_day - first).get_days();
            if to < 0 || from > 6 {
                continue;
            }
            match all_day || end_day > start_day {
                true => bars.push((index, from.max(0) as usize, to.min(6) as usize)),
                false => timed[from as usize].push(index),
            }
        }
        let (rows, _) = stack_rows(
            &bars
                .iter()
                .map(|(_, from, to)| (*from, *to))
                .collect::<Vec<_>>(),
        );
        let mut used = [0usize; 7];
        for (day_index, day) in week.days.iter().enumerate() {
            let cell = month_cell(area, MONTH_HEADER, weeks, week_index, day_index);
            painter.line(
                Point::new(cell.x, cell.y + 0.5),
                Point::new(cell.x + cell.width, cell.y + 0.5),
                1.0,
                palette.hairline,
            );
            if day_index > 0 {
                painter.line(
                    Point::new(cell.x + 0.5, cell.y),
                    Point::new(cell.x + 0.5, cell.y + cell.height),
                    1.0,
                    palette.hairline,
                );
            }
            if !month.contains(*day) {
                painter.fill(cell, 0.0, palette.well.with_opacity(0.35));
            }
            painter.hit(cell, Hit::MonthDay(*day));
            let number = match day.day() {
                1 => format!("{} {}", day.strftime("%b"), day.day()),
                _ => day.day().to_string(),
            };
            let style = TextStyle::role(
                TextRole::Body,
                match month.contains(*day) {
                    true => palette.ink,
                    false => palette.ink3,
                },
            )
            .weight(TextStyle::MEDIUM);
            let width = painter.measure(&number, style).max(14.0) + 12.0;
            let badge = Rect::new(cell.x + cell.width - width - 4.0, cell.y + 3.0, width, 22.0);
            let style = match *day == today {
                true => {
                    painter.fill(badge, 11.0, palette.danger);
                    TextStyle {
                        color: palette.on_danger,
                        ..style
                    }
                }
                false => style,
            };
            painter.label(&number, style, badge, TextAlign::Center);
        }
        for ((index, from, to), row) in bars.iter().zip(&rows) {
            let cell = month_cell(area, MONTH_HEADER, weeks, week_index, *from);
            if *row + 1 >= lines(cell) {
                continue;
            }
            for slot in &mut used[*from..=*to] {
                *slot = (*slot).max(row + 1);
            }
            let found = &shown[*index];
            let rect = Rect::new(
                cell.x + 2.0,
                cell.y + MONTH_DAY_TOP + MONTH_LINE * *row as f32,
                cell.width * (to - from + 1) as f32 - 4.0,
                MONTH_LINE - 2.0,
            );
            let color = calendar_color(model, found.calendar);
            let selected = model.is_selected(found);
            painter.fill(rect, Radius::XS, fill_of(model, color, selected));
            let ink = match selected {
                true => Color::WHITE,
                false => text_on(model, color),
            };
            painter.label(
                &summary(model, found),
                TextStyle::new(11.5, ink).weight(TextStyle::SEMIBOLD),
                Rect::new(rect.x + 6.0, rect.y, rect.width - 8.0, rect.height),
                TextAlign::Start,
            );
            painter.hit(rect, Hit::Event(*index, Part::Body));
        }
        for (day_index, indices) in timed.iter().enumerate() {
            let cell = month_cell(area, MONTH_HEADER, weeks, week_index, day_index);
            let capacity = lines(cell);
            let total = indices.len() + used[day_index];
            for (row, (position, index)) in (used[day_index]..).zip(indices.iter().enumerate()) {
                if row + 1 >= capacity && total > capacity {
                    let more = indices.len() - position;
                    painter.label(
                        &L10N.tr_args("more-events", [("count", more.into())]),
                        TextStyle::role(TextRole::Callout, palette.ink2),
                        Rect::new(
                            cell.x + 8.0,
                            cell.y + MONTH_DAY_TOP + MONTH_LINE * row as f32,
                            cell.width - 12.0,
                            MONTH_LINE,
                        ),
                        TextAlign::Start,
                    );
                    break;
                }
                let found = &shown[*index];
                let rect = Rect::new(
                    cell.x + 2.0,
                    cell.y + MONTH_DAY_TOP + MONTH_LINE * row as f32,
                    cell.width - 4.0,
                    MONTH_LINE - 1.0,
                );
                let color = calendar_color(model, found.calendar);
                let selected = model.is_selected(found);
                if selected {
                    painter.fill(rect, Radius::XS, color);
                }
                painter.fill(
                    Rect::new(rect.x + 4.0, rect.y + (rect.height - 7.0) / 2.0, 7.0, 7.0),
                    3.5,
                    match selected {
                        true => Color::WHITE,
                        false => color,
                    },
                );
                let ink = match selected {
                    true => Color::WHITE,
                    false => palette.ink,
                };
                let start = preview_of(model, found).map_or(
                    found.occurrence.start.clone(),
                    |(start, _, _)| {
                        start
                            .to_zoned(model.zone.clone())
                            .unwrap_or(found.occurrence.start.clone())
                    },
                );
                let time = time_label(model, &start);
                let time_style = TextStyle::new(
                    11.0,
                    match selected {
                        true => Color::WHITE,
                        false => palette.ink3,
                    },
                )
                .tabular();
                let time_width = match rect.width > 120.0 {
                    true => painter.measure(&time, time_style),
                    false => 0.0,
                };
                let text_rect = Rect::new(
                    rect.x + 15.0,
                    rect.y,
                    (rect.width - 19.0 - time_width - 4.0).max(0.0),
                    rect.height,
                );
                painter.label(
                    &summary(model, found),
                    TextStyle::new(11.5, ink),
                    text_rect,
                    TextAlign::Start,
                );
                painter.label(
                    match time_width > 0.0 {
                        true => &time,
                        false => "",
                    },
                    time_style,
                    Rect::new(
                        rect.x + rect.width - time_width - 4.0,
                        rect.y,
                        time_width,
                        rect.height,
                    ),
                    TextAlign::End,
                );
                painter.hit(rect, Hit::Event(*index, Part::Body));
            }
        }
    }
}

fn paint_year(model: &ViewModel, shown: &[Found], area: Rect, painter: &mut ViewPainter) {
    let palette = model.palette;
    let year_start = period(Mode::Year, model.focus, model.formats).0;
    let busy: HashSet<Date> = shown
        .iter()
        .flat_map(|found| {
            let mut days = Vec::new();
            let mut day = found.occurrence.first_day;
            while day <= found.occurrence.last_day && days.len() < 366 {
                days.push(day);
                day = day.saturating_add(1.day());
            }
            days
        })
        .collect();
    let today = model.now.date();
    let margin = 16.0;
    let width = (area.width - margin * (YEAR_COLUMNS as f32 + 1.0)) / YEAR_COLUMNS as f32;
    let height = (area.height - margin * (YEAR_ROWS as f32 + 1.0)) / YEAR_ROWS as f32;
    for index in 0..12 {
        let first = year_start.saturating_add((index as i64).months());
        let frame = Rect::new(
            area.x + margin + (width + margin) * (index % YEAR_COLUMNS) as f32,
            area.y + margin + (height + margin) * (index / YEAR_COLUMNS) as f32,
            width,
            height,
        );
        let current = first.year() == today.year() && first.month() == today.month();
        let title = first.strftime("%B").to_string();
        let title_rect = Rect::new(frame.x + 4.0, frame.y, frame.width, 24.0);
        painter.label(
            &title,
            TextStyle::role(
                TextRole::Heading,
                match current {
                    true => palette.danger,
                    false => palette.ink,
                },
            ),
            title_rect,
            TextAlign::Start,
        );
        painter.hit(title_rect, Hit::YearMonth(first));
        let month = Month::containing(first, model.formats.first_weekday());
        let cell_width = frame.width / 7.0;
        let cell_height = ((frame.height - 48.0) / 6.0).min(cell_width);
        if let Some(week) = month.weeks.first() {
            for (column, day) in week.days.iter().enumerate() {
                let initial: String = model.formats.short_weekday(*day).chars().take(1).collect();
                painter.label(
                    &initial,
                    TextStyle::new(10.0, palette.ink3).weight(TextStyle::SEMIBOLD),
                    Rect::new(
                        frame.x + cell_width * column as f32,
                        frame.y + 26.0,
                        cell_width,
                        16.0,
                    ),
                    TextAlign::Center,
                );
            }
        }
        for (row, week) in month.weeks.iter().enumerate() {
            for (column, day) in week.days.iter().enumerate() {
                if !month.contains(*day) {
                    continue;
                }
                let cell = Rect::new(
                    frame.x + cell_width * column as f32,
                    frame.y + 44.0 + cell_height * row as f32,
                    cell_width,
                    cell_height,
                );
                let size = cell_height.min(cell_width) - 2.0;
                let badge = Rect::new(
                    cell.x + (cell.width - size) / 2.0,
                    cell.y + (cell.height - size) / 2.0,
                    size,
                    size,
                );
                let ink = match (*day == today, busy.contains(day)) {
                    (true, _) => {
                        painter.fill(badge, size / 2.0, palette.danger);
                        palette.on_danger
                    }
                    (false, true) => {
                        painter.fill(badge, size / 2.0, palette.accent_soft);
                        palette.accent_text
                    }
                    (false, false) => palette.ink,
                };
                painter.label(
                    &day.day().to_string(),
                    TextStyle::new(11.0, ink).tabular(),
                    cell,
                    TextAlign::Center,
                );
                painter.hit(cell, Hit::YearDay(*day));
            }
        }
    }
}

fn paint_results(model: &ViewModel, area: Rect, painter: &mut ViewPainter) {
    let palette = model.palette;
    let sheet = model.sheet;
    let panel = Rect::new(
        area.x + area.width,
        sheet.y,
        sheet.x + sheet.width - area.x - area.width,
        sheet.height,
    );
    painter.fill(panel, 0.0, palette.sheet);
    painter.line(
        Point::new(panel.x + 0.5, panel.y),
        Point::new(panel.x + 0.5, panel.y + panel.height),
        1.0,
        palette.hairline,
    );
    let heading = match model.results.is_empty() {
        true => L10N.tr_args(
            "no-results",
            [("text", model.search.trim().to_owned().into())],
        ),
        false => L10N.tr_args(
            "search-results",
            [("text", model.search.trim().to_owned().into())],
        ),
    };
    painter.label(
        &heading,
        TextStyle::role(TextRole::Label, palette.ink3),
        Rect::new(panel.x + 16.0, panel.y + 8.0, panel.width - 32.0, 24.0),
        TextAlign::Start,
    );
    for (index, found) in model.results.iter().enumerate() {
        let row = Rect::new(
            panel.x + 8.0,
            panel.y + 36.0 + RESULT_ROW * index as f32,
            panel.width - 16.0,
            RESULT_ROW - 4.0,
        );
        if row.y > panel.y + panel.height {
            break;
        }
        let selected = model.is_selected(found);
        if selected {
            painter.fill(row, Radius::S, palette.accent_soft);
        }
        let color = calendar_color(model, found.calendar);
        painter.fill(Rect::new(row.x + 8.0, row.y + 10.0, 8.0, 8.0), 4.0, color);
        painter.label(
            &summary(model, found),
            TextStyle::role(TextRole::BodyStrong, palette.ink),
            Rect::new(row.x + 24.0, row.y + 4.0, row.width - 32.0, 20.0),
            TextAlign::Start,
        );
        let when = match found.occurrence.all_day {
            true => model.formats.date(&found.occurrence.start, true),
            false => format!(
                "{}, {}",
                model.formats.date(&found.occurrence.start, true),
                time_label(model, &found.occurrence.start)
            ),
        };
        painter.label(
            &when,
            TextStyle::role(TextRole::Callout, palette.ink2),
            Rect::new(row.x + 24.0, row.y + 24.0, row.width - 32.0, 18.0),
            TextAlign::Start,
        );
        painter.hit(row, Hit::Result(index));
    }
}
