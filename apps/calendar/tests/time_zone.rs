use bande_calendar::edit::{Alert, Details, EndRepeat, Repeat, Scope, apply_details};
use bande_calendar::ical::CalendarObject;
use jiff::Timestamp;
use jiff::civil::date;
use jiff::tz::TimeZone;

const SERIES: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\n\
DTSTART;TZID=Europe/Stockholm:20261103T100000\r\nDTEND;TZID=Europe/Stockholm:20261103T110000\r\n\
RRULE:FREQ=WEEKLY;UNTIL=20261201T225959Z\r\nEXDATE;TZID=Europe/Stockholm:20261110T100000\r\n\
SUMMARY:Sync\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn a_series_moved_to_another_zone_keeps_its_times_exceptions_and_end() {
    let local = TimeZone::get("Europe/Stockholm").unwrap();
    let new_york = TimeZone::get("America/New_York").unwrap();
    let object = CalendarObject::parse(SERIES).unwrap().remove(0);
    let (from, to): (Timestamp, Timestamp) = (
        "2026-11-01T00:00Z".parse().unwrap(),
        "2027-01-01T00:00Z".parse().unwrap(),
    );
    let first = object.occurrences(from, to, &local)[0].key;
    let details = Details {
        summary: "Sync".into(),
        location: String::new(),
        notes: String::new(),
        all_day: false,
        start: date(2026, 11, 3).at(10, 0, 0, 0),
        end: date(2026, 11, 3).at(11, 0, 0, 0),
        repeat: Repeat::Weekly,
        custom: None,
        end_repeat: EndRepeat::On(date(2026, 12, 1)),
        alert: Alert::None,
        invitees: Vec::new(),
        time_zone: Some("America/New_York".into()),
    };
    let moved = apply_details(&object, 0, first, &details, Scope::Series, &local);
    let starts: Vec<_> = moved
        .occurrences(from, to, &local)
        .iter()
        .map(|found| found.start.with_time_zone(new_york.clone()).datetime())
        .collect();
    let expected = [(11, 3), (11, 17), (11, 24), (12, 1)]
        .map(|(month, day)| date(2026, month, day).at(10, 0, 0, 0));
    assert_eq!(starts, expected);
}
