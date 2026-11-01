use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;

use crate::ical::{
    Alarm, CalendarObject, DaySpec, Duration, Event, Frequency, InstanceKey, Moment, Rule, Trigger,
};
use crate::store::new_uid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Occurrence,
    Series,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Never,
    Daily,
    Weekdays,
    Weekly,
    Biweekly,
    Monthly,
    Yearly,
    Custom,
}

impl Repeat {
    pub const CHOICES: [Self; 7] = [
        Self::Never,
        Self::Daily,
        Self::Weekdays,
        Self::Weekly,
        Self::Biweekly,
        Self::Monthly,
        Self::Yearly,
    ];

    pub fn of(rule: Option<&Rule>) -> Self {
        let Some(rule) = rule else {
            return Self::Never;
        };
        let plain = rule.by_month_day.is_empty()
            && rule.by_month.is_empty()
            && rule.by_set_pos.is_empty()
            && rule.by_year_day.is_empty()
            && rule.by_week_no.is_empty()
            && rule.by_hour.is_empty()
            && rule.by_minute.is_empty()
            && rule.count.is_none()
            && rule.until.is_none();
        let workdays: Vec<DaySpec> = [
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
        ]
        .into_iter()
        .map(|weekday| DaySpec {
            ordinal: None,
            weekday,
        })
        .collect();
        match (plain, rule.frequency, rule.interval, rule.by_day.as_slice()) {
            (false, ..) => Self::Custom,
            (true, Frequency::Daily, 1, []) => Self::Daily,
            (true, Frequency::Weekly, 1, days) if days == workdays.as_slice() => Self::Weekdays,
            (true, Frequency::Weekly, 1, [] | [_]) => Self::Weekly,
            (true, Frequency::Weekly, 2, [] | [_]) => Self::Biweekly,
            (true, Frequency::Monthly, 1, []) => Self::Monthly,
            (true, Frequency::Yearly, 1, []) => Self::Yearly,
            _ => Self::Custom,
        }
    }

