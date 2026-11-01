use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::ical::Trigger;
use crate::store::Library;

const LOOKAHEAD: SignedDuration = SignedDuration::from_hours(24 * 15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub key: String,
    pub at: Timestamp,
    pub title: String,
    pub starts: Timestamp,
    pub all_day: bool,
    pub location: String,
}

pub fn due_between(library: &Library, from: Timestamp, to: Timestamp, zone: &TimeZone) -> Vec<Due> {
    let window_start = from.saturating_sub(LOOKAHEAD).unwrap_or(from);
    let window_end = to.saturating_add(LOOKAHEAD).unwrap_or(to);
    let mut due = Vec::new();
    for found in library.occurrences(window_start, window_end, zone) {
        let Some(event) = library.event(&found) else {
            continue;
        };
        let calendar = &library.calendars[found.calendar].info;
        for (index, alarm) in event.alarms.iter().enumerate() {
            if !["DISPLAY", "AUDIO"]
                .iter()
                .any(|action| alarm.action.eq_ignore_ascii_case(action))
            {
                continue;
            }
            let at = match &alarm.trigger {
                Trigger::Relative { offset, from_end } => {
                    let base = match from_end {
                        true => &found.occurrence.end,
                        false => &found.occurrence.start,
                    };
                    base.checked_add(offset.span())
                        .map(|at| at.timestamp())
                        .ok()
                }
                Trigger::Absolute(at) if !found.occurrence.recurring => Some(*at),
                Trigger::Absolute(_) => None,
            };
            let Some(at) = at.filter(|at| *at >= from && *at < to) else {
                continue;
            };
            due.push(Due {
                key: format!(
                    "{}|{}|{:?}|{index}",
                    calendar.id, found.occurrence.uid, found.occurrence.key
                ),
                at,
                title: event.summary.clone(),
                starts: found.occurrence.start.timestamp(),
                all_day: found.occurrence.all_day,
                location: event.location.clone(),
            });
        }
    }
    due.sort_by_key(|due| due.at);
    due
}
