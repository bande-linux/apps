use zbus::blocking::{Connection, Proxy};
use zbus::proxy::MethodFlags;
use zbus::zvariant::OwnedObjectPath;

use super::Flavor;

const SYSTEMD: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const DENIED: [&str; 2] = [
    "org.freedesktop.DBus.Error.AccessDenied",
    "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired",
];
const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifecycle {
    pub flavor: Flavor,
    pub user: bool,
    pub start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Controlled {
    Done,
    Refused,
    Failed(String),
}

impl Lifecycle {
    fn units(self) -> &'static [&'static str] {
        match (self.flavor, self.start) {
            (Flavor::Docker, true) => &["docker.service"],
            (Flavor::Docker, false) => &["docker.socket", "docker.service"],
            (Flavor::Podman, true) => &["podman.socket"],
            (Flavor::Podman, false) => &["podman.socket", "podman.service"],
        }
    }

    pub fn run(self) -> Controlled {
        let connection = match self.user {
            true => Connection::session(),
            false => Connection::system(),
        };
        let result = connection.and_then(|connection| {
            let manager = Proxy::new(&connection, SYSTEMD, MANAGER_PATH, MANAGER)?;
            manager.call_method("Subscribe", &())?;
            let mut done = Ok(Controlled::Done);
            for unit in self.units() {
                done = job(&manager, self.start, unit);
                if done != Ok(Controlled::Done) {
                    break;
                }
            }
            let _ = manager.call_method("Unsubscribe", &());
            done
        });
        match result {
            Ok(controlled) => controlled,
            Err(zbus::Error::MethodError(name, _, _)) if DENIED.contains(&name.as_str()) => {
                Controlled::Refused
            }
            Err(error) => Controlled::Failed(error.to_string()),
        }
    }
}

fn job(manager: &Proxy<'_>, start: bool, unit: &str) -> zbus::Result<Controlled> {
    let method = match start {
        true => "StartUnit",
        false => "StopUnit",
    };
    let removed = manager.receive_signal("JobRemoved")?;
    let body = (unit, "replace");
    let queued: zbus::Result<Option<OwnedObjectPath>> =
        match manager.call_with_flags(method, Default::default(), &body) {
            Err(zbus::Error::MethodError(name, _, _)) if DENIED.contains(&name.as_str()) => {
                manager.call_with_flags(method, MethodFlags::AllowInteractiveAuth.into(), &body)
            }
            other => other,
        };
    let job = match queued {
        Ok(Some(job)) => job,
        Ok(None) => return Ok(Controlled::Done),
        Err(zbus::Error::MethodError(name, _, _)) if !start && name.as_str() == NO_SUCH_UNIT => {
            return Ok(Controlled::Done);
        }
        Err(error) => return Err(error),
    };
    for signal in removed {
        let Ok((_, path, _, result)) = signal
            .body()
            .deserialize::<(u32, OwnedObjectPath, String, String)>()
        else {
            continue;
        };
        if path == job {
            return Ok(match result.as_str() {
                "done" => Controlled::Done,
                result => Controlled::Failed(format!("{unit}: {result}")),
            });
        }
    }
    Ok(Controlled::Done)
}
