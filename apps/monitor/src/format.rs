use crate::L10N;

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1_000 {
        return L10N.tr_args("size-bytes", [("count", bytes.into())]);
    }
    let kilobytes = (bytes as f64 / 1e3).round();
    if kilobytes < 1e3 {
        return L10N.tr_args("size-kilobytes", [("value", kilobytes.to_string().into())]);
    }
    let units = [(1e6, 1), (1e9, 2), (1e12, 2)];
    let (unit, (scale, decimals)) = units
        .into_iter()
        .enumerate()
        .find(|(_, (scale, decimals))| {
            (bytes as f64 / scale * 10f64.powi(*decimals)).round() < 1e3 * 10f64.powi(*decimals)
        })
        .unwrap_or((2, units[2]));
    let value = format!("{:.*}", decimals as usize, bytes as f64 / scale);
    let value = value.trim_end_matches('0').trim_end_matches('.').to_owned();
    let args = [("value", value.into())];
    match unit {
        0 => L10N.tr_args("size-megabytes", args),
        1 => L10N.tr_args("size-gigabytes", args),
        _ => L10N.tr_args("size-terabytes", args),
    }
}

pub fn format_rate(bytes_per_second: f64) -> String {
    L10N.tr_args(
        "per-second",
        [("size", format_bytes(bytes_per_second.round() as u64).into())],
    )
}

pub fn format_percent(percent: f64) -> String {
    format!("{percent:.1}")
}

pub fn format_cpu_time(seconds: f64) -> String {
    let hundredths = (seconds * 100.0).round() as u64;
    let (hours, minutes) = (hundredths / 360_000, hundredths / 6_000 % 60);
    let seconds = hundredths % 6_000;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{:02}", seconds / 100)
    } else {
        format!("{minutes}:{:02}.{:02}", seconds / 100, seconds % 100)
    }
}
