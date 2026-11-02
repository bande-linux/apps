use std::collections::HashMap;

use jiff::Timestamp;

use crate::caldav::{Client, Error, Precondition, Resource, same_resource};
use crate::ical::CalendarObject;
use crate::store::{Calendar, CalendarInfo, Library, Pending, Stored, href_for, new_uid};

#[derive(Debug, Clone, PartialEq)]
pub struct Upsert {
    pub href: String,
    pub etag: Option<String>,
    pub data: String,
    pub pushed: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub calendar: String,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub upserts: Vec<Upsert>,
    pub removed: Vec<String>,
    pub conflicts: Vec<String>,
    pub copies: Vec<Upsert>,
}

fn summary(data: &str) -> String {
    CalendarObject::parse(data)
        .ok()
        .and_then(|objects| objects.into_iter().next())
        .and_then(|object| {
            object
                .master()
                .or(object.events.first())
                .map(|event| event.summary.clone())
        })
        .unwrap_or_default()
}

fn kept_copy(info: &CalendarInfo, data: &str) -> Option<Upsert> {
    let mut object = CalendarObject::parse(data).ok()?.into_iter().next()?;
    let uid = new_uid();
    for event in &mut object.events {
        event.uid = uid.clone();
    }
    Some(Upsert {
        href: href_for(info, &uid),
        etag: None,
        data: object.to_ics(Timestamp::now()),
        pushed: None,
    })
}

fn conflict_copy(
    client: &Client,
    info: &CalendarInfo,
    stored: &Stored,
    outcome: &mut Outcome,
) -> Result<(), Error> {
    outcome.conflicts.push(summary(&stored.data));
    if stored.pending == Some(Pending::Update) {
        outcome.copies.extend(kept_copy(info, &stored.data));
    }
    match client.get(&stored.href) {
        Ok(Resource { etag, data, .. }) => outcome.upserts.push(Upsert {
            href: stored.href.clone(),
            etag,
            data,
            pushed: Some(stored.data.clone()),
        }),
        Err(Error::Status { status: 404, .. }) => outcome.removed.push(stored.href.clone()),
        Err(error) => return Err(error),
    }
    Ok(())
}

