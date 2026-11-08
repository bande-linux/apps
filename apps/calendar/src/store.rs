use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};

use bande_l10n::tr;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::ical::{CalendarObject, Event, Occurrence};

pub const ACCOUNTS_FILE: &str = "accounts.json";
const CALENDARS_DIR: &str = "calendars";
pub const LOCAL_ACCOUNT: &str = "local";
const PROGRAM: &str = "bande-calendar";

pub fn data_dir() -> PathBuf {
    bande_xdg::data_home()
        .unwrap_or_else(std::env::temp_dir)
        .join(PROGRAM)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    Local,
    CalDav {
        server: String,
        user: String,
        home: String,
        #[serde(default)]
        addresses: Vec<String>,
    },
    Subscription {
        url: String,
        #[serde(default)]
        refresh_minutes: Option<u32>,
        #[serde(default)]
        alerts: bool,
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
    #[serde(default)]
    pub refreshed: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pending {
    Create,
    Update,
    Delete,
}

type Expansion = (Timestamp, Timestamp, TimeZone, Vec<Occurrence>);

#[derive(Debug, Default)]
pub struct Expanded(RefCell<Option<Expansion>>);

impl Clone for Expanded {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for Expanded {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    pub href: String,
    pub etag: Option<String>,
    pub data: String,
    pub pending: Option<Pending>,
    #[serde(skip)]
    pub parsed: Option<CalendarObject>,
    #[serde(skip)]
    expanded: Expanded,
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
            expanded: Expanded::default(),
        }
    }

    fn reparse(&mut self) {
        self.set_parsed(parse_one(&self.data));
    }

    fn set_parsed(&mut self, parsed: Option<CalendarObject>) {
        self.parsed = parsed;
        self.expanded = Expanded::default();
    }

    fn occurrences(&self, from: Timestamp, to: Timestamp, local: &TimeZone) -> Vec<Occurrence> {
        let Some(object) = &self.parsed else {
            return Vec::new();
        };
        let Ok(mut memo) = self.expanded.0.try_borrow_mut() else {
            return object.occurrences(from, to, local);
        };
        if let Some((start, end, zone, found)) = memo.as_ref()
            && (*start, *end) == (from, to)
            && zone == local
        {
            return found.clone();
        }
        let found = object.occurrences(from, to, local);
        *memo = Some((from, to, local.clone(), found.clone()));
        found
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

    pub fn for_sync(&self) -> Self {
        Self {
            info: self.info.clone(),
            objects: self
                .objects
                .iter()
                .map(|stored| Stored {
                    href: stored.href.clone(),
                    etag: stored.etag.clone(),
                    data: match stored.pending {
                        Some(_) => stored.data.clone(),
                        None => String::new(),
                    },
                    pending: stored.pending,
                    parsed: None,
                    expanded: Expanded::default(),
                })
                .collect(),
        }
    }

    pub fn has_pending(&self) -> bool {
        self.objects.iter().any(|stored| stored.pending.is_some())
    }
}

#[derive(Deserialize)]
struct Head {
    info: CalendarInfo,
}

#[derive(Debug, Default)]
pub struct Summary {
    pub accounts: Vec<Account>,
    pub calendars: Vec<CalendarInfo>,
}

impl Summary {
    pub fn load(dir: &Path) -> Self {
        let accounts: Vec<Account> = std::fs::read_to_string(dir.join(ACCOUNTS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let mut calendars: Vec<CalendarInfo> = std::fs::read_dir(dir.join(CALENDARS_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str::<Head>(&text).ok())
            .map(|head| head.info)
            .filter(|info| accounts.iter().any(|account| account.id == info.account))
            .collect();
        calendars.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            accounts,
            calendars,
        }
    }

    pub fn writable(&self) -> impl Iterator<Item = (&CalendarInfo, Option<&str>)> {
        self.calendars
            .iter()
            .filter(|info| !info.read_only)
            .map(|info| {
                let account = self
                    .accounts
                    .iter()
                    .find(|account| account.id == info.account && account.id != LOCAL_ACCOUNT)
                    .map(|account| account.name.as_str());
                (info, account)
            })
    }

    pub fn automatic(&self) -> Option<usize> {
        self.writable()
            .position(|(info, _)| info.url.is_some())
            .or_else(|| self.writable().next().map(|_| 0))
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

fn read_or_set_aside<T: serde::de::DeserializeOwned>(file: &Path) -> Option<T> {
    let text = std::fs::read_to_string(file).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(error) => {
            let aside = file.with_extension("broken");
            tracing::warn!(
                "cannot read {}: {error}; keeping it as {}",
                file.display(),
                aside.display()
            );
            if let Err(error) = std::fs::rename(file, &aside) {
                tracing::warn!("cannot keep {}: {error}", aside.display());
            }
            None
        }
    }
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
        let accounts: Vec<Account> =
            read_or_set_aside(&dir.join(ACCOUNTS_FILE)).unwrap_or_default();
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
            .filter_map(|entry| read_or_set_aside::<Calendar>(&entry.path()))
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
        crate::donate::offer_to_search(&library.calendars);
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
                name: tr!("Calendar"),
                color: "#E8590C".to_owned(),
                read_only: false,
                visible: true,
                ctag: None,
                sync_token: None,
                refreshed: None,
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
            .and_then(|text| bande_xdg::replace_private_file(&dir.join(ACCOUNTS_FILE), &text));
        if let Err(error) = written {
            tracing::warn!("cannot save the accounts: {error}");
        }
    }

    pub fn save(&self, index: usize) {
        crate::donate::offer_to_search(&self.calendars);
        let (Some(dir), Some(calendar)) = (&self.dir, self.calendars.get(index)) else {
            return;
        };
        let file = calendar_file(dir, &calendar.info.id);
        let written = serde_json::to_string(calendar)
            .map_err(std::io::Error::other)
            .and_then(|text| bande_xdg::replace_private_file(&file, &text));
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

    pub fn add_calendar(&mut self, info: CalendarInfo) -> usize {
        self.calendars.push(Calendar::new(info));
        let index = self.calendars.len() - 1;
        self.save(index);
        index
    }

    pub fn rename_calendar(&mut self, index: usize, name: &str) {
        if let Some(calendar) = self.calendars.get_mut(index) {
            calendar.info.name = name.trim().to_owned();
            self.save(index);
        }
    }

    pub fn remove_calendar(&mut self, index: usize) {
        if index >= self.calendars.len() {
            return;
        }
        let calendar = self.calendars.remove(index);
        if let Some(dir) = &self.dir
            && let Err(error) = std::fs::remove_file(calendar_file(dir, &calendar.info.id))
        {
            tracing::warn!("cannot remove {}: {error}", calendar.info.name);
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

    pub fn set_subscription(&mut self, id: &str, refresh: Option<Option<u32>>, keep: Option<bool>) {
        let Some(AccountKind::Subscription {
            refresh_minutes,
            alerts,
            ..
        }) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .map(|account| &mut account.kind)
        else {
            return;
        };
        if let Some(refresh) = refresh {
            *refresh_minutes = refresh;
        }
        if let Some(keep) = keep.filter(|keep| keep != alerts) {
            *alerts = keep;
            for calendar in self
                .calendars
                .iter_mut()
                .filter(|calendar| calendar.info.account == id)
            {
                calendar.info.refreshed = None;
            }
        }
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
                found.extend(
                    stored
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
                stored.set_parsed(Some(object));
                index
            }
            None => {
                let href = href_for(&info, object.uid());
                let pending = (!local).then_some(Pending::Create);
                let mut stored = Stored::new(href, None, data, pending);
                stored.set_parsed(Some(object));
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