    pub fn rule(self, existing: Option<&Rule>) -> Option<Rule> {
        let weekly = |interval| Rule {
            interval,
            ..Rule::new(Frequency::Weekly)
        };
        match self {
            Self::Never => None,
            Self::Custom => existing.cloned(),
            Self::Daily => Some(Rule::new(Frequency::Daily)),
            Self::Weekdays => Some(Rule {
                by_day: [
                    Weekday::Monday,
                    Weekday::Tuesday,
                    Weekday::Wednesday,
                    Weekday::Thursday,
                    Weekday::Friday,
                ]
                .into_iter()
                .map(|weekday| DaySpec {
                    ordinal: None,
                    weekday,
                })
                .collect(),
                ..weekly(1)
            }),
            Self::Weekly => Some(weekly(1)),
            Self::Biweekly => Some(weekly(2)),
            Self::Monthly => Some(Rule::new(Frequency::Monthly)),
            Self::Yearly => Some(Rule::new(Frequency::Yearly)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    None,
    Minutes(i64),
    DayAt { days_before: i64, hour: i64 },
    Custom,
}

impl Alert {
    pub const TIMED: [Self; 9] = [
        Self::None,
        Self::Minutes(0),
        Self::Minutes(5),
        Self::Minutes(10),
        Self::Minutes(15),
        Self::Minutes(30),
        Self::Minutes(60),
        Self::Minutes(120),
        Self::Minutes(1440),
    ];

    pub const ALL_DAY: [Self; 4] = [
        Self::None,
        Self::DayAt {
            days_before: 0,
            hour: 9,
        },
        Self::DayAt {
            days_before: 1,
            hour: 9,
        },
        Self::DayAt {
            days_before: 7,
            hour: 9,
        },
    ];

    pub fn of(alarms: &[Alarm], all_day: bool) -> Self {
        let mut shown = alarms.iter().filter(|alarm| {
            alarm.action.eq_ignore_ascii_case("DISPLAY")
                || alarm.action.eq_ignore_ascii_case("AUDIO")
        });
        let Some(first) = shown.next() else {
            return Self::None;
        };
        let Trigger::Relative {
            offset,
            from_end: false,
        } = first.trigger
        else {
            return Self::Custom;
        };
        let seconds = offset.total_seconds();
        let found = match all_day {
            true => {
                let days_before =
                    (-seconds).div_euclid(86_400) + i64::from((-seconds).rem_euclid(86_400) != 0);
                let hour = (seconds + days_before * 86_400) / 3600;
                Self::DayAt { days_before, hour }
            }
            false => Self::Minutes(-seconds / 60),
        };
        match (shown.next(), seconds % 60 == 0) {
            (None, true) => found,
            _ => Self::Custom,
        }
    }

    pub fn alarms(self, existing: &[Alarm]) -> Vec<Alarm> {
        match self {
            Self::None => Vec::new(),
            Self::Custom => existing.to_vec(),
            Self::Minutes(minutes) => vec![Alarm::before(Duration::minutes(-minutes))],
            Self::DayAt { days_before, hour } => {
                vec![Alarm::before(Duration::from_seconds(
                    -days_before * 86_400 + hour * 3600,
                ))]
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub summary: String,
    pub location: String,
    pub notes: String,
    pub all_day: bool,
    pub start: DateTime,
    pub end: DateTime,
    pub repeat: Repeat,
    pub alert: Alert,
}

pub fn moment_of(key: InstanceKey, like: &Moment) -> Moment {
    match (key, like) {
        (InstanceKey::Date(day), _) => Moment::Date(day),
        (InstanceKey::Floating(at), _) => Moment::Floating(at),
        (InstanceKey::Instant(at), Moment::Zoned { at: zoned, tzid }) => Moment::Zoned {
            at: at.to_zoned(zoned.time_zone().clone()),
            tzid: tzid.clone(),
        },
        (InstanceKey::Instant(at), _) => Moment::utc(at),
    }
}

pub fn local_moment(wall: DateTime, all_day: bool, local: &TimeZone) -> Moment {
    match all_day {
        true => Moment::Date(wall.date()),
        false => local
            .to_ambiguous_zoned(wall)
            .compatible()
            .map(Moment::in_zone)
            .unwrap_or(Moment::Floating(wall)),
    }
}

pub fn new_event(
    start: DateTime,
    end: DateTime,
    all_day: bool,
    summary: &str,
    local: &TimeZone,
) -> CalendarObject {
    let end = match all_day {
        true => end
            .date()
            .max(start.date().tomorrow().unwrap_or(start.date()))
            .to_datetime(jiff::civil::Time::midnight()),
        false => end.max(start),
    };
    CalendarObject::new(Event::new(
        new_uid(),
        summary,
        local_moment(start, all_day, local),
        local_moment(end, all_day, local),
    ))
}

fn follow_weekday(rule: &mut Rule, from: Date, to: Date) {
    if rule.frequency == Frequency::Weekly
        && let [only] = rule.by_day.as_mut_slice()
        && only.weekday == from.weekday()
    {
        only.weekday = to.weekday();
    }
}

fn place(moment: &Moment, wall: DateTime, all_day: bool, local: &TimeZone) -> Moment {
    match (moment, all_day) {
        (Moment::Date(_), true) | (Moment::Floating(_), false) => moment.with_wall(wall),
        (Moment::Zoned { .. }, false) => {
            let zoned = local.to_ambiguous_zoned(wall).compatible();
            match zoned {
                Ok(zoned) => Moment::Zoned {
                    at: zoned.with_time_zone(moment.zone(local)),
                    tzid: match moment {
                        Moment::Zoned { tzid, .. } => tzid.clone(),
                        _ => None,
                    },
                },
                Err(_) => moment.clone(),
            }
        }
        _ => local_moment(wall, all_day, local),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn reschedule(
    object: &CalendarObject,
    event_index: usize,
    key: InstanceKey,
    start: DateTime,
    end: DateTime,
    all_day: bool,
    scope: Scope,
    local: &TimeZone,
) -> CalendarObject {
    let mut object = object.clone();
    let Some(event) = object.events.get(event_index).cloned() else {
        return object;
    };
    let recurring = object
        .master()
        .is_some_and(|master| master.rule.is_some() || !master.rdates.is_empty());
    let editing_override = event.recurrence_id.is_some();
    if recurring && scope == Scope::Occurrence && !editing_override {
        let mut exception = event.clone();
        exception
            .component
            .properties
            .retain(|property| !["RRULE", "EXDATE", "RDATE"].contains(&property.name.as_str()));
        exception.rule = None;
        exception.exdates.clear();
        exception.rdates.clear();
        exception.recurrence_id = Some(moment_of(key, &event.start));
        exception.start = place(&event.start, start, all_day, local);
        exception.end = place(&event.end, end, all_day, local);
        exception.sequence += 1;
        object.events.push(exception);
        return object;
    }
    if recurring && scope == Scope::Series && !editing_override {
        let Some(master_index) = object
            .events
            .iter()
            .position(|event| event.recurrence_id.is_none())
        else {
            return object;
        };
        let occurrence = moment_of(key, &object.events[master_index].start)
            .to_zoned(local)
            .datetime();
        let master = &mut object.events[master_index];
        let shift = |moment: &Moment, wall: DateTime| -> DateTime {
            let delta = occurrence.duration_until(wall);
            moment
                .to_zoned(local)
                .datetime()
                .checked_add(delta)
                .unwrap_or(wall)
        };
        let new_start = shift(&master.start, start);
        let length = start.duration_until(end);
        let new_end = new_start.checked_add(length).unwrap_or(new_start);
        if let Some(rule) = &mut master.rule {
            follow_weekday(rule, occurrence.date(), start.date());
        }
        master.start = place(&master.start, new_start, all_day, local);
        master.end = place(&master.end, new_end, all_day, local);
        master.sequence += 1;
        return object;
    }
    if let Some(event) = object.events.get_mut(event_index) {
        event.start = place(&event.start, start, all_day, local);
        event.end = place(&event.end, end, all_day, local);
        event.sequence += 1;
    }
    object
}

pub fn remove_occurrence(
    object: &CalendarObject,
    key: InstanceKey,
    scope: Scope,
) -> Option<CalendarObject> {
    let recurring = object
        .master()
        .is_some_and(|master| master.rule.is_some() || !master.rdates.is_empty());
    if !recurring || scope == Scope::Series {
        return None;
    }
    let mut object = object.clone();
    object
        .events
        .retain(|event| event.recurrence_id.as_ref().map(Moment::key) != Some(key));
    let master = object.master_mut()?;
    let excluded = moment_of(key, &master.start);
    master.exdates.push(excluded);
    master.sequence += 1;
    Some(object)
}

pub fn apply_details(
    object: &CalendarObject,
    event_index: usize,
    key: InstanceKey,
    details: &Details,
    scope: Scope,
    local: &TimeZone,
) -> CalendarObject {
    let mut object = reschedule(
        object,
        event_index,
        key,
        details.start,
        details.end,
        details.all_day,
        scope,
        local,
    );
    let editing_override = object
        .events
        .get(event_index)
        .is_some_and(|event| event.recurrence_id.is_some());
    let target = match editing_override
        || scope == Scope::Occurrence && object.master().is_some_and(|master| master.rule.is_some())
    {
        true => object
            .events
            .iter()
            .position(|event| event.recurrence_id.as_ref().map(Moment::key) == Some(key))
            .unwrap_or(event_index),
        false => object
            .events
            .iter()
            .position(|event| event.recurrence_id.is_none())
            .unwrap_or(event_index),
    };
    if let Some(event) = object.events.get_mut(target) {
        event.summary = details.summary.trim().to_owned();
        event.location = details.location.trim().to_owned();
        event.description = details.notes.clone();
        event.alarms = details.alert.alarms(&event.alarms);
        if event.recurrence_id.is_none() {
            event.rule = details.repeat.rule(event.rule.as_ref());
        }
    }
    object
}

pub fn shifted(
    from: DateTime,
    to: DateTime,
    start: DateTime,
    end: DateTime,
) -> (DateTime, DateTime) {
    let delta = from.duration_until(to);
    (
        start.checked_add(delta).unwrap_or(start),
        end.checked_add(delta).unwrap_or(end),
    )
}
