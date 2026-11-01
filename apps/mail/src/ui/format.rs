use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::L10N;

pub fn list_date(seconds: i64, now: Timestamp, zone: &TimeZone) -> String {
    let Ok(time) = Timestamp::from_second(seconds) else {
        return String::new();
    };
    let (time, now) = (time.to_zoned(zone.clone()), now.to_zoned(zone.clone()));
    let (day, today) = (time.date(), now.date());
    if day == today {
        return time.strftime("%H:%M").to_string();
    }
    if today.yesterday().is_ok_and(|yesterday| yesterday == day) {
        return L10N.tr("date-yesterday");
    }
    let days = (today - day).get_days();
    if (0..7).contains(&days) {
        return time.strftime("%A").to_string();
    }
    time.strftime("%d/%m/%Y").to_string()
}

pub fn full_date(seconds: i64, zone: &TimeZone) -> String {
    Timestamp::from_second(seconds)
        .map(|time| {
            time.to_zoned(zone.clone())
                .strftime("%-d %B %Y at %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn initials(name: &str) -> String {
    if name.contains('@') {
        return name.chars().take(1).flat_map(char::to_uppercase).collect();
    }
    let words = name
        .split(|character: char| character.is_whitespace() || character == '@' || character == '.')
        .filter(|word| word.chars().next().is_some_and(char::is_alphabetic))
        .collect::<Vec<_>>();
    match words.as_slice() {
        [] => String::new(),
        [only] => only.chars().take(1).flat_map(char::to_uppercase).collect(),
        [first, .., last] => [first, last]
            .iter()
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect(),
    }
}
