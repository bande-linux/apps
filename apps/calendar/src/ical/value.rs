use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

use super::lines::Property;
use super::tz::Zones;

#[derive(Debug, Clone, PartialEq)]
pub enum Moment {
    Date(Date),
    Floating(DateTime),
    Zoned { at: Zoned, tzid: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstanceKey {
    Date(Date),
    Floating(DateTime),
    Instant(Timestamp),
}

impl Moment {
    pub fn utc(at: Timestamp) -> Self {
        Self::Zoned {
            at: at.to_zoned(TimeZone::UTC),
            tzid: None,
        }
    }

    pub fn in_zone(at: Zoned) -> Self {
        let tzid = at.time_zone().iana_name().map(str::to_owned);
        match tzid.as_deref() {
            Some("UTC") | None => Self::utc(at.timestamp()),
            Some(_) => Self::Zoned { at, tzid },
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(self, Self::Date(_))
    }

    pub fn key(&self) -> InstanceKey {
        match self {
            Self::Date(date) => InstanceKey::Date(*date),
            Self::Floating(at) => InstanceKey::Floating(*at),
            Self::Zoned { at, .. } => InstanceKey::Instant(at.timestamp()),
        }
    }

    pub fn wall(&self) -> DateTime {
        match self {
            Self::Date(date) => date.to_datetime(jiff::civil::Time::midnight()),
            Self::Floating(at) => *at,
            Self::Zoned { at, .. } => at.datetime(),
        }
    }

    pub fn date(&self) -> Date {
        self.wall().date()
    }

    pub fn zone(&self, local: &TimeZone) -> TimeZone {
        match self {
            Self::Zoned { at, .. } => at.time_zone().clone(),
            _ => local.clone(),
        }
    }

    pub fn with_wall(&self, wall: DateTime) -> Self {
        match self {
            Self::Date(_) => Self::Date(wall.date()),
            Self::Floating(_) => Self::Floating(wall),
            Self::Zoned { at, tzid } => Self::Zoned {
                at: at
                    .time_zone()
                    .to_ambiguous_zoned(wall)
                    .compatible()
                    .unwrap_or_else(|_| at.clone()),
                tzid: tzid.clone(),
            },
        }
    }

    pub fn to_zoned(&self, local: &TimeZone) -> Zoned {
        match self {
            Self::Zoned { at, .. } => at.with_time_zone(local.clone()),
            other => local
                .to_ambiguous_zoned(other.wall())
                .compatible()
                .unwrap_or_else(|_| Timestamp::UNIX_EPOCH.to_zoned(local.clone())),
        }
    }

    pub fn timestamp(&self, local: &TimeZone) -> Timestamp {
        self.to_zoned(local).timestamp()
    }

    pub fn add(&self, duration: &Duration) -> Self {
        match self {
            Self::Date(date) => Self::Date(
                Span::new()
                    .try_days(duration.whole_days())
                    .and_then(|span| date.checked_add(span))
                    .unwrap_or(*date),
            ),
            Self::Floating(at) => Self::Floating(at.checked_add(duration.span()).unwrap_or(*at)),
            Self::Zoned { at, tzid } => Self::Zoned {
                at: at
                    .checked_add(duration.span())
                    .unwrap_or_else(|_| at.clone()),
                tzid: tzid.clone(),
            },
        }
    }

    pub fn parse(property: &Property, zones: &Zones) -> Option<Self> {
        Self::parse_text(
            property.value.trim(),
            property.param("TZID"),
            property.param("VALUE"),
            zones,
        )
    }

    pub fn parse_text(
        text: &str,
        tzid: Option<&str>,
        kind: Option<&str>,
        zones: &Zones,
    ) -> Option<Self> {
        let is_date =
            kind.is_some_and(|kind| kind.eq_ignore_ascii_case("DATE")) || !text.contains('T');
        if is_date {
            return parse_date(text).map(Self::Date);
        }
        let (text, utc) = match text.strip_suffix(['Z', 'z']) {
            Some(text) => (text, true),
            None => (text, false),
        };
        let (date, time) = text.split_once(['T', 't'])?;
        let date = parse_date(date)?;
        let digits = |range: std::ops::Range<usize>| time.get(range)?.parse::<i8>().ok();
        let wall = date.to_datetime(
            jiff::civil::Time::new(
                digits(0..2)?,
                digits(2..4)?,
                digits(4..6).unwrap_or(0).min(59),
                0,
            )
            .ok()?,
        );
        if utc {
            return wall
                .to_zoned(TimeZone::UTC)
                .ok()
                .map(|at| Self::Zoned { at, tzid: None });
        }
        match tzid.and_then(|tzid| zones.resolve(tzid).map(|zone| (tzid, zone))) {
            Some((tzid, zone)) => {
                zone.to_ambiguous_zoned(wall)
                    .compatible()
                    .ok()
                    .map(|at| Self::Zoned {
                        at,
                        tzid: Some(tzid.to_owned()),
                    })
            }
            None => Some(Self::Floating(wall)),
        }
    }

    pub fn value_text(&self) -> String {
        match self {
            Self::Date(date) => format_date(*date),
            Self::Floating(at) => format_wall(*at),
            Self::Zoned { at, tzid: None } => {
                format!(
                    "{}Z",
                    format_wall(at.with_time_zone(TimeZone::UTC).datetime())
                )
            }
            Self::Zoned { at, .. } => format_wall(at.datetime()),
        }
    }

    pub fn property(&self, name: &str) -> Property {
        let property = Property::new(name, self.value_text());
        match self {
            Self::Date(_) => property.with_param("VALUE", "DATE"),
            Self::Zoned {
                tzid: Some(tzid), ..
            } => property.with_param("TZID", tzid),
            _ => property,
        }
    }
}

fn parse_date(text: &str) -> Option<Date> {
    let text = text.trim();
    if text.len() != 8 {
        return None;
    }
    Date::new(
        text.get(0..4)?.parse().ok()?,
        text.get(4..6)?.parse().ok()?,
        text.get(6..8)?.parse().ok()?,
    )
    .ok()
}

pub fn format_date(date: Date) -> String {
    format!("{:04}{:02}{:02}", date.year(), date.month(), date.day())
}

pub fn format_wall(at: DateTime) -> String {
    format!(
        "{}T{:02}{:02}{:02}",
        format_date(at.date()),
        at.hour(),
        at.minute(),
        at.second()
    )
}

const MAX_DURATION_DAYS: i64 = 100 * 366;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Duration {
    pub negative: bool,
    pub days: i64,
    pub seconds: i64,
}

impl Duration {
    pub fn minutes(minutes: i64) -> Self {
        Self::from_seconds(minutes * 60)
    }

    pub fn from_seconds(seconds: i64) -> Self {
        Self {
            negative: seconds < 0,
            days: 0,
            seconds: seconds.abs(),
        }
    }

    pub fn from_days(days: i64) -> Self {
        Self {
            negative: days < 0,
            days: days.abs(),
            seconds: 0,
        }
    }

    pub fn whole_days(&self) -> i64 {
        let days = self.days + self.seconds / 86_400;
        match self.negative {
            true => -days,
            false => days,
        }
    }

    pub fn total_seconds(&self) -> i64 {
        let seconds = self.days.saturating_mul(86_400).saturating_add(self.seconds);
        match self.negative {
            true => -seconds,
            false => seconds,
        }
    }

    pub fn span(&self) -> Span {
        let sign = if self.negative { -1 } else { 1 };
        Span::new()
            .try_days(sign * self.days)
            .and_then(|span| span.try_seconds(sign * self.seconds))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (negative, rest) = match text.as_bytes().first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let rest = rest.strip_prefix(['P', 'p'])?;
        let mut duration = Self {
            negative,
            ..Self::default()
        };
        let mut number = String::new();
        let mut in_time = false;
        for character in rest.chars() {
            match character.to_ascii_uppercase() {
                digit @ '0'..='9' => number.push(digit),
                'T' => in_time = true,
                unit => {
                    let value: i64 = number.parse().ok()?;
                    number.clear();
                    let (total, scale) = match (unit, in_time) {
                        ('W', false) => (&mut duration.days, 7),
                        ('D', false) => (&mut duration.days, 1),
                        ('H', true) => (&mut duration.seconds, 3600),
                        ('M', true) => (&mut duration.seconds, 60),
                        ('S', true) => (&mut duration.seconds, 1),
                        _ => return None,
                    };
                    *total = value
                        .checked_mul(scale)
                        .and_then(|value| total.checked_add(value))?;
                }
            }
        }
        (number.is_empty()
            && duration.days <= MAX_DURATION_DAYS
            && duration.seconds <= MAX_DURATION_DAYS * 86_400)
            .then_some(duration)
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if self.negative && (self.days != 0 || self.seconds != 0) {
            text.push('-');
        }
        text.push('P');
        if self.days != 0 {
            match self.days % 7 == 0 && self.seconds == 0 {
                true => text.push_str(&format!("{}W", self.days / 7)),
                false => text.push_str(&format!("{}D", self.days)),
            }
        }
        if self.seconds != 0 || self.days == 0 {
            text.push('T');
            let (hours, minutes, seconds) = (
                self.seconds / 3600,
                self.seconds % 3600 / 60,
                self.seconds % 60,
            );
            if hours != 0 {
                text.push_str(&format!("{hours}H"));
            }
            if minutes != 0 {
                text.push_str(&format!("{minutes}M"));
            }
            if seconds != 0 || (hours == 0 && minutes == 0) {
                text.push_str(&format!("{seconds}S"));
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn durations_round_trip() {
        for text in ["-PT15M", "P1D", "P2W", "PT1H30M", "P1DT12H", "PT0S"] {
            let duration = Duration::parse(text).unwrap();
            assert_eq!(duration.to_text(), text);
        }
        assert_eq!(Duration::parse("-PT15M").unwrap().total_seconds(), -900);
        assert_eq!(Duration::parse("P1W").unwrap().whole_days(), 7);
        assert_eq!(Duration::parse("PT5X"), None);
        assert_eq!(Duration::parse("P99999999D"), None);
        assert_eq!(Duration::parse("PT9223372036854775807H"), None);
        assert_eq!(Duration::parse("P36600D").unwrap().whole_days(), 36_600);
    }

    #[test]
    fn reads_the_three_date_time_forms() {
        let zones = Zones::default();
        let utc = Moment::parse_text("20261031T120000Z", None, None, &zones).unwrap();
        assert_eq!(utc.value_text(), "20261031T120000Z");
        let zoned =
            Moment::parse_text("20261031T120000", Some("Europe/Berlin"), None, &zones).unwrap();
        assert_eq!(
            zoned.timestamp(&TimeZone::UTC).to_string(),
            "2026-10-31T11:00:00Z"
        );
        assert_eq!(
            zoned.property("DTSTART").param("TZID"),
            Some("Europe/Berlin")
        );
        let floating = Moment::parse_text("20261031T120000", None, None, &zones).unwrap();
        assert_eq!(
            floating,
            Moment::Floating(date(2026, 10, 31).at(12, 0, 0, 0))
        );
        let day = Moment::parse_text("20261031", None, Some("DATE"), &zones).unwrap();
        assert_eq!(day, Moment::Date(date(2026, 10, 31)));
    }

    #[test]
    fn a_nominal_day_keeps_the_wall_clock_across_dst() {
        let zones = Zones::default();
        let start =
            Moment::parse_text("20261024T090000", Some("Europe/Berlin"), None, &zones).unwrap();
        let next = start.add(&Duration::from_days(7));
        assert_eq!(next.wall(), date(2026, 10, 31).at(9, 0, 0, 0));
        assert_eq!(
            next.timestamp(&TimeZone::UTC).as_second()
                - start.timestamp(&TimeZone::UTC).as_second(),
            7 * 86_400 + 3600
        );
    }
}
