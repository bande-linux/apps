use std::io::Read;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::ical::{CalendarObject, Event, Occurrence};

const ACCOUNTS_FILE: &str = "accounts.json";
const CALENDARS_DIR: &str = "calendars";
pub const LOCAL_ACCOUNT: &str = "local";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    Local,
    CalDav {
        server: String,
        user: String,
        home: String,
    },
    Subscription {
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarInfo {
    pub id: String,
    pub account: String,
    pub url: Option<String>,
    pub name: String,
    pub color: String,
    pub read_only: bool,
    pub visible: bool,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pending {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    pub href: String,
    pub etag: Option<String>,
    pub data: String,
    pub pending: Option<Pending>,
    #[serde(skip)]
    pub parsed: Option<CalendarObject>,
}

impl Stored {
    pub fn new(href: String, etag: Option<String>, data: String, pending: Option<Pending>) -> Self {
        let parsed = parse_one(&data);
        Self {
            href,
            etag,
            data,
            pending,
            parsed,
        }
    }

    fn reparse(&mut self) {
        self.parsed = parse_one(&self.data);
    }
}

fn parse_one(data: &str) -> Option<CalendarObject> {
    CalendarObject::parse(data)
        .map_err(|error| tracing::debug!("cannot read an event: {error}"))
        .ok()?
        .into_iter()
        .next()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    pub info: CalendarInfo,
    pub objects: Vec<Stored>,
}

impl Calendar {
    pub fn new(info: CalendarInfo) -> Self {
        Self {
            info,
            objects: Vec::new(),
        }
    }

    pub fn find(&self, uid: &str) -> Option<usize> {
        self.objects.iter().position(|stored| {
            stored.pending != Some(Pending::Delete)
                && stored
                    .parsed
                    .as_ref()
                    .is_some_and(|object| object.uid() == uid)
        })
    }

    pub fn has_pending(&self) -> bool {
        self.objects.iter().any(|stored| stored.pending.is_some())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub calendar: usize,
    pub object: usize,
    pub occurrence: Occurrence,
}

#[derive(Debug, Default)]
pub struct Library {
    dir: Option<PathBuf>,
    pub accounts: Vec<Account>,
    pub calendars: Vec<Calendar>,
}

pub fn new_uid() -> String {
    let mut bytes = [0u8; 16];
    let read =
        std::fs::File::open("/dev/urandom").and_then(|mut random| random.read_exact(&mut bytes));
    if read.is_err() {
        let now = Timestamp::now().as_nanosecond().to_le_bytes();
        bytes.copy_from_slice(&now);
        bytes[0] ^= std::process::id() as u8;
    }
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn write_atomically(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staging = file.with_extension("new");
    std::fs::write(&staging, text)?;
    std::fs::rename(&staging, file)
}

pub fn href_for(calendar: &CalendarInfo, uid: &str) -> String {
    let safe: String = uid
        .chars()
        .map(
            |character| match character.is_ascii_alphanumeric() || "-_.".contains(character) {
                true => character,
                false => '_',
            },
        )
        .collect();
    match &calendar.url {
        Some(url) => format!("{}/{safe}.ics", url.trim_end_matches('/')),
        None => format!("{safe}.ics"),
    }
}

fn calendar_file(dir: &Path, id: &str) -> PathBuf {
    let name: String = id
        .chars()
        .map(
            |character| match character.is_ascii_alphanumeric() || character == '-' {
                true => character,
                false => '_',
            },
        )
        .collect();
    dir.join(CALENDARS_DIR).join(format!("{name}.json"))
}

impl Library {
    pub fn load(dir: PathBuf) -> Self {
        let accounts: Vec<Account> = std::fs::read_to_string(dir.join(ACCOUNTS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let mut calendars: Vec<Calendar> = std::fs::read_dir(dir.join(CALENDARS_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|entry| {
                let text = std::fs::read_to_string(entry.path()).ok()?;
                serde_json::from_str::<Calendar>(&text)
                    .map_err(|error| {
                        tracing::warn!("cannot read {}: {error}", entry.path().display())
                    })
                    .ok()
            })
            .collect();
        for calendar in &mut calendars {
            for stored in &mut calendar.objects {
                stored.reparse();
            }
        }
        calendars.retain(|calendar| {
            accounts
                .iter()
                .any(|account| account.id == calendar.info.account)
        });
        calendars.sort_by(|a, b| a.info.name.cmp(&b.info.name));
        let mut library = Self {
            dir: Some(dir),
            accounts,
            calendars,
        };
        library.ensure_local();
        library
    }

    fn ensure_local(&mut self) {
        if !self
            .accounts
            .iter()
            .any(|account| account.kind == AccountKind::Local)
        {
            self.accounts.insert(
                0,
                Account {
                    id: LOCAL_ACCOUNT.to_owned(),
                    name: String::new(),
                    kind: AccountKind::Local,
                },
            );
            self.save_accounts();
        }
        if !self
            .calendars
            .iter()
            .any(|calendar| calendar.info.account == LOCAL_ACCOUNT)
        {
            let calendar = Calendar::new(CalendarInfo {
                id: new_uid(),
                account: LOCAL_ACCOUNT.to_owned(),
                url: None,
                name: "Calendar".to_owned(),
                color: "#3478F6".to_owned(),
                read_only: false,
                visible: true,
                ctag: None,
                sync_token: None,
            });
            self.calendars.push(calendar);
            self.save(self.calendars.len() - 1);
        }
    }

    pub fn save_accounts(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        let written = serde_json::to_string_pretty(&self.accounts)
            .map_err(std::io::Error::other)
            .and_then(|text| write_atomically(&dir.join(ACCOUNTS_FILE), &text));
        if let Err(error) = written {
            tracing::warn!("cannot save the accounts: {error}");
        }
    }

    pub fn save(&self, index: usize) {
        let (Some(dir), Some(calendar)) = (&self.dir, self.calendars.get(index)) else {
            return;
        };
        let file = calendar_file(dir, &calendar.info.id);
        let written = serde_json::to_string(calendar)
            .map_err(std::io::Error::other)
            .and_then(|text| write_atomically(&file, &text));
        if let Err(error) = written {
            tracing::warn!("cannot save {}: {error}", calendar.info.name);
        }
    }

    pub fn add_account(&mut self, account: Account, calendars: Vec<CalendarInfo>) {
        self.accounts.push(account);
        self.save_accounts();
        for info in calendars {
            self.calendars.push(Calendar::new(info));
            self.save(self.calendars.len() - 1);
        }
    }

    pub fn remove_account(&mut self, id: &str) {
        if id == LOCAL_ACCOUNT {
            return;
        }
        self.accounts.retain(|account| account.id != id);
        if let Some(dir) = &self.dir {
            for calendar in self
                .calendars
                .iter()
                .filter(|calendar| calendar.info.account == id)
            {
                let file = calendar_file(dir, &calendar.info.id);
                if let Err(error) = std::fs::remove_file(&file) {
                    tracing::warn!("cannot remove {}: {error}", file.display());
                }
            }
        }
        self.calendars
            .retain(|calendar| calendar.info.account != id);
        self.save_accounts();
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.id == id)
    }

    pub fn calendar_index(&self, id: &str) -> Option<usize> {
        self.calendars
            .iter()
            .position(|calendar| calendar.info.id == id)
    }

    pub fn writable(&self) -> impl Iterator<Item = (usize, &Calendar)> {
        self.calendars
            .iter()
            .enumerate()
            .filter(|(_, calendar)| !calendar.info.read_only)
    }

    pub fn occurrences(&self, from: Timestamp, to: Timestamp, local: &TimeZone) -> Vec<Found> {
        let mut found = Vec::new();
        for (calendar_index, calendar) in self.calendars.iter().enumerate() {
            if !calendar.info.visible {
                continue;
            }
            for (object_index, stored) in calendar.objects.iter().enumerate() {
                if stored.pending == Some(Pending::Delete) {
                    continue;
                }
                let Some(object) = &stored.parsed else {
                    continue;
                };
                found.extend(
                    object
                        .occurrences(from, to, local)
                        .into_iter()
                        .map(|occurrence| Found {
                            calendar: calendar_index,
                            object: object_index,
                            occurrence,
                        }),
                );
            }
        }
        found.sort_by(|a, b| {
            (!a.occurrence.all_day, &a.occurrence.start)
                .cmp(&(!b.occurrence.all_day, &b.occurrence.start))
        });
        found
    }

    pub fn event(&self, found: &Found) -> Option<&Event> {
        self.calendars
            .get(found.calendar)?
            .objects
            .get(found.object)?
            .parsed
            .as_ref()?
            .events
            .get(found.occurrence.event)
    }

    pub fn object(&self, calendar: usize, object: usize) -> Option<&CalendarObject> {
        self.calendars
            .get(calendar)?
            .objects
            .get(object)?
            .parsed
            .as_ref()
    }

    pub fn search(&self, text: &str, around: Timestamp, local: &TimeZone) -> Vec<Found> {
        let needle = text.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let span = jiff::SignedDuration::from_hours(24 * 366 * 2);
        let (from, to) = (
            around.saturating_sub(span).unwrap_or(around),
            around.saturating_add(span).unwrap_or(around),
        );
        let matches = |event: &Event| {
            [&event.summary, &event.location, &event.description]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        };
        let mut found: Vec<Found> = Vec::new();
        for (calendar_index, calendar) in self.calendars.iter().enumerate() {
            if !calendar.info.visible {
                continue;
            }
            for (object_index, stored) in calendar.objects.iter().enumerate() {
                let Some(object) = stored
                    .parsed
                    .as_ref()
                    .filter(|_| stored.pending != Some(Pending::Delete))
                else {
                    continue;
                };
                if !object.events.iter().any(matches) {
                    continue;
                }
                let occurrences = object.occurrences(from, to, local);
                let next = occurrences
                    .iter()
                    .find(|occurrence| occurrence.end.timestamp() >= around)
                    .or(occurrences.last());
                if let Some(occurrence) = next
                    .filter(|occurrence| object.events.get(occurrence.event).is_some_and(matches))
                {
                    found.push(Found {
                        calendar: calendar_index,
                        object: object_index,
                        occurrence: occurrence.clone(),
                    });
                }
            }
        }
        found.sort_by(|a, b| a.occurrence.start.cmp(&b.occurrence.start));
        found
    }

    pub fn put_object(&mut self, calendar: usize, object: CalendarObject) -> Option<usize> {
        let info = self.calendars.get(calendar)?.info.clone();
        if info.read_only {
            return None;
        }
        let data = object.to_ics(Timestamp::now());
        let local = info.account == LOCAL_ACCOUNT;
        let target = self.calendars.get_mut(calendar)?;
        let index = match target.find(object.uid()) {
            Some(index) => {
                let stored = &mut target.objects[index];
                stored.pending = match (local, stored.pending) {
                    (true, _) => None,
                    (false, Some(Pending::Create)) => Some(Pending::Create),
                    (false, _) => Some(Pending::Update),
                };
                stored.data = data;
                stored.parsed = Some(object);
                index
            }
            None => {
                let href = href_for(&info, object.uid());
                let pending = (!local).then_some(Pending::Create);
                let mut stored = Stored::new(href, None, data, pending);
                stored.parsed = Some(object);
                target.objects.push(stored);
                target.objects.len() - 1
            }
        };
        self.save(calendar);
        Some(index)
    }

    pub fn remove_object(&mut self, calendar: usize, object: usize) {
        let Some(target) = self.calendars.get_mut(calendar) else {
            return;
        };
        if target.info.read_only {
            return;
        }
        let local = target.info.account == LOCAL_ACCOUNT;
        let Some(stored) = target.objects.get_mut(object) else {
            return;
        };
        match (local, stored.pending) {
            (true, _) | (false, Some(Pending::Create)) => {
                target.objects.remove(object);
            }
            (false, _) => stored.pending = Some(Pending::Delete),
        }
        self.save(calendar);
    }

    pub fn move_object(&mut self, from: usize, object: usize, to: usize) -> Option<usize> {
        let moved = self.object(from, object)?.clone();
        if from == to || self.calendars.get(to)?.info.read_only {
            return None;
        }
        let index = self.put_object(to, moved)?;
        self.remove_object(from, object);
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::Moment;
    use jiff::civil::date;

    fn event(uid: &str, summary: &str, day: i8) -> CalendarObject {
        let start = Moment::Floating(date(2026, 11, day).at(10, 0, 0, 0));
        let end = Moment::Floating(date(2026, 11, day).at(11, 0, 0, 0));
        CalendarObject::new(Event::new(uid.to_owned(), summary, start, end))
    }

    fn remote(library: &mut Library) -> usize {
        library.add_account(
            Account {
                id: "dav".to_owned(),
                name: "Work".to_owned(),
                kind: AccountKind::CalDav {
                    server: "https://dav.example.com".to_owned(),
                    user: "jane".to_owned(),
                    home: "https://dav.example.com/jane/".to_owned(),
                },
            },
            vec![CalendarInfo {
                id: "work".to_owned(),
                account: "dav".to_owned(),
                url: Some("https://dav.example.com/jane/work/".to_owned()),
                name: "Work".to_owned(),
                color: "#FF0000".to_owned(),
                read_only: false,
                visible: true,
                ctag: None,
                sync_token: None,
            }],
        );
        library.calendars.len() - 1
    }

    #[test]
    fn edits_queue_the_right_pending_change() {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::load(directory.path().to_owned());
        let work = remote(&mut library);
        let index = library.put_object(work, event("a", "Plan", 2)).unwrap();
        assert_eq!(
            library.calendars[work].objects[index].pending,
            Some(Pending::Create)
        );
        assert_eq!(
            library.calendars[work].objects[index].href,
            "https://dav.example.com/jane/work/a.ics"
        );
        library.put_object(work, event("a", "Plan v2", 2));
        assert_eq!(
            library.calendars[work].objects[index].pending,
            Some(Pending::Create)
        );
        library.calendars[work].objects[index].pending = None;
        library.put_object(work, event("a", "Plan v3", 2));
        assert_eq!(
            library.calendars[work].objects[index].pending,
            Some(Pending::Update)
        );
        library.remove_object(work, index);
        assert_eq!(
            library.calendars[work].objects[index].pending,
            Some(Pending::Delete)
        );
        let reloaded = Library::load(directory.path().to_owned());
        let work = reloaded.calendar_index("work").unwrap();
        assert_eq!(
            reloaded.calendars[work].objects[0].pending,
            Some(Pending::Delete)
        );
        assert!(
            reloaded.calendars[work].objects[0]
                .data
                .contains("SUMMARY:Plan v3")
        );
        assert_eq!(reloaded.calendars.len(), 2);
    }

    #[test]
    fn moving_between_calendars_deletes_from_the_source() {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::load(directory.path().to_owned());
        let local = library
            .calendars
            .iter()
            .position(|calendar| calendar.info.account == LOCAL_ACCOUNT)
            .unwrap();
        let work = remote(&mut library);
        library.put_object(local, event("b", "Dentist", 3));
        assert!(library.calendars[local].objects[0].pending.is_none());
        library.move_object(local, 0, work);
        assert!(library.calendars[local].objects.is_empty());
        assert_eq!(
            library.calendars[work].objects[0].pending,
            Some(Pending::Create)
        );
    }

    #[test]
    fn search_finds_text_in_any_field_and_hidden_calendars_stay_hidden() {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::load(directory.path().to_owned());
        let work = remote(&mut library);
        library.put_object(work, event("c", "Design review", 4));
        let mut lunch = event("d", "Lunch", 5);
        if let Some(master) = lunch.master_mut() {
            master.location = "Review café".to_owned();
        }
        library.put_object(work, lunch);
        library.put_object(work, event("e", "Gym", 6));
        let around = date(2026, 11, 1)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp();
        let found = library.search("review", around, &TimeZone::UTC);
        assert_eq!(found.len(), 2);
        library.calendars[work].info.visible = false;
        assert!(library.search("review", around, &TimeZone::UTC).is_empty());
    }
}
