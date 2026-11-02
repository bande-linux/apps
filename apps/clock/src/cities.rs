use std::collections::HashMap;

use bande_datetime::daylight::ZONE_TAB;

const COUNTRIES: &str = "/usr/share/zoneinfo/iso3166.tab";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct City {
    pub zone: String,
    pub name: String,
    pub country: String,
}

pub fn city_name(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

fn rows(file: &str) -> Vec<Vec<String>> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

pub fn all() -> Vec<City> {
    let countries: HashMap<String, String> = rows(COUNTRIES)
        .into_iter()
        .filter_map(|row| Some((row.first()?.clone(), row.get(1)?.clone())))
        .collect();
    let mut cities: Vec<City> = rows(ZONE_TAB)
        .into_iter()
        .filter_map(|row| {
            let zone = row.get(2)?.clone();
            let country = row
                .first()
                .and_then(|code| countries.get(code))
                .cloned()
                .unwrap_or_default();
            Some(City {
                name: city_name(&zone),
                zone,
                country,
            })
        })
        .collect();
    cities.sort_by(|a, b| a.name.cmp(&b.name));
    cities
}

pub fn matching<'a>(cities: &'a [City], text: &str) -> impl Iterator<Item = &'a City> {
    let text = text.trim().to_lowercase();
    cities.iter().filter(move |city| {
        text.is_empty()
            || city.name.to_lowercase().contains(&text)
            || city.country.to_lowercase().contains(&text)
            || city.zone.to_lowercase().contains(&text)
    })
}
