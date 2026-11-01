use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, ToSpan};

use super::tz::Zones;
use super::value::Moment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Secondly,
    Minutely,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Frequency {
    fn name(self) -> &'static str {
        match self {
            Self::Secondly => "SECONDLY",
            Self::Minutely => "MINUTELY",
            Self::Hourly => "HOURLY",
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Self::Secondly,
            Self::Minutely,
            Self::Hourly,
            Self::Daily,
            Self::Weekly,
            Self::Monthly,
            Self::Yearly,
        ]
        .into_iter()
        .find(|frequency| frequency.name().eq_ignore_ascii_case(text))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySpec {
    pub ordinal: Option<i16>,
    pub weekday: Weekday,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub frequency: Frequency,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<Moment>,
    pub by_second: Vec<i8>,
    pub by_minute: Vec<i8>,
    pub by_hour: Vec<i8>,
    pub by_day: Vec<DaySpec>,
    pub by_month_day: Vec<i8>,
    pub by_year_day: Vec<i16>,
    pub by_week_no: Vec<i8>,
    pub by_month: Vec<i8>,
    pub by_set_pos: Vec<i32>,
    pub week_start: Weekday,
}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("MO", Weekday::Monday),
    ("TU", Weekday::Tuesday),
    ("WE", Weekday::Wednesday),
    ("TH", Weekday::Thursday),
    ("FR", Weekday::Friday),
    ("SA", Weekday::Saturday),
    ("SU", Weekday::Sunday),
];

const MAX_PERIODS: usize = 50_000;
const MAX_CANDIDATES: usize = 10_000;

pub fn weekday_code(weekday: Weekday) -> &'static str {
    WEEKDAYS
        .iter()
        .find(|(_, day)| *day == weekday)
        .map_or("MO", |(code, _)| code)
}

fn parse_weekday(code: &str) -> Option<Weekday> {
    WEEKDAYS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(code))
        .map(|(_, weekday)| *weekday)
}

fn unique<T: PartialEq>(values: Vec<T>) -> Vec<T> {
    let mut kept = Vec::new();
    for value in values {
        if !kept.contains(&value) {
            kept.push(value);
        }
    }
    kept
}

fn numbers<T: std::str::FromStr + PartialEq>(value: &str) -> Option<Vec<T>> {
    value
        .split(',')
        .map(|part| part.trim().trim_start_matches('+').parse().ok())
        .collect::<Option<_>>()
        .map(unique)
}

fn join<T: ToString>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

impl Rule {
    pub fn new(frequency: Frequency) -> Self {
        Self {
            frequency,
            interval: 1,
            count: None,
            until: None,
            by_second: Vec::new(),
            by_minute: Vec::new(),
            by_hour: Vec::new(),
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_year_day: Vec::new(),
            by_week_no: Vec::new(),
            by_month: Vec::new(),
            by_set_pos: Vec::new(),
            week_start: Weekday::Monday,
        }
    }

