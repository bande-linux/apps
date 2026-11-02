use bande_datetime::timedate::zone_changes;
use bande_inbox::Outbox;
use zbus::blocking::{Connection, Proxy};

const MANAGER: &str = "org.freedesktop.NetworkManager";
const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const CONNECTED_GLOBAL: u32 = 70;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum System {
    Online,
    Zone(String),
}

fn follow_network(outbox: &Outbox<System>) -> zbus::Result<()> {
    let bus = Connection::system()?;
    let manager = Proxy::new(&bus, MANAGER, MANAGER_PATH, MANAGER)?;
    let mut online = manager.get_property::<u32>("State")? == CONNECTED_GLOBAL;
    for changed in manager.receive_signal("StateChanged")? {
        let now = changed.body().deserialize::<u32>()? == CONNECTED_GLOBAL;
        if now && !online && !outbox.send(System::Online) {
            break;
        }
        online = now;
    }
    Ok(())
}

fn follow_zone(outbox: &Outbox<System>) -> zbus::Result<()> {
    for zone in zone_changes(&Connection::system()?)? {
        if !outbox.send(System::Zone(zone)) {
            break;
        }
    }
    Ok(())
}

fn spawn(name: &str, work: impl FnOnce() -> zbus::Result<()> + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            if let Err(error) = work() {
                tracing::debug!("cannot follow the system: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot follow the system: {error}");
    }
}

pub fn watch(outbox: Outbox<System>) {
    let zones = outbox.clone();
    spawn("zone", move || follow_zone(&zones));
    spawn("online", move || follow_network(&outbox));
}
