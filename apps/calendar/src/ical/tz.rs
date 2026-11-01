use std::collections::HashMap;

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, ToSpan};

use super::lines::{Component, Property};

const WINDOWS_ZONES: [(&str, &str); 24] = [
    ("UTC", "UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("Romance Standard Time", "Europe/Paris"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("China Standard Time", "Asia/Shanghai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
];

#[derive(Debug, Clone, Default)]
pub struct Zones {
    defined: HashMap<String, Component>,
}

impl Zones {
    pub fn from_calendar(calendar: &Component) -> Self {
        Self {
            defined: calendar
                .children("VTIMEZONE")
                .filter_map(|zone| Some((zone.value("TZID")?.trim().to_owned(), zone.clone())))
                .collect(),
        }
    }

    pub fn resolve(&self, tzid: &str) -> Option<TimeZone> {
        let tzid = tzid.trim();
        system_zone(tzid).or_else(|| self.defined.get(tzid).and_then(from_vtimezone))
    }
}

fn system_zone(tzid: &str) -> Option<TimeZone> {
    if let Ok(zone) = TimeZone::get(tzid) {
        return Some(zone);
    }
    if let Some((_, iana)) = WINDOWS_ZONES
        .iter()
        .find(|(windows, _)| windows.eq_ignore_ascii_case(tzid))
    {
        return TimeZone::get(iana).ok();
    }
    let segments: Vec<&str> = tzid.split('/').filter(|part| !part.is_empty()).collect();
    (1..segments.len()).find_map(|skip| TimeZone::get(&segments[skip..].join("/")).ok())
}

fn parse_offset(text: &str) -> Option<i32> {
    let text = text.trim();
    let (sign, digits) = match text.as_bytes().first()? {
        b'-' => (-1, &text[1..]),
        b'+' => (1, &text[1..]),
        _ => (1, text),
    };
    let number = |range: std::ops::Range<usize>| digits.get(range)?.parse::<i32>().ok();
    let seconds = number(0..2)? * 3600 + number(2..4)? * 60 + number(4..6).unwrap_or(0);
    Some(sign * seconds)
}

fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.abs();
    match seconds % 60 {
        0 => format!("{sign}{:02}{:02}", seconds / 3600, seconds % 3600 / 60),
        rest => format!(
            "{sign}{:02}{:02}{rest:02}",
            seconds / 3600,
            seconds % 3600 / 60
        ),
    }
}

fn posix_offset(seconds: i32) -> String {
    let inverted = -seconds;
    let sign = if inverted < 0 { "-" } else { "" };
    let inverted = inverted.abs();
    format!(
        "{sign}{:02}:{:02}:{:02}",
        inverted / 3600,
        inverted % 3600 / 60,
        inverted % 60
    )
}

struct Observance {
    start: String,
    offset_to: i32,
    rule: Option<String>,
}

fn observance(component: &Component) -> Option<Observance> {
    Some(Observance {
        start: component.value("DTSTART")?.trim().to_owned(),
        offset_to: parse_offset(component.value("TZOFFSETTO")?)?,
        rule: component.value("RRULE").map(str::to_owned),
    })
}

fn latest<'a>(component: &'a Component, kind: &'a str) -> Option<Observance> {
    component
        .children(kind)
        .filter_map(observance)
        .max_by(|a, b| {
            (a.rule.is_some(), a.start.as_str()).cmp(&(b.rule.is_some(), b.start.as_str()))
        })
}

fn posix_rule(observance: &Observance) -> Option<String> {
    let rule = observance.rule.as_deref()?;
    let part = |name: &str| {
        rule.split(';')
            .find_map(|pair| {
                pair.split_once('=')
                    .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            })
            .map(|(_, value)| value.to_ascii_uppercase())
    };
    if part("FREQ")? != "YEARLY" {
        return None;
    }
    let month: u8 = part("BYMONTH")?.parse().ok()?;
    let by_day = part("BYDAY")?;
    let split = by_day.len().checked_sub(2)?;
    let (ordinal, day) = by_day.split_at(split);
    let ordinal: i32 = match ordinal {
        "" => 1,
        text => text.trim_start_matches('+').parse().ok()?,
    };
    let week = match ordinal {
        -1 => 5,
        1..=4 => ordinal,
        _ => return None,
    };
    let weekday = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"]
        .iter()
        .position(|name| *name == day)?;
    let time = observance
        .start
        .split_once('T')
        .map_or("020000", |(_, time)| time);
    let number = |range: std::ops::Range<usize>| time.get(range)?.parse::<u32>().ok();
    Some(format!(
        "M{month}.{week}.{weekday}/{:02}:{:02}:{:02}",
        number(0..2)?,
        number(2..4)?,
        number(4..6).unwrap_or(0)
    ))
}

