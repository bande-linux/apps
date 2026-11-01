use std::collections::HashSet;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use super::lines::{self, Component, ParseError, Property, escape, split_list, unescape};
use super::rule::Rule;
use super::tz::{Zones, vtimezone};
use super::value::{Duration, InstanceKey, Moment};

const PRODUCT: &str = "-//Bande//Calendar 0.1//EN";

const MANAGED: [&str; 15] = [
    "UID",
    "SUMMARY",
    "LOCATION",
    "DESCRIPTION",
    "DTSTART",
    "DTEND",
    "DURATION",
    "RRULE",
    "EXDATE",
    "RDATE",
    "RECURRENCE-ID",
    "SEQUENCE",
    "DTSTAMP",
    "LAST-MODIFIED",
    "URL",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    Relative { offset: Duration, from_end: bool },
    Absolute(Timestamp),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alarm {
    pub trigger: Trigger,
    pub action: String,
    pub description: String,
    pub component: Component,
}

impl Alarm {
    pub fn before(offset: Duration) -> Self {
        Self {
            trigger: Trigger::Relative {
                offset,
                from_end: false,
            },
            action: "DISPLAY".to_owned(),
            description: String::new(),
            component: Component::new("VALARM"),
        }
    }

    fn parse(component: &Component) -> Option<Self> {
        let trigger = component.property("TRIGGER")?;
        let absolute = trigger
            .param("VALUE")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("DATE-TIME"));
        let trigger = match absolute {
            true => Trigger::Absolute(
                Moment::parse_text(&trigger.value, None, None, &Zones::default())?
                    .timestamp(&TimeZone::UTC),
            ),
            false => Trigger::Relative {
                offset: Duration::parse(&trigger.value)?,
                from_end: trigger
                    .param("RELATED")
                    .is_some_and(|related| related.eq_ignore_ascii_case("END")),
            },
        };
        Some(Self {
            trigger,
            action: component
                .value("ACTION")
                .unwrap_or("DISPLAY")
                .trim()
                .to_owned(),
            description: component
                .value("DESCRIPTION")
                .map(unescape)
                .unwrap_or_default(),
            component: component.clone(),
        })
    }

    fn to_component(&self, summary: &str) -> Component {
        let mut component = self.component.clone();
        component.properties.retain(|property| {
            !["TRIGGER", "ACTION", "DESCRIPTION"].contains(&property.name.as_str())
        });
        let trigger = match &self.trigger {
            Trigger::Relative { offset, from_end } => {
                let property = Property::new("TRIGGER", offset.to_text());
                match from_end {
                    true => property.with_param("RELATED", "END"),
                    false => property,
                }
            }
            Trigger::Absolute(at) => Moment::utc(*at)
                .property("TRIGGER")
                .with_param("VALUE", "DATE-TIME"),
        };
        component.push(trigger);
        component.push(Property::new("ACTION", self.action.clone()));
        let description = match self.description.is_empty() {
            true => summary,
            false => self.description.as_str(),
        };
        component.push(Property::new("DESCRIPTION", escape(description)));
        component
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub url: String,
    pub start: Moment,
    pub end: Moment,
    pub rule: Option<Rule>,
    pub exdates: Vec<Moment>,
    pub rdates: Vec<Moment>,
    pub recurrence_id: Option<Moment>,
    pub alarms: Vec<Alarm>,
    pub sequence: u32,
    pub component: Component,
}

fn moments(component: &Component, name: &str, zones: &Zones) -> Vec<Moment> {
    component
        .all(name)
        .flat_map(|property| {
            split_list(&property.value)
                .into_iter()
                .filter_map(|value| {
                    Moment::parse_text(
                        value,
                        property.param("TZID"),
                        property.param("VALUE"),
                        zones,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

impl Event {
    pub fn new(uid: String, summary: &str, start: Moment, end: Moment) -> Self {
        Self {
            uid,
            summary: summary.to_owned(),
            location: String::new(),
            description: String::new(),
            url: String::new(),
            start,
            end,
            rule: None,
            exdates: Vec::new(),
            rdates: Vec::new(),
            recurrence_id: None,
            alarms: Vec::new(),
            sequence: 0,
            component: Component::new("VEVENT"),
        }
    }

    fn parse(component: &Component, zones: &Zones) -> Option<Self> {
        let start = Moment::parse(component.property("DTSTART")?, zones)?;
        let end = component
            .property("DTEND")
            .and_then(|property| Moment::parse(property, zones))
            .or_else(|| {
                component
                    .value("DURATION")
                    .and_then(Duration::parse)
                    .map(|duration| start.add(&duration))
            })
            .unwrap_or_else(|| match start.is_date() {
                true => start.add(&Duration::from_days(1)),
                false => start.clone(),
            });
        let text = |name: &str| component.value(name).map(unescape).unwrap_or_default();
        Some(Self {
            uid: component.value("UID")?.trim().to_owned(),
            summary: text("SUMMARY"),
            location: text("LOCATION"),
            description: text("DESCRIPTION"),
            url: component.value("URL").unwrap_or_default().trim().to_owned(),
            rule: component
                .value("RRULE")
                .and_then(|rule| Rule::parse(rule, zones)),
            exdates: moments(component, "EXDATE", zones),
            rdates: moments(component, "RDATE", zones),
            recurrence_id: component
                .property("RECURRENCE-ID")
                .and_then(|property| Moment::parse(property, zones)),
            alarms: component
                .children("VALARM")
                .filter_map(Alarm::parse)
                .collect(),
            sequence: component
                .value("SEQUENCE")
                .and_then(|sequence| sequence.trim().parse().ok())
                .unwrap_or(0),
            start,
            end,
            component: component.clone(),
        })
    }

    pub fn all_day(&self) -> bool {
        self.start.is_date()
    }

    pub fn length(&self, local: &TimeZone) -> Duration {
        match (&self.start, &self.end) {
            (Moment::Date(start), Moment::Date(end)) => {
                Duration::from_days(i64::from((*end - *start).get_days()).max(1))
            }
            (start, end) => Duration::from_seconds(
                (end.timestamp(local).as_second() - start.timestamp(local).as_second()).max(0),
            ),
        }
    }

    pub fn to_component(&self, stamp: Timestamp) -> Component {
        let mut component = self.component.clone();
        component
            .properties
            .retain(|property| !MANAGED.contains(&property.name.as_str()));
        component.components.retain(|child| child.name != "VALARM");
        let mut managed = vec![
            Property::new("UID", self.uid.clone()),
            Moment::utc(stamp).property("DTSTAMP"),
            Moment::utc(stamp).property("LAST-MODIFIED"),
            self.start.property("DTSTART"),
            self.end.property("DTEND"),
        ];
        if let Some(recurrence_id) = &self.recurrence_id {
            managed.push(recurrence_id.property("RECURRENCE-ID"));
        }
        if self.sequence > 0 {
            managed.push(Property::new("SEQUENCE", self.sequence.to_string()));
        }
        for (name, value) in [
            ("SUMMARY", &self.summary),
            ("LOCATION", &self.location),
            ("DESCRIPTION", &self.description),
        ] {
            if !value.is_empty() {
                managed.push(Property::new(name, escape(value)));
            }
        }
        if !self.url.is_empty() {
            managed.push(Property::new("URL", self.url.clone()));
        }
        if let Some(rule) = &self.rule {
            managed.push(Property::new("RRULE", rule.to_text()));
        }
        managed.extend(self.exdates.iter().map(|moment| moment.property("EXDATE")));
        managed.extend(self.rdates.iter().map(|moment| moment.property("RDATE")));
        managed.append(&mut component.properties);
        component.properties = managed;
        component.components.extend(
            self.alarms
                .iter()
                .map(|alarm| alarm.to_component(&self.summary)),
        );
        component
    }

    fn tzids(&self) -> Vec<(String, TimeZone, i16)> {
        let mut found = Vec::new();
        for moment in [&self.start, &self.end]
            .into_iter()
            .chain(&self.exdates)
            .chain(&self.rdates)
            .chain(&self.recurrence_id)
        {
            if let Moment::Zoned {
                at,
                tzid: Some(tzid),
            } = moment
            {
                found.push((tzid.clone(), at.time_zone().clone(), at.year()));
            }
        }
        found
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarObject {
    pub calendar: Component,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub uid: String,
    pub key: InstanceKey,
    pub event: usize,
    pub start: Zoned,
    pub end: Zoned,
    pub all_day: bool,
    pub first_day: Date,
    pub last_day: Date,
    pub recurring: bool,
}

impl CalendarObject {
    pub fn parse(text: &str) -> Result<Vec<Self>, ParseError> {
        let roots = lines::parse(text)?;
        if !roots.iter().any(|root| root.name == "VCALENDAR") {
            return Err(ParseError::Empty);
        }
        let mut objects = Vec::new();
        for root in roots.into_iter().filter(|root| root.name == "VCALENDAR") {
            let zones = Zones::from_calendar(&root);
            let mut shell = root.clone();
            shell.components.retain(|child| child.name != "VEVENT");
            let mut by_uid: Vec<Self> = Vec::new();
            for event in root
                .children("VEVENT")
                .filter_map(|event| Event::parse(event, &zones))
            {
                match by_uid.iter_mut().find(|object| object.uid() == event.uid) {
                    Some(object) => object.events.push(event),
                    None => by_uid.push(Self {
                        calendar: shell.clone(),
                        events: vec![event],
                    }),
                }
            }
            objects.extend(by_uid);
        }
        Ok(objects)
    }

    pub fn new(event: Event) -> Self {
        let mut calendar = Component::new("VCALENDAR");
        calendar.push(Property::new("VERSION", "2.0"));
        calendar.push(Property::new("PRODID", PRODUCT));
        Self {
            calendar,
            events: vec![event],
        }
    }

    pub fn uid(&self) -> &str {
        self.events.first().map_or("", |event| event.uid.as_str())
    }

    pub fn master(&self) -> Option<&Event> {
        self.events
            .iter()
            .find(|event| event.recurrence_id.is_none())
    }

    pub fn master_mut(&mut self) -> Option<&mut Event> {
        self.events
            .iter_mut()
            .find(|event| event.recurrence_id.is_none())
    }

    pub fn to_ics(&self, stamp: Timestamp) -> String {
        let mut calendar = self.calendar.clone();
        let present: HashSet<String> = calendar
            .children("VTIMEZONE")
            .filter_map(|zone| zone.value("TZID").map(|tzid| tzid.trim().to_owned()))
            .collect();
        let mut added = HashSet::new();
        for (tzid, zone, year) in self.events.iter().flat_map(Event::tzids) {
            if !present.contains(&tzid) && added.insert(tzid.clone()) {
                calendar.components.push(vtimezone(&zone, &tzid, year));
            }
        }
        calendar
            .components
            .extend(self.events.iter().map(|event| event.to_component(stamp)));
        calendar.to_ics()
    }

    pub fn occurrences(&self, from: Timestamp, to: Timestamp, local: &TimeZone) -> Vec<Occurrence> {
        let Some(master) = self.master() else {
            return self
                .events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| occurrence(event, index, &event.start, local, false))
                .filter(|found| overlaps(found, from, to))
                .collect();
        };
        let master_index = self
            .events
            .iter()
            .position(|event| event.recurrence_id.is_none())
            .unwrap_or(0);
        let recurring = master.rule.is_some() || !master.rdates.is_empty();
        let overrides: Vec<(InstanceKey, usize)> = self
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| Some((event.recurrence_id.as_ref()?.key(), index)))
            .collect();
        let excluded: HashSet<InstanceKey> = master.exdates.iter().map(Moment::key).collect();
        let length = master.length(local);
        let mut starts = Vec::new();
        let slack = Duration::from_days(2);
        let latest = Moment::utc(to)
            .add(&slack)
            .to_zoned(&master.start.zone(local))
            .datetime();
        match &master.rule {
            Some(rule) => rule.expand(&master.start, local, latest, |moment| {
                let keep = moment.wall() <= latest;
                if keep {
                    starts.push(moment.clone());
                }
                keep
            }),
            None => starts.push(master.start.clone()),
        }
        starts.extend(master.rdates.iter().cloned());
        let mut found: Vec<Occurrence> = Vec::new();
        let mut seen = HashSet::new();
        for start in starts {
            let key = start.key();
            if excluded.contains(&key) || !seen.insert(key) {
                continue;
            }
            let shown = match overrides.iter().find(|(overridden, _)| *overridden == key) {
                Some((_, index)) => {
                    let event = &self.events[*index];
                    occurrence(event, *index, &event.start, local, recurring)
                }
                None => {
                    let end = start.add(&length);
                    occurrence_between(master, master_index, &start, &end, local, recurring)
                }
            };
            if let Some(mut shown) = shown.filter(|shown| overlaps(shown, from, to)) {
                shown.key = key;
                found.push(shown);
            }
        }
        for (key, index) in &overrides {
            if seen.contains(key) {
                continue;
            }
            let event = &self.events[*index];
            if let Some(mut shown) = occurrence(event, *index, &event.start, local, recurring)
                .filter(|shown| overlaps(shown, from, to))
            {
                shown.key = *key;
                found.push(shown);
            }
        }
        found.sort_by(|a, b| a.start.cmp(&b.start));
        found
    }
}

fn overlaps(found: &Occurrence, from: Timestamp, to: Timestamp) -> bool {
    let end = found
        .end
        .timestamp()
        .max(found.start.timestamp() + jiff::SignedDuration::from_secs(1));
    found.start.timestamp() < to && end > from
}

fn occurrence(
    event: &Event,
    index: usize,
    start: &Moment,
    local: &TimeZone,
    recurring: bool,
) -> Option<Occurrence> {
    occurrence_between(event, index, start, &event.end, local, recurring)
}

fn occurrence_between(
    event: &Event,
    index: usize,
    start: &Moment,
    end: &Moment,
    local: &TimeZone,
    recurring: bool,
) -> Option<Occurrence> {
    let all_day = start.is_date();
    let shown_start = start.to_zoned(local);
    let shown_end = end.to_zoned(local).max(shown_start.clone());
    let (first_day, last_day) = match all_day {
        true => (start.date(), end.date().yesterday().ok()?.max(start.date())),
        false => {
            let last = match shown_end.time() == jiff::civil::Time::midnight()
                && shown_end > shown_start
            {
                true => shown_end.date().yesterday().ok()?,
                false => shown_end.date(),
            };
            (shown_start.date(), last.max(shown_start.date()))
        }
    };
    Some(Occurrence {
        uid: event.uid.clone(),
        key: start.key(),
        event: index,
        start: shown_start,
        end: shown_end,
        all_day,
        first_day,
        last_day,
        recurring,
    })
}
