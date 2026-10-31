use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use unbound_status::devices::battery;
use unbound_status::history::{ChargeSample, charge_history};
use unbound_status::status::Battery;
use zbus::blocking::Connection;

pub const BATTERY_HISTORY: u32 = 12 * 3600;
const HISTORY_POINTS: u32 = 72;
const HISTORY_REFRESH: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatteryReport {
    pub battery: Option<Battery>,
    pub history: Vec<ChargeSample>,
    pub now: u32,
}

fn unix_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u32::try_from(since.as_secs()).ok())
        .unwrap_or(0)
}

pub fn battery_reader(system: Connection) -> impl FnMut() -> Option<Option<BatteryReport>> {
    let mut history = Vec::new();
    let mut read_at: Option<Instant> = None;
    move || {
        if read_at.is_none_or(|at| at.elapsed() >= HISTORY_REFRESH) {
            history = charge_history(&system, BATTERY_HISTORY, HISTORY_POINTS)
                .inspect_err(|error| tracing::debug!("no battery history: {error}"))
                .unwrap_or_default();
            read_at = Some(Instant::now());
        }
        Some(Some(BatteryReport {
            battery: battery(&system),
            history: history.clone(),
            now: unix_now(),
        }))
    }
}
