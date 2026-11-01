use jiff::ToSpan;
use jiff::civil::{Date, DateTime, Time};

use bande_toolkit::{Point, Rect};

pub const HOUR_HEIGHT: f32 = 48.0;
pub const GUTTER: f32 = 56.0;
pub const DAY_HEADER: f32 = 44.0;
pub const ALL_DAY_ROW: f32 = 22.0;
pub const SNAP_MINUTES: i32 = 15;
pub const MINUTES_PER_DAY: i32 = 24 * 60;
pub const MIN_LANE: f32 = 64.0;
const SHORTEST: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeGrid {
    pub area: Rect,
    pub first: Date,
    pub days: usize,
    pub all_day_rows: usize,
    pub scroll: f32,
}

impl TimeGrid {
    pub fn all_day_height(&self) -> f32 {
        ALL_DAY_ROW * self.all_day_rows.max(1) as f32 + 6.0
    }

    pub fn hours_top(&self) -> f32 {
        self.area.y + DAY_HEADER + self.all_day_height()
    }

    pub fn hours_area(&self) -> Rect {
        let top = self.hours_top();
        Rect::new(
            self.area.x + GUTTER,
            top,
            (self.area.width - GUTTER).max(0.0),
            (self.area.y + self.area.height - top).max(0.0),
        )
    }

    pub fn content_height(&self) -> f32 {
        HOUR_HEIGHT * 24.0
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height() - self.hours_area().height).max(0.0)
    }

    pub fn column_width(&self) -> f32 {
        self.hours_area().width / self.days.max(1) as f32
    }

    pub fn column(&self, index: usize) -> Rect {
        let hours = self.hours_area();
        let width = self.column_width();
        Rect::new(hours.x + width * index as f32, hours.y, width, hours.height)
    }

    pub fn day(&self, index: usize) -> Date {
        self.first.saturating_add((index as i64).days())
    }

    pub fn day_index(&self, day: Date) -> Option<usize> {
        let offset = (day - self.first).get_days();
        usize::try_from(offset)
            .ok()
            .filter(|offset| *offset < self.days)
    }

    pub fn y_for_minutes(&self, minutes: i32) -> f32 {
        self.hours_top() - self.scroll + minutes as f32 * HOUR_HEIGHT / 60.0
    }

    pub fn minutes_at(&self, y: f32) -> i32 {
        let minutes = (y - self.hours_top() + self.scroll) * 60.0 / HOUR_HEIGHT;
        (minutes.round() as i32).clamp(0, MINUTES_PER_DAY)
    }

    pub fn column_at(&self, x: f32) -> Option<usize> {
        let hours = self.hours_area();
        if x < hours.x || x >= hours.x + hours.width {
            return None;
        }
        Some((((x - hours.x) / self.column_width()) as usize).min(self.days.saturating_sub(1)))
    }

    pub fn clamped_column(&self, x: f32) -> usize {
        let hours = self.hours_area();
        let index = ((x - hours.x) / self.column_width()).floor();
        (index.max(0.0) as usize).min(self.days.saturating_sub(1))
    }

    pub fn at(&self, point: Point) -> Option<DateTime> {
        let column = self.column_at(point.x)?;
        Some(at_minutes(self.day(column), snap(self.minutes_at(point.y))))
    }

    pub fn all_day_area(&self) -> Rect {
        Rect::new(
            self.area.x + GUTTER,
            self.area.y + DAY_HEADER,
            (self.area.width - GUTTER).max(0.0),
            self.all_day_height(),
        )
    }

    pub fn lanes_that_fit(&self) -> usize {
        ((self.column_width() - 6.0) / MIN_LANE).floor().max(2.0) as usize
    }

    pub fn block(&self, column: usize, start: i32, end: i32, slot: Slot) -> Rect {
        let day = self.column(column);
        let width = (day.width - 6.0) / slot.lanes.max(1) as f32;
        let (lane, span) = (slot.lane, slot.span.max(1));
        let top = self.y_for_minutes(start);
        let bottom = self.y_for_minutes(end.max(start + 20));
        Rect::new(
            day.x + 1.0 + width * lane as f32,
            top + 1.0,
            (width * span as f32 - 1.0).max(4.0),
            (bottom - top - 2.0).max(10.0),
        )
    }
}

pub fn snap(minutes: i32) -> i32 {
    ((minutes as f32 / SNAP_MINUTES as f32).round() as i32 * SNAP_MINUTES).clamp(0, MINUTES_PER_DAY)
}