    pub fn parse(text: &str, zones: &Zones) -> Option<Self> {
        let mut rule = Self::new(Frequency::Daily);
        let mut frequency = None;
        for pair in text.trim().split(';').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=')?;
            match key.trim().to_ascii_uppercase().as_str() {
                "FREQ" => frequency = Some(Frequency::parse(value)?),
                "INTERVAL" => rule.interval = value.parse::<u32>().ok()?.max(1),
                "COUNT" => rule.count = Some(value.parse().ok()?),
                "UNTIL" => rule.until = Some(Moment::parse_text(value, None, None, zones)?),
                "BYSECOND" => rule.by_second = numbers(value)?,
                "BYMINUTE" => rule.by_minute = numbers(value)?,
                "BYHOUR" => rule.by_hour = numbers(value)?,
                "BYMONTHDAY" => rule.by_month_day = numbers(value)?,
                "BYYEARDAY" => rule.by_year_day = numbers(value)?,
                "BYWEEKNO" => rule.by_week_no = numbers(value)?,
                "BYMONTH" => rule.by_month = numbers(value)?,
                "BYSETPOS" => rule.by_set_pos = numbers(value)?,
                "WKST" => rule.week_start = parse_weekday(value)?,
                "BYDAY" => {
                    rule.by_day = unique(
                        value
                            .split(',')
                            .map(|day| {
                                let day = day.trim();
                                let split = day.len().checked_sub(2)?;
                                let (ordinal, code) = (day.get(..split)?, day.get(split..)?);
                                Some(DaySpec {
                                    ordinal: match ordinal {
                                        "" => None,
                                        text => Some(text.trim_start_matches('+').parse().ok()?),
                                    },
                                    weekday: parse_weekday(code)?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?,
                    );
                }
                _ => {}
            }
        }
        rule.frequency = frequency?;
        Some(rule)
    }

    pub fn to_text(&self) -> String {
        let mut parts = vec![format!("FREQ={}", self.frequency.name())];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if let Some(count) = self.count {
            parts.push(format!("COUNT={count}"));
        }
        if let Some(until) = &self.until {
            parts.push(format!("UNTIL={}", until.value_text()));
        }
        let lists: [(&str, String); 9] = [
            ("BYMONTH", join(&self.by_month)),
            ("BYWEEKNO", join(&self.by_week_no)),
            ("BYYEARDAY", join(&self.by_year_day)),
            ("BYMONTHDAY", join(&self.by_month_day)),
            (
                "BYDAY",
                self.by_day
                    .iter()
                    .map(|day| {
                        format!(
                            "{}{}",
                            day.ordinal
                                .map(|ordinal| ordinal.to_string())
                                .unwrap_or_default(),
                            weekday_code(day.weekday)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            ("BYHOUR", join(&self.by_hour)),
            ("BYMINUTE", join(&self.by_minute)),
            ("BYSECOND", join(&self.by_second)),
            ("BYSETPOS", join(&self.by_set_pos)),
        ];
        for (name, list) in lists {
            if !list.is_empty() {
                parts.push(format!("{name}={list}"));
            }
        }
        if self.week_start != Weekday::Monday {
            parts.push(format!("WKST={}", weekday_code(self.week_start)));
        }
        parts.join(";")
    }

    pub fn expand(
        &self,
        start: &Moment,
        local: &TimeZone,
        horizon: DateTime,
        mut keep_going: impl FnMut(&Moment) -> bool,
    ) {
        let zone = start.zone(local);
        let first = start.wall();
        let mut emitted = 0u32;
        if !self.emit(start, &zone, &mut emitted, &mut keep_going) {
            return;
        }
        for period in 0..MAX_PERIODS {
            let Some(anchor) = self
                .period_anchor(first, period)
                .filter(|anchor| *anchor <= horizon)
            else {
                return;
            };
            let mut candidates = self.candidates(first, anchor);
            candidates.sort();
            candidates.dedup();
            let chosen = self.select_positions(candidates);
            for wall in chosen {
                if wall <= first {
                    continue;
                }
                let moment = start.with_wall(wall);
                if !self.emit(&moment, &zone, &mut emitted, &mut keep_going) {
                    return;
                }
            }
        }
    }

    fn emit(
        &self,
        moment: &Moment,
        zone: &TimeZone,
        emitted: &mut u32,
        keep_going: &mut impl FnMut(&Moment) -> bool,
    ) -> bool {
        if self.count.is_some_and(|count| *emitted >= count) {
            return false;
        }
        if let Some(until) = &self.until
            && self.after_until(moment, until, zone)
        {
            return false;
        }
        *emitted += 1;
        keep_going(moment)
    }

    fn after_until(&self, moment: &Moment, until: &Moment, zone: &TimeZone) -> bool {
        match (moment, until) {
            (_, Moment::Date(until)) => moment.date() > *until,
            (Moment::Date(day), until) => *day > until.to_zoned(zone).date(),
            (Moment::Floating(at), Moment::Floating(until)) => at > until,
            (moment, until) => moment.timestamp(zone) > until.timestamp(zone),
        }
    }

    fn period_anchor(&self, first: DateTime, period: usize) -> Option<DateTime> {
        let steps = i64::try_from(period)
            .ok()?
            .checked_mul(i64::from(self.interval))?;
        let span = match self.frequency {
            Frequency::Yearly => {
                return first
                    .date()
                    .first_of_month()
                    .with()
                    .month(1)
                    .build()
                    .ok()?
                    .checked_add(Span::new().try_years(steps).ok()?)
                    .ok()
                    .map(|date| date.to_datetime(Time::midnight()));
            }
            Frequency::Monthly => {
                return first
                    .date()
                    .first_of_month()
                    .checked_add(Span::new().try_months(steps).ok()?)
                    .ok()
                    .map(|date| date.to_datetime(Time::midnight()));
            }
            Frequency::Weekly => {
                let back = i64::from(days_since(first.date().weekday(), self.week_start));
                return first
                    .date()
                    .checked_sub(Span::new().try_days(back).ok()?)
                    .ok()?
                    .checked_add(Span::new().try_weeks(steps).ok()?)
                    .ok()
                    .map(|date| date.to_datetime(Time::midnight()));
            }
            Frequency::Daily => Span::new().try_days(steps),
            Frequency::Hourly => Span::new().try_hours(steps),
            Frequency::Minutely => Span::new().try_minutes(steps),
            Frequency::Secondly => Span::new().try_seconds(steps),
        }
        .ok()?;
        let base = match self.frequency {
            Frequency::Daily => first.date().to_datetime(Time::midnight()),
            _ => first,
        };
        base.checked_add(span).ok()
    }

    fn period_days(&self, first: DateTime, anchor: DateTime) -> Vec<Date> {
        let day = anchor.date();
        match self.frequency {
            Frequency::Yearly => self.yearly_days(first.date(), day.year()),
            Frequency::Monthly => {
                if !self.month_allowed(day) {
                    return Vec::new();
                }
                let month = month_days(day);
                match self.by_month_day.is_empty() && self.by_day.is_empty() {
                    true => month
                        .into_iter()
                        .filter(|candidate| candidate.day() == first.day())
                        .collect(),
                    false => month
                        .into_iter()
                        .filter(|candidate| self.month_day_allowed(*candidate))
                        .filter(|candidate| {
                            self.weekday_allowed_in(*candidate, &month_days(*candidate))
                        })
                        .collect(),
                }
            }
            Frequency::Weekly => {
                let week: Vec<Date> = (0..7)
                    .filter_map(|offset| day.checked_add(offset.days()).ok())
                    .collect();
                week.into_iter()
                    .filter(|candidate| self.month_allowed(*candidate))
                    .filter(|candidate| match self.by_day.is_empty() {
                        true => candidate.weekday() == first.weekday(),
                        false => self
                            .by_day
                            .iter()
                            .any(|spec| spec.weekday == candidate.weekday()),
                    })
                    .collect()
            }
            Frequency::Daily | Frequency::Hourly | Frequency::Minutely | Frequency::Secondly => {
                let allowed = self.month_allowed(day)
                    && self.month_day_allowed(day)
                    && self.year_day_allowed(day)
                    && (self.by_day.is_empty()
                        || self.by_day.iter().any(|spec| spec.weekday == day.weekday()));
                match allowed {
                    true => vec![day],
                    false => Vec::new(),
                }
            }
        }
    }

    fn yearly_days(&self, first: Date, year: i16) -> Vec<Date> {
        let Ok(january) = Date::new(year, 1, 1) else {
            return Vec::new();
        };
        let nothing_but_months = self.by_week_no.is_empty()
            && self.by_year_day.is_empty()
            && self.by_month_day.is_empty()
            && self.by_day.is_empty();
        if nothing_but_months {
            let months = match self.by_month.is_empty() {
                true => vec![first.month()],
                false => self.by_month.clone(),
            };
            return months
                .into_iter()
                .filter_map(|month| Date::new(year, month, first.day()).ok())
                .collect();
        }
        let year_days: Vec<Date> = (0..i64::from(january.days_in_year()))
            .filter_map(|offset| january.checked_add(offset.days()).ok())
            .collect();
        year_days
            .iter()
            .copied()
            .filter(|day| self.month_allowed(*day))
            .filter(|day| self.week_allowed(*day))
            .filter(|day| self.year_day_allowed(*day))
            .filter(|day| self.month_day_allowed(*day))
            .filter(|day| {
                if self.by_day.is_empty() {
                    return self.by_week_no.is_empty() || day.weekday() == first.weekday();
                }
                match (self.by_month.is_empty(), self.by_week_no.is_empty()) {
                    (false, _) => self.weekday_allowed_in(*day, &month_days(*day)),
                    (true, false) => self.by_day.iter().any(|spec| spec.weekday == day.weekday()),
                    (true, true) => self.weekday_allowed_in(*day, &year_days),
                }
            })
            .collect()
    }

    fn month_allowed(&self, day: Date) -> bool {
        self.by_month.is_empty() || self.by_month.contains(&day.month())
    }

    fn month_day_allowed(&self, day: Date) -> bool {
        self.by_month_day.is_empty()
            || self.by_month_day.iter().any(|wanted| {
                let wanted = match *wanted < 0 {
                    true => day.days_in_month() + 1 + wanted,
                    false => *wanted,
                };
                wanted == day.day()
            })
    }

    fn year_day_allowed(&self, day: Date) -> bool {
        self.by_year_day.is_empty()
            || self.by_year_day.iter().any(|wanted| {
                let wanted = match *wanted < 0 {
                    true => day.days_in_year() + 1 + wanted,
                    false => *wanted,
                };
                wanted == day.day_of_year()
            })
    }

    fn week_allowed(&self, day: Date) -> bool {
        if self.by_week_no.is_empty() {
            return true;
        }
        let (week, weeks_in_year) = week_number(day, self.week_start);
        self.by_week_no.iter().any(|wanted| {
            let wanted = match *wanted < 0 {
                true => weeks_in_year + 1 + wanted,
                false => *wanted,
            };
            wanted == week
        })
    }

    fn weekday_allowed_in(&self, day: Date, scope: &[Date]) -> bool {
        if self.by_day.is_empty() {
            return true;
        }
        self.by_day.iter().any(|spec| {
            if spec.weekday != day.weekday() {
                return false;
            }
            let Some(ordinal) = spec.ordinal else {
                return true;
            };
            let same: Vec<&Date> = scope
                .iter()
                .filter(|other| other.weekday() == day.weekday())
                .collect();
            let position = same.iter().position(|other| **other == day);
            match (position, ordinal) {
                (Some(index), ordinal) if ordinal > 0 => index + 1 == ordinal as usize,
                (Some(index), ordinal) if ordinal < 0 => {
                    same.len() - index == ordinal.unsigned_abs() as usize
                }
                _ => false,
            }
        })
    }

    fn times(&self, first: DateTime) -> Vec<Time> {
        let hours = match self.by_hour.is_empty() {
            true => vec![first.hour()],
            false => self.by_hour.clone(),
        };
        let minutes = match self.by_minute.is_empty() {
            true => vec![first.minute()],
            false => self.by_minute.clone(),
        };
        let seconds = match self.by_second.is_empty() {
            true => vec![first.second()],
            false => self.by_second.clone(),
        };
        let mut times = Vec::new();
        for hour in &hours {
            for minute in &minutes {
                for second in &seconds {
                    if let Ok(time) = Time::new(*hour, *minute, (*second).min(59), 0) {
                        times.push(time);
                    }
                }
            }
        }
        times
    }

    fn candidates(&self, first: DateTime, anchor: DateTime) -> Vec<DateTime> {
        match self.frequency {
            Frequency::Hourly | Frequency::Minutely | Frequency::Secondly => {
                let fits = !self.period_days(first, anchor).is_empty()
                    && (self.by_hour.is_empty() || self.by_hour.contains(&anchor.hour()))
                    && (self.by_minute.is_empty() || self.by_minute.contains(&anchor.minute()))
                    && (self.by_second.is_empty() || self.by_second.contains(&anchor.second()));
                match fits {
                    true => vec![anchor],
                    false => Vec::new(),
                }
            }
            _ => {
                let times = self.times(first);
                self.period_days(first, anchor)
                    .into_iter()
                    .flat_map(|day| times.iter().map(move |time| day.to_datetime(*time)))
                    .take(MAX_CANDIDATES)
                    .collect()
            }
        }
    }

    fn select_positions(&self, candidates: Vec<DateTime>) -> Vec<DateTime> {
        if self.by_set_pos.is_empty() {
            return candidates;
        }
        let length = candidates.len() as i64;
        let mut chosen: Vec<DateTime> = self
            .by_set_pos
            .iter()
            .filter_map(|position| {
                let index = match *position < 0 {
                    true => length + i64::from(*position),
                    false => i64::from(*position) - 1,
                };
                usize::try_from(index)
                    .ok()
                    .and_then(|index| candidates.get(index).copied())
            })
            .collect();
        chosen.sort();
        chosen.dedup();
        chosen
    }
}

fn days_since(day: Weekday, start: Weekday) -> i8 {
    (day.to_monday_zero_offset() - start.to_monday_zero_offset()).rem_euclid(7)
}

fn month_days(day: Date) -> Vec<Date> {
    let first = day.first_of_month();
    (0..i64::from(first.days_in_month()))
        .filter_map(|offset| first.checked_add(offset.days()).ok())
        .collect()
}

fn week_one_start(year: i16, week_start: Weekday) -> Option<Date> {
    let january_4 = Date::new(year, 1, 4).ok()?;
    january_4
        .checked_sub(i64::from(days_since(january_4.weekday(), week_start)).days())
        .ok()
}

fn week_number(day: Date, week_start: Weekday) -> (i8, i8) {
    let year = day.year();
    let candidates = [year + 1, year, year - 1];
    for candidate in candidates {
        let (Some(start), Some(next)) = (
            week_one_start(candidate, week_start),
            week_one_start(candidate + 1, week_start),
        ) else {
            continue;
        };
        if day >= start {
            let week = (day - start).get_days() / 7 + 1;
            let weeks = (next - start).get_days() / 7;
            return (week as i8, weeks as i8);
        }
    }
    (1, 52)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn expand(start: &str, tzid: Option<&str>, rule: &str, limit: usize) -> Vec<String> {
        let zones = Zones::default();
        let start = Moment::parse_text(start, tzid, None, &zones).unwrap();
        let rule = Rule::parse(rule, &zones).unwrap();
        let mut out = Vec::new();
        rule.expand(
            &start,
            &TimeZone::UTC,
            date(2100, 1, 1).at(0, 0, 0, 0),
            |moment| {
                out.push(moment.value_text());
                out.len() < limit
            },
        );
        out
    }

    #[test]
    fn text_round_trips() {
        let zones = Zones::default();
        for text in [
            "FREQ=WEEKLY;INTERVAL=2;COUNT=10;BYDAY=MO,WE,FR",
            "FREQ=MONTHLY;UNTIL=20271231T235959Z;BYDAY=-1FR",
            "FREQ=YEARLY;BYMONTH=3;BYDAY=2SU;WKST=SU",
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
        ] {
            assert_eq!(Rule::parse(text, &zones).unwrap().to_text(), text);
        }
    }

    #[test]
    fn hostile_rules_end_without_panicking_or_blowing_up() {
        assert_eq!(
            expand("20261102T100000", None, "FREQ=YEARLY;INTERVAL=20000", 10),
            ["20261102T100000"]
        );
        assert_eq!(
            expand(
                "20261102T100000",
                None,
                "FREQ=DAILY;INTERVAL=4294967295",
                10
            ),
            ["20261102T100000"]
        );
        assert_eq!(Rule::parse("FREQ=WEEKLY;BYDAY=€", &Zones::default()), None);
        let every_second = (0..60).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        let every_hour = (0..24).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        let rule = format!(
            "FREQ=YEARLY;BYHOUR={every_hour},{every_hour};BYMINUTE={every_second};BYSECOND={every_second}"
        );
        assert_eq!(
            Rule::parse(&rule, &Zones::default()).unwrap().by_hour.len(),
            24
        );
        assert!(!expand("20261102T100000", None, &rule, 3).is_empty());
    }

    #[test]
    fn weekly_with_count() {
        assert_eq!(
            expand(
                "20261102T100000",
                None,
                "FREQ=WEEKLY;COUNT=4;BYDAY=MO,TH",
                100
            ),
            [
                "20261102T100000",
                "20261105T100000",
                "20261109T100000",
                "20261112T100000"
            ]
        );
    }

    #[test]
    fn last_weekday_of_the_month_with_setpos() {
        assert_eq!(
            expand(
                "20260130T090000",
                None,
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=4",
                100
            ),
            [
                "20260130T090000",
                "20260227T090000",
                "20260331T090000",
                "20260430T090000"
            ]
        );
    }

    #[test]
    fn the_31st_skips_short_months() {
        assert_eq!(
            expand("20260131T080000", None, "FREQ=MONTHLY;COUNT=4", 100),
            [
                "20260131T080000",
                "20260331T080000",
                "20260531T080000",
                "20260731T080000"
            ]
        );
    }

    #[test]
    fn leap_day_birthdays() {
        assert_eq!(
            expand("20240229", None, "FREQ=YEARLY;COUNT=3", 100),
            ["20240229", "20280229", "20320229"]
        );
    }

    #[test]
    fn us_election_day() {
        assert_eq!(
            expand(
                "19961105T090000",
                None,
                "FREQ=YEARLY;INTERVAL=4;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8",
                3
            ),
            ["19961105T090000", "20001107T090000", "20041102T090000"]
        );
    }

    #[test]
    fn every_20th_monday_and_week_numbers() {
        assert_eq!(
            expand("19970519T090000", None, "FREQ=YEARLY;BYDAY=20MO", 3),
            ["19970519T090000", "19980518T090000", "19990517T090000"]
        );
        assert_eq!(
            expand(
                "19970512T090000",
                None,
                "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO",
                3
            ),
            ["19970512T090000", "19980511T090000", "19990517T090000"]
        );
    }

    #[test]
    fn until_is_inclusive_and_in_utc() {
        assert_eq!(
            expand(
                "20261026T090000",
                Some("Europe/Berlin"),
                "FREQ=DAILY;UNTIL=20261028T080000Z",
                100
            ),
            ["20261026T090000", "20261027T090000", "20261028T090000"]
        );
    }

    #[test]
    fn weekly_keeps_local_time_through_dst() {
        let zones = Zones::default();
        let start =
            Moment::parse_text("20261018T093000", Some("Europe/Berlin"), None, &zones).unwrap();
        let rule = Rule::parse("FREQ=WEEKLY;COUNT=3", &zones).unwrap();
        let mut utc = Vec::new();
        rule.expand(
            &start,
            &TimeZone::UTC,
            date(2100, 1, 1).at(0, 0, 0, 0),
            |moment| {
                utc.push(moment.timestamp(&TimeZone::UTC).to_string());
                true
            },
        );
        assert_eq!(
            utc,
            [
                "2026-10-18T07:30:00Z",
                "2026-10-25T08:30:00Z",
                "2026-11-01T08:30:00Z"
            ]
        );
    }

    #[test]
    fn a_time_in_the_spring_gap_moves_forward() {
        let zones = Zones::default();
        let start =
            Moment::parse_text("20260322T023000", Some("Europe/Berlin"), None, &zones).unwrap();
        let rule = Rule::parse("FREQ=WEEKLY;COUNT=2", &zones).unwrap();
        let mut walls = Vec::new();
        rule.expand(
            &start,
            &TimeZone::UTC,
            date(2100, 1, 1).at(0, 0, 0, 0),
            |moment| {
                walls.push(moment.wall());
                true
            },
        );
        assert_eq!(
            walls,
            [
                date(2026, 3, 22).at(2, 30, 0, 0),
                date(2026, 3, 29).at(3, 30, 0, 0)
            ]
        );
    }

    #[test]
    fn an_impossible_rule_ends() {
        assert_eq!(
            expand(
                "20260101T090000",
                None,
                "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30",
                10
            ),
            ["20260101T090000"]
        );
    }
}
