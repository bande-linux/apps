use zbus::blocking::Connection;

pub type Inhibitor = (String, String, String, String, u32, u32);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn list_inhibitors(&self) -> zbus::Result<Vec<Inhibitor>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SleepBlocker {
    pub pid: u32,
    pub who: String,
    pub why: String,
}

pub fn sleep_blockers(inhibitors: Vec<Inhibitor>) -> Vec<SleepBlocker> {
    inhibitors
        .into_iter()
        .filter(|(what, _, _, mode, _, _)| {
            mode == "block" && what.split(':').any(|kind| matches!(kind, "sleep" | "idle"))
        })
        .map(|(_, who, why, _, _, pid)| SleepBlocker { pid, who, why })
        .collect()
}

pub fn list_sleep_blockers(bus: &Connection) -> zbus::Result<Vec<SleepBlocker>> {
    Ok(sleep_blockers(
        ManagerProxyBlocking::new(bus)?.list_inhibitors()?,
    ))
}
