use jiff::ToSpan;
use jiff::civil::{Date, DateTime, Time};

use unbound_toolkit::{Point, Rect};

pub const HOUR_HEIGHT: f32 = 48.0;
pub const GUTTER: f32 = 56.0;
pub const DAY_HEADER: f32 = 44.0;
pub const ALL_DAY_ROW: f32 = 22.0;
pub const SNAP_MINUTES: i32 = 15;
pub const MINUTES_PER_DAY: i32 = 24 * 60;

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

    pub fn block(&self, column: usize, start: i32, end: i32, lane: usize, lanes: usize) -> Rect {
        let day = self.column(column);
        let lanes = lanes.max(1) as f32;
        let width = (day.width - 6.0) / lanes;
        let top = self.y_for_minutes(start);
        let bottom = self.y_for_minutes(end.max(start + 20));
        Rect::new(
            day.x + 1.0 + width * lane as f32,
            top + 1.0,
            (width - 1.0).max(4.0),
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
pub struct Placed {
    pub lane: usize,
    pub lanes: usize,
}

pub fn lanes(spans: &[(i32, i32)]) -> Vec<Placed> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|index| (spans[*index].0, -spans[*index].1));
    let mut placed = vec![Placed { lane: 0, lanes: 1 }; spans.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<i32> = Vec::new();
    let mut cluster_end = i32::MIN;
    let finish = |cluster: &mut Vec<usize>, lane_ends: &mut Vec<i32>, placed: &mut Vec<Placed>| {
        for index in cluster.drain(..) {
            placed[index].lanes = lane_ends.len().max(1);
        }
        lane_ends.clear();
    };
    for index in order {
        let (start, end) = spans[index];
        let end = end.max(start + 20);
        if start >= cluster_end && !cluster.is_empty() {
            finish(&mut cluster, &mut lane_ends, &mut placed);
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
        placed[index].lane = lane;
        cluster.push(index);
        cluster_end = match cluster.len() {
            1 => end,
            _ => cluster_end.max(end),
        };
    }
    finish(&mut cluster, &mut lane_ends, &mut placed);
    placed
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

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn grid() -> TimeGrid {
        TimeGrid {
            area: Rect::new(0.0, 0.0, GUTTER + 700.0, 800.0),
            first: date(2026, 10, 26),
            days: 7,
            all_day_rows: 1,
            scroll: 8.0 * HOUR_HEIGHT,
        }
    }

    #[test]
    fn a_point_maps_to_a_snapped_day_and_time_and_back() {
        let grid = grid();
        let y = grid.y_for_minutes(9 * 60 + 37);
        let x = grid.column(3).x + 10.0;
        assert_eq!(
            grid.at(Point::new(x, y)),
            Some(date(2026, 10, 29).at(9, 30, 0, 0))
        );
        assert_eq!(grid.at(Point::new(GUTTER - 1.0, y)), None);
        assert_eq!(grid.day_index(date(2026, 11, 1)), Some(6));
        assert_eq!(grid.day_index(date(2026, 11, 2)), None);
        assert_eq!(
            at_minutes(date(2026, 10, 26), MINUTES_PER_DAY),
            date(2026, 10, 27).at(0, 0, 0, 0)
        );
    }

    #[test]
    fn overlapping_events_share_lanes_per_cluster() {
        let placed = lanes(&[(540, 600), (570, 660), (600, 630), (700, 760)]);
        assert_eq!(placed[0], Placed { lane: 0, lanes: 2 });
        assert_eq!(placed[1], Placed { lane: 1, lanes: 2 });
        assert_eq!(placed[2], Placed { lane: 0, lanes: 2 });
        assert_eq!(placed[3], Placed { lane: 0, lanes: 1 });
    }

    #[test]
    fn multi_day_bars_stack_without_overlap() {
        let (rows, count) = stack_rows(&[(0, 2), (1, 1), (3, 6), (2, 4)]);
        assert_eq!(count, 2);
        assert_eq!(rows[0], 0);
        assert_ne!(rows[1], rows[0]);
        assert_ne!(rows[3], rows[0]);
        assert_ne!(rows[3], rows[2]);
    }
}