pub fn at_minutes(day: Date, minutes: i32) -> DateTime {
    let minutes = minutes.clamp(0, MINUTES_PER_DAY);
    match minutes == MINUTES_PER_DAY {
        true => day.saturating_add(1.day()).to_datetime(Time::midnight()),
        false => day
            .to_datetime(Time::constant(0, 0, 0, 0))
            .saturating_add((minutes as i64).minutes()),
    }
}

pub fn minutes_of(at: DateTime) -> i32 {
    i32::from(at.hour()) * 60 + i32::from(at.minute())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub lane: usize,
    pub span: usize,
    pub lanes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct More {
    pub slot: Slot,
    pub start: i32,
    pub end: i32,
    pub count: usize,
}

fn overlap(a: (i32, i32), b: (i32, i32)) -> bool {
    a.0 < b.1.max(b.0 + SHORTEST) && b.0 < a.1.max(a.0 + SHORTEST)
}

fn clusters(spans: &[(i32, i32)]) -> Vec<Vec<(usize, usize)>> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|index| (spans[*index].0, -spans[*index].1));
    let mut found: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut lane_ends: Vec<i32> = Vec::new();
    let mut cluster_end = i32::MIN;
    for index in order {
        let (start, end) = spans[index];
        let end = end.max(start + SHORTEST);
        if start >= cluster_end || found.is_empty() {
            found.push(Vec::new());
            lane_ends.clear();
            cluster_end = end;
        }
        let lane = match lane_ends.iter().position(|lane_end| *lane_end <= start) {
            Some(lane) => {
                lane_ends[lane] = end;
                lane
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        cluster_end = cluster_end.max(end);
        if let Some(cluster) = found.last_mut() {
            cluster.push((index, lane));
        }
    }
    found
}

pub fn arrange(spans: &[(i32, i32)], fits: usize) -> (Vec<Option<Slot>>, Vec<More>) {
    let mut slots = vec![None; spans.len()];
    let mut more = Vec::new();
    for cluster in clusters(spans) {
        let lanes = cluster.iter().map(|(_, lane)| lane + 1).max().unwrap_or(1);
        let shown = match lanes > fits {
            true => fits.saturating_sub(1).max(1),
            false => lanes,
        };
        let columns = match lanes > shown {
            true => shown + 1,
            false => shown,
        };
        let hidden: Vec<usize> = cluster
            .iter()
            .filter(|(_, lane)| *lane >= shown)
            .map(|(index, _)| *index)
            .collect();
        if !hidden.is_empty() {
            more.push(More {
                slot: Slot {
                    lane: shown,
                    span: 1,
                    lanes: columns,
                },
                start: hidden
                    .iter()
                    .map(|index| spans[*index].0)
                    .min()
                    .unwrap_or(0),
                end: hidden
                    .iter()
                    .map(|index| spans[*index].1.max(spans[*index].0 + SHORTEST))
                    .max()
                    .unwrap_or(0),
                count: hidden.len(),
            });
        }
        for (index, lane) in cluster.iter().filter(|(_, lane)| *lane < shown) {
            let blocked = |other_lane: usize| {
                cluster.iter().any(|(other, placed)| {
                    *placed == other_lane && overlap(spans[*index], spans[*other])
                })
            };
            let span = (lane + 1..shown).take_while(|next| !blocked(*next)).count() + 1;
            slots[*index] = Some(Slot {
                lane: *lane,
                span,
                lanes: columns,
            });
        }
    }
    (slots, more)
}

pub fn stack_rows(spans: &[(usize, usize)]) -> (Vec<usize>, usize) {
    let mut rows: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|index| (spans[*index].0, usize::MAX - spans[*index].1));
    let mut assigned = vec![0; spans.len()];
    for index in order {
        let (first, last) = spans[index];
        let row = rows.iter().position(|row| {
            row.iter()
                .all(|(other_first, other_last)| last < *other_first || first > *other_last)
        });
        let row = match row {
            Some(row) => row,
            None => {
                rows.push(Vec::new());
                rows.len() - 1
            }
        };
        rows[row].push((first, last));
        assigned[index] = row;
    }
    (assigned, rows.len())
}

pub fn month_cell(area: Rect, header: f32, weeks: usize, week: usize, day: usize) -> Rect {
    let width = area.width / 7.0;
    let height = (area.height - header) / weeks.max(1) as f32;
    Rect::new(
        area.x + width * day as f32,
        area.y + header + height * week as f32,
        width,
        height,
    )
}
