#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_clock::model::{Alarm, Clock, Days};
use jiff::Timestamp;
use jiff::tz::TimeZone;

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn next(alarm: &Alarm, after: &str, zone: &TimeZone) -> String {
    let next = alarm.next_after(at(after), zone).unwrap();
    next.to_zoned(zone.clone())
        .strftime("%a %Y-%m-%d %H:%M %:z")
        .to_string()
}

#[test]
fn repeating_alarms_keep_their_wall_time_across_weekends_and_daylight_saving_changes() {
    let stockholm = TimeZone::get("Europe/Stockholm").unwrap();
    let mut night = Alarm::new(1, 2, 30);
    night.days = Days::EVERY_DAY;
    assert_eq!(
        next(&night, "2026-03-28T23:00:00+01:00", &stockholm),
        "Sun 2026-03-29 03:30 +02:00"
    );
    assert_eq!(
        next(&night, "2026-03-29T03:30:00+02:00", &stockholm),
        "Mon 2026-03-30 02:30 +02:00"
    );
    assert_eq!(
        next(&night, "2026-10-25T00:00:00+02:00", &stockholm),
        "Sun 2026-10-25 02:30 +02:00"
    );
    assert_eq!(
        next(&night, "2026-10-25T02:30:00+02:00", &stockholm),
        "Mon 2026-10-26 02:30 +01:00"
    );
    let mut work = Alarm::new(2, 8, 0);
    work.days = Days::WEEKDAYS;
    assert_eq!(
        next(&work, "2026-10-30T09:00:00+01:00", &stockholm),
        "Mon 2026-11-02 08:00 +01:00"
    );
    let new_york = TimeZone::get("America/New_York").unwrap();
    assert_eq!(
        next(&work, "2026-10-30T09:00:00+01:00", &new_york),
        "Fri 2026-10-30 08:00 -04:00"
    );
}

#[test]
fn an_agent_started_late_rings_a_missed_alarm_once_but_not_a_stale_one() {
    let zone = TimeZone::get("Europe/Stockholm").unwrap();
    let once = Alarm::new(1, 7, 0);
    let mut stale = Alarm::new(2, 4, 0);
    stale.days = Days::EVERY_DAY;
    let mut clock = Clock::default();
    clock.alarms = vec![once, stale];
    clock.checked = Some(at("2026-11-01T22:00:00+01:00"));
    let now = at("2026-11-02T07:10:00+01:00");
    let due: Vec<u64> = clock
        .take_due_alarms(now, &zone)
        .iter()
        .map(|alarm| alarm.id)
        .collect();
    assert_eq!(due, [1]);
    assert!(!clock.alarms[0].enabled);
    assert!(clock.alarms[1].enabled);
    assert!(clock.take_due_alarms(now, &zone).is_empty());
    let (next, alarm) = clock.next_alarm(now, &zone).unwrap();
    assert_eq!((next, alarm.id), (at("2026-11-03T04:00:00+01:00"), 2));
}
