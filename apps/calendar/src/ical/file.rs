use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rustix::fs::OFlags;

use super::event::PRODUCT;
use super::lines::{self, Component, Property, escape};

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub fn read_file(path: &Path) -> io::Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(OFlags::NONBLOCK.bits() as i32)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    Ok(text)
}

pub fn export<'a>(name: &str, objects: impl IntoIterator<Item = &'a str>) -> String {
    let mut calendar = Component::new("VCALENDAR");
    calendar.push(Property::new("VERSION", "2.0"));
    calendar.push(Property::new("PRODID", PRODUCT));
    calendar.push(Property::new("X-WR-CALNAME", escape(name)));
    let mut zones = HashSet::new();
    let roots = objects
        .into_iter()
        .filter_map(|data| lines::parse(data).ok())
        .flatten()
        .filter(|root| root.name == "VCALENDAR");
    for root in roots {
        for child in root.components {
            let keep = match child.name.as_str() {
                "VTIMEZONE" => zones.insert(child.value("TZID").unwrap_or_default().to_owned()),
                "VEVENT" | "VTODO" | "VJOURNAL" => true,
                _ => false,
            };
            if keep {
                calendar.components.push(child);
            }
        }
    }
    calendar.to_ics()
}
