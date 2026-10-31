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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{Alert, new_event};
    use crate::ical::{Frequency, Rule};
    use jiff::civil::date;

    #[test]
    fn alarms_fire_for_each_occurrence_and_all_day_events() {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::load(directory.path().to_owned());
        let zone = TimeZone::get("Europe/Berlin").unwrap();
        let mut daily = new_event(
            date(2026, 10, 24).at(9, 0, 0, 0),
            date(2026, 10, 24).at(9, 30, 0, 0),
            false,
            "Standup",
            &zone,
        );
        let master = daily.master_mut().unwrap();
        master.rule = Some(Rule::new(Frequency::Daily));
        master.alarms = Alert::Minutes(10).alarms(&[]);
        library.put_object(0, daily);
        let mut holiday = new_event(
            date(2026, 10, 26).at(0, 0, 0, 0),
            date(2026, 10, 27).at(0, 0, 0, 0),
            true,
            "Holiday",
            &zone,
        );
        holiday.master_mut().unwrap().alarms = Alert::DayAt {
            days_before: 1,
            hour: 9,
        }
        .alarms(&[]);
        library.put_object(0, holiday);
        let from = date(2026, 10, 24)
            .at(12, 0, 0, 0)
            .to_zoned(zone.clone())
            .unwrap()
            .timestamp();
        let to = date(2026, 10, 26)
            .at(12, 0, 0, 0)
            .to_zoned(zone.clone())
            .unwrap()
            .timestamp();
        let shown: Vec<String> = due_between(&library, from, to, &zone)
            .iter()
            .map(|due| {
                format!(
                    "{} {}",
                    due.at.to_zoned(zone.clone()).strftime("%d %H:%M"),
                    due.title
                )
            })
            .collect();
        assert_eq!(
            shown,
            ["25 08:50 Standup", "25 09:00 Holiday", "26 08:50 Standup"]
        );
    }
}
