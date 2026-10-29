use std::time::{Duration, Instant};

use unbound_toolkit::Point;

const INTERVAL: Duration = Duration::from_millis(400);
const SLOP: f32 = 4.0;
const MOST: u8 = 3;

#[derive(Debug, Default)]
pub struct ClickCounter {
    last: Option<(Instant, Point, u8)>,
}

impl ClickCounter {
    pub fn press(&mut self, at: Point, now: Instant) -> u8 {
        let count = match self.last {
            Some((when, place, count))
                if now.duration_since(when) <= INTERVAL
                    && (at.x - place.x).abs() <= SLOP
                    && (at.y - place.y).abs() <= SLOP =>
            {
                count % MOST + 1
            }
            _ => 1,
        };
        self.last = Some((now, at, count));
        count
    }
}
