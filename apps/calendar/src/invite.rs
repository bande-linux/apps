use crate::ical::{CalendarObject, Event, Property};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    Tentative,
    Declined,
    Pending,
}

impl Answer {
    pub const CHOICES: [Self; 3] = [Self::Accepted, Self::Tentative, Self::Declined];

    fn of(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_uppercase).as_deref() {
            Some("ACCEPTED") => Self::Accepted,
            Some("TENTATIVE") => Self::Tentative,
            Some("DECLINED") => Self::Declined,
            _ => Self::Pending,
        }
    }

    fn value(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Tentative => "TENTATIVE",
            Self::Declined => "DECLINED",
            Self::Pending => "NEEDS-ACTION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitee {
    pub address: String,
    pub name: String,
    pub answer: Answer,
    pub organizer: bool,
}

fn address(value: &str) -> &str {
    let value = value.trim();
    match value.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => value.get(7..).unwrap_or(value),
        _ => value,
    }
}

fn same(property: &Property, me: &[String]) -> bool {
    let found = address(&property.value);
    me.iter()
        .any(|mine| found.eq_ignore_ascii_case(mine.trim()))
}

fn invitee(property: &Property, organizer: bool) -> Invitee {
    Invitee {
        address: address(&property.value).to_owned(),
        name: property
            .param("CN")
            .map(|name| name.trim_matches('"').to_owned())
            .unwrap_or_default(),
        answer: match organizer {
            true => Answer::Accepted,
            false => Answer::of(property.param("PARTSTAT")),
        },
        organizer,
    }
}

pub fn invitees(event: &Event) -> Vec<Invitee> {
    let organizer = event.component.property("ORGANIZER");
    organizer
        .map(|property| invitee(property, true))
        .into_iter()
        .chain(
            event
                .component
                .all("ATTENDEE")
                .filter(|attendee| {
                    organizer.is_none_or(|organizer| {
                        !address(&attendee.value).eq_ignore_ascii_case(address(&organizer.value))
                    })
                })
                .map(|attendee| invitee(attendee, false)),
        )
        .collect()
}

pub fn my_answer(event: &Event, me: &[String]) -> Option<Answer> {
    let organizer = event.component.property("ORGANIZER")?;
    if same(organizer, me) {
        return None;
    }
    event
        .component
        .all("ATTENDEE")
        .find(|attendee| same(attendee, me))
        .map(|attendee| Answer::of(attendee.param("PARTSTAT")))
}

pub fn answer(object: &CalendarObject, me: &[String], answer: Answer) -> CalendarObject {
    let mut object = object.clone();
    for event in &mut object.events {
        for attendee in
            event.component.properties.iter_mut().filter(|property| {
                property.name.eq_ignore_ascii_case("ATTENDEE") && same(property, me)
            })
        {
            attendee.params.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("PARTSTAT") && !name.eq_ignore_ascii_case("RSVP")
            });
            attendee
                .params
                .push(("PARTSTAT".to_owned(), answer.value().to_owned()));
        }
    }
    object
}

pub fn parse_addresses(text: &str) -> Vec<String> {
    text.split([',', ';', ' ', '\n'])
        .map(|part| part.trim().trim_matches(['<', '>']))
        .filter(|part| {
            part.split_once('@')
                .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'))
        })
        .map(str::to_owned)
        .collect()
}

pub fn invite(event: &mut Event, me: &[String], addresses: &[String]) {
    let Some(organizer) = me.first().filter(|_| !addresses.is_empty()) else {
        return;
    };
    let component = &mut event.component;
    if component.property("ORGANIZER").is_none() {
        component.push(Property::new("ORGANIZER", format!("mailto:{organizer}")));
        component.push(
            Property::new("ATTENDEE", format!("mailto:{organizer}"))
                .with_param("PARTSTAT", "ACCEPTED")
                .with_param("ROLE", "CHAIR"),
        );
    }
    for address in addresses {
        if component
            .all("ATTENDEE")
            .any(|attendee| same(attendee, std::slice::from_ref(address)))
        {
            continue;
        }
        component.push(
            Property::new("ATTENDEE", format!("mailto:{address}"))
                .with_param("PARTSTAT", "NEEDS-ACTION")
                .with_param("ROLE", "REQ-PARTICIPANT")
                .with_param("RSVP", "TRUE"),
        );
    }
}