pub fn from_vtimezone(component: &Component) -> Option<TimeZone> {
    let standard = latest(component, "STANDARD");
    let daylight = latest(component, "DAYLIGHT");
    match (standard, daylight) {
        (Some(standard), Some(daylight)) if standard.rule.is_some() && daylight.rule.is_some() => {
            let posix = format!(
                "<STD>{}<DST>{},{},{}",
                posix_offset(standard.offset_to),
                posix_offset(daylight.offset_to),
                posix_rule(&daylight)?,
                posix_rule(&standard)?
            );
            TimeZone::posix(&posix).ok()
        }
        (Some(only), _) | (None, Some(only)) => Offset::from_seconds(only.offset_to)
            .ok()
            .map(TimeZone::fixed),
        _ => None,
    }
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "MO",
        Weekday::Tuesday => "TU",
        Weekday::Wednesday => "WE",
        Weekday::Thursday => "TH",
        Weekday::Friday => "FR",
        Weekday::Saturday => "SA",
        Weekday::Sunday => "SU",
    }
}

fn transitions_in(zone: &TimeZone, year: i16) -> Vec<(Timestamp, Offset, Offset)> {
    let Ok(start) = Date::new(year, 1, 1).and_then(|day| day.to_zoned(TimeZone::UTC)) else {
        return Vec::new();
    };
    let Ok(end) = start.timestamp().checked_add((366 * 24).hours()) else {
        return Vec::new();
    };
    let mut before = zone.to_offset(start.timestamp());
    let mut found = Vec::new();
    for transition in zone.following(start.timestamp()) {
        if transition.timestamp() >= end || found.len() >= 2 {
            break;
        }
        found.push((transition.timestamp(), before, transition.offset()));
        before = transition.offset();
    }
    found
}

fn observance_component(at: Timestamp, from: Offset, to: Offset) -> Component {
    let wall: DateTime = at.to_zoned(TimeZone::fixed(from)).datetime();
    let date = wall.date();
    let ordinal = match date.day() + 7 > date.days_in_month() {
        true => -1,
        false => (date.day() - 1) / 7 + 1,
    };
    let kind = match to.seconds() > from.seconds() {
        true => "DAYLIGHT",
        false => "STANDARD",
    };
    let mut component = Component::new(kind);
    component.push(Property::new("DTSTART", super::value::format_wall(wall)));
    component.push(Property::new("TZOFFSETFROM", format_offset(from.seconds())));
    component.push(Property::new("TZOFFSETTO", format_offset(to.seconds())));
    component.push(Property::new(
        "RRULE",
        format!(
            "FREQ=YEARLY;BYMONTH={};BYDAY={ordinal}{}",
            date.month(),
            weekday_name(date.weekday())
        ),
    ));
    component
}

pub fn vtimezone(zone: &TimeZone, tzid: &str, year: i16) -> Component {
    let mut component = Component::new("VTIMEZONE");
    component.push(Property::new("TZID", tzid));
    let transitions = transitions_in(zone, year);
    if transitions.len() == 2 {
        for (at, from, to) in transitions {
            component
                .components
                .push(observance_component(at, from, to));
        }
    } else {
        let offset = Date::new(year, 1, 1)
            .and_then(|day| day.to_zoned(TimeZone::UTC))
            .map_or(Offset::UTC, |at| zone.to_offset(at.timestamp()));
        let mut standard = Component::new("STANDARD");
        standard.push(Property::new("DTSTART", "19700101T000000"));
        standard.push(Property::new(
            "TZOFFSETFROM",
            format_offset(offset.seconds()),
        ));
        standard.push(Property::new("TZOFFSETTO", format_offset(offset.seconds())));
        component.components.push(standard);
    }
    component
}
