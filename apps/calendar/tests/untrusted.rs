#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::ErrorKind;

use bande_calendar::caldav::{Client, Credentials, Error};
use bande_calendar::ical::{CalendarObject, read_file};
use jiff::Timestamp;
use jiff::tz::TimeZone;

fn event(uid: &str, lines: &str) -> String {
    format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261001T000000Z\r\nDTSTART:20261102T100000\r\n{lines}\r\nEND:VEVENT\r\n"
    )
}

#[test]
fn a_hostile_fifo_or_huge_calendar_file_opens_without_panicking_hanging_or_being_read_whole() {
    let hours = (0..24).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    let seconds = (0..60).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    let by = format!("BYHOUR={hours},{hours};BYMINUTE={seconds};BYSECOND={seconds}");
    let events = [
        event("a", "RRULE:FREQ=YEARLY;INTERVAL=20000"),
        event("b", "RRULE:FREQ=DAILY;INTERVAL=4294967295"),
        event("c", "RRULE:FREQ=WEEKLY;BYDAY=€"),
        event("d", &format!("RRULE:FREQ=YEARLY;{by}")),
        event("e", "DURATION:P99999999D"),
        event("f", "DURATION:PT9223372036854775807H"),
    ];
    let folder = tempfile::tempdir().unwrap();
    let hostile = folder.path().join("hostile.ics");
    let calendar = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}END:VCALENDAR\r\n",
        events.concat()
    );
    std::fs::write(&hostile, calendar).unwrap();
    let objects = CalendarObject::parse(&read_file(&hostile).unwrap()).unwrap();
    let from = Timestamp::from_second(1_790_000_000).unwrap();
    let to = Timestamp::from_second(1_830_000_000).unwrap();
    for object in &objects {
        object.occurrences(from, to, &TimeZone::UTC);
    }
    let pipe = folder.path().join("pipe.ics");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        read_file(&pipe).map_err(|error| error.kind()),
        Err(ErrorKind::InvalidInput)
    );
    let huge = folder.path().join("huge.ics");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(65 << 20)
        .unwrap();
    assert_eq!(
        read_file(&huge).map_err(|error| error.kind()),
        Err(ErrorKind::FileTooLarge)
    );
}

#[test]
fn a_password_is_sent_only_over_https_or_to_this_computer() {
    let client = Client::new(Some(&Credentials {
        user: "jane".to_owned(),
        password: "secret".to_owned(),
    }));
    for server in ["http://dav.example.com/", "http://localhost.evil.example/"] {
        let refused = client.discover(server, "jane");
        assert!(matches!(refused, Err(Error::Insecure(_))));
    }
}