fn push(client: &Client, calendar: &Calendar, outcome: &mut Outcome) -> Result<(), Error> {
    for stored in &calendar.objects {
        let Some(pending) = stored.pending else {
            continue;
        };
        let result = match pending {
            Pending::Create => {
                match client.put(&stored.href, &stored.data, &Precondition::Create) {
                    Err(Error::PreconditionFailed) => {
                        client.get(&stored.href).and_then(|existing| {
                            let precondition = existing
                                .etag
                                .map_or(Precondition::None, Precondition::Match);
                            client.put(&stored.href, &stored.data, &precondition)
                        })
                    }
                    other => other,
                }
                .map(Some)
            }
            Pending::Update => {
                let precondition = match &stored.etag {
                    Some(etag) => Precondition::Match(etag.clone()),
                    None => Precondition::None,
                };
                client
                    .put(&stored.href, &stored.data, &precondition)
                    .map(Some)
            }
            Pending::Delete => client
                .delete(&stored.href, stored.etag.as_deref())
                .map(|()| None),
        };
        match result {
            Ok(Some(etag)) => outcome.upserts.push(Upsert {
                href: stored.href.clone(),
                etag,
                data: stored.data.clone(),
                pushed: Some(stored.data.clone()),
            }),
            Ok(None) => outcome.removed.push(stored.href.clone()),
            Err(Error::PreconditionFailed) => {
                conflict_copy(client, &calendar.info, stored, outcome)?
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn known_etags(calendar: &Calendar, outcome: &Outcome) -> HashMap<String, Option<String>> {
    let mut known: HashMap<String, Option<String>> = calendar
        .objects
        .iter()
        .filter(|stored| stored.pending != Some(Pending::Create))
        .map(|stored| (stored.href.clone(), stored.etag.clone()))
        .collect();
    for upsert in &outcome.upserts {
        known.insert(upsert.href.clone(), upsert.etag.clone());
    }
    for removed in &outcome.removed {
        known.remove(removed);
    }
    known
}

fn fetch_changed(
    client: &Client,
    url: &str,
    listed: Vec<(String, Option<String>)>,
    known: &HashMap<String, Option<String>>,
    outcome: &mut Outcome,
) -> Result<(), Error> {
    let wanted: Vec<String> = listed
        .into_iter()
        .filter(|(href, etag)| {
            let mine = known
                .iter()
                .find(|(known, _)| same_resource(known, href))
                .map(|(_, etag)| etag);
            etag.is_none() || mine != Some(etag)
        })
        .map(|(href, _)| href)
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    for resource in client.multiget(url, &wanted)? {
        let href = known
            .keys()
            .find(|known| same_resource(known, &resource.url))
            .cloned()
            .unwrap_or(resource.url);
        match outcome
            .upserts
            .iter_mut()
            .find(|upsert| upsert.href == href)
        {
            Some(upsert) => {
                upsert.etag = resource.etag;
                upsert.data = resource.data;
            }
            None => outcome.upserts.push(Upsert {
                href,
                etag: resource.etag,
                data: resource.data,
                pushed: None,
            }),
        }
    }
    Ok(())
}

pub fn sync_calendar(client: &Client, calendar: &Calendar) -> Result<Outcome, Error> {
    let url = calendar
        .info
        .url
        .clone()
        .ok_or_else(|| Error::NotFound(calendar.info.name.clone()))?;
    let mut outcome = Outcome {
        calendar: calendar.info.id.clone(),
        ctag: calendar.info.ctag.clone(),
        sync_token: calendar.info.sync_token.clone(),
        upserts: Vec::new(),
        removed: Vec::new(),
        conflicts: Vec::new(),
        copies: Vec::new(),
    };
    push(client, calendar, &mut outcome)?;
    let known = known_etags(calendar, &outcome);
    let (ctag, token) = client.collection_tags(&url)?;
    if token.is_some() {
        let delta = match client.sync_collection(&url, outcome.sync_token.as_deref()) {
            Ok(delta) => Some(delta),
            Err(Error::Status {
                status: 403 | 409 | 400,
                ..
            }) if outcome.sync_token.is_some() => None,
            Err(error) => return Err(error),
        };
        let (delta, full) = match delta {
            Some(delta) => (delta, outcome.sync_token.is_none()),
            None => (client.sync_collection(&url, None)?, true),
        };
        if full {
            let listed: Vec<&String> = delta.changed.iter().map(|(href, _)| href).collect();
            for href in known.keys() {
                if !listed.iter().any(|listed| same_resource(listed, href))
                    && !outcome.removed.contains(href)
                {
                    outcome.removed.push(href.clone());
                }
            }
        }
        for href in &delta.removed {
            if let Some(mine) = known.keys().find(|known| same_resource(known, href)) {
                outcome.removed.push(mine.clone());
            }
        }
        fetch_changed(client, &url, delta.changed, &known, &mut outcome)?;
        outcome.sync_token = delta.token.or(token);
    } else if ctag.is_none() || ctag != outcome.ctag || calendar.has_pending() {
        let listed = client.etags(&url)?;
        for href in known.keys() {
            if !listed.iter().any(|(listed, _)| same_resource(listed, href))
                && !outcome.removed.contains(href)
            {
                outcome.removed.push(href.clone());
            }
        }
        fetch_changed(client, &url, listed, &known, &mut outcome)?;
    }
    outcome.ctag = ctag;
    Ok(outcome)
}

pub fn subscription_outcome(info: &CalendarInfo, data: &str) -> Option<Outcome> {
    let objects = CalendarObject::parse(data).ok()?;
    let stamp = Timestamp::now();
    Some(Outcome {
        calendar: info.id.clone(),
        ctag: None,
        sync_token: None,
        upserts: objects
            .iter()
            .map(|object| Upsert {
                href: format!("{}.ics", object.uid()),
                etag: None,
                data: object.to_ics(stamp),
                pushed: None,
            })
            .collect(),
        removed: Vec::new(),
        conflicts: Vec::new(),
        copies: Vec::new(),
    })
}

pub fn apply(library: &mut Library, outcome: Outcome, replace_all: bool) -> Vec<String> {
    let Some(index) = library.calendar_index(&outcome.calendar) else {
        return Vec::new();
    };
    let calendar = &mut library.calendars[index];
    calendar.info.ctag = outcome.ctag;
    calendar.info.sync_token = outcome.sync_token;
    if replace_all {
        calendar.objects.clear();
    }
    calendar
        .objects
        .retain(|stored| !outcome.removed.contains(&stored.href));
    for upsert in outcome.upserts {
        match calendar
            .objects
            .iter_mut()
            .find(|stored| stored.href == upsert.href)
        {
            Some(stored) => {
                let edited_meanwhile = stored.pending.is_some()
                    && upsert.pushed.as_deref() != Some(stored.data.as_str());
                stored.etag = upsert.etag;
                if !edited_meanwhile {
                    *stored =
                        Stored::new(stored.href.clone(), stored.etag.clone(), upsert.data, None);
                } else if stored.pending == Some(Pending::Create) {
                    stored.pending = Some(Pending::Update);
                }
            }
            None => calendar
                .objects
                .push(Stored::new(upsert.href, upsert.etag, upsert.data, None)),
        }
    }
    calendar.objects.extend(
        outcome
            .copies
            .into_iter()
            .map(|copy| Stored::new(copy.href, None, copy.data, Some(Pending::Create))),
    );
    library.save(index);
    outcome.conflicts
}
