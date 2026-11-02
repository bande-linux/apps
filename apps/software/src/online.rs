use bande_inbox::Outbox;
use zbus::blocking::{Connection, Proxy};

use crate::worker::Message;

const MANAGER: &str = "org.freedesktop.NetworkManager";
const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const CONNECTED_GLOBAL: u32 = 70;

fn follow(outbox: &Outbox<Message>) -> zbus::Result<()> {
    let bus = Connection::system()?;
    let manager = Proxy::new(&bus, MANAGER, MANAGER_PATH, MANAGER)?;
    let mut online = manager.get_property::<u32>("State")? == CONNECTED_GLOBAL;
    if !outbox.send(Message::Online(online)) {
        return Ok(());
    }
    for changed in manager.receive_signal("StateChanged")? {
        let now = changed.body().deserialize::<u32>()? == CONNECTED_GLOBAL;
        if now != online && !outbox.send(Message::Online(now)) {
            return Ok(());
        }
        online = now;
    }
    Ok(())
}

pub fn watch(outbox: Outbox<Message>) {
    let spawned = std::thread::Builder::new()
        .name("software-online".to_owned())
        .spawn(move || {
            if let Err(error) = follow(&outbox) {
                tracing::debug!("cannot follow the network: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot follow the network: {error}");
    }
}
