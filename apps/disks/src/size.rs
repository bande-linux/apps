const UNITS: [(&str, f64); 9] = [
    ("tb", 1e12),
    ("gb", 1e9),
    ("mb", 1e6),
    ("kb", 1e3),
    ("tib", 1_099_511_627_776.0),
    ("gib", 1_073_741_824.0),
    ("mib", 1_048_576.0),
    ("kib", 1024.0),
    ("b", 1.0),
];

pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim().to_lowercase().replace(',', ".");
    let split = text
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number.parse().ok()?;
    let unit = unit.trim();
    let scale = match unit {
        "" => 1e9,
        unit => UNITS
            .iter()
            .find(|(name, _)| *name == unit || name.strip_suffix('b') == Some(unit))
            .map(|(_, scale)| *scale)?,
    };
    let bytes = number * scale;
    (bytes.is_finite() && bytes >= 0.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
}
