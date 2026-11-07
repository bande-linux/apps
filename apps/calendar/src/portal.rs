use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::path::PathBuf;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SUCCESS: u32 = 0;

pub enum Choose<'a> {
    Open,
    Save(&'a str),
}

pub fn choose_file(title: &str, choose: Choose) -> zbus::Result<Option<PathBuf>> {
    let connection = Connection::session()?;
    let sender = connection
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let token = format!("bande_calendar_{:x}", RandomState::new().hash_one(title));
    let handle = OwnedObjectPath::try_from(format!("{DESKTOP_PATH}/request/{sender}/{token}"))
        .map_err(zbus::Error::from)?;
    let request = Proxy::new(&connection, DESKTOP, handle, REQUEST)?;
    let mut responses = request.receive_signal("Response")?;
    let filter = vec![("iCalendar", vec![(0u32, "*.ics"), (1u32, "text/calendar")])];
    let mut options = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("filters", Value::from(filter)),
    ]);
    let method = match choose {
        Choose::Open => "OpenFile",
        Choose::Save(name) => {
            options.insert("current_name", Value::from(name));
            "SaveFile"
        }
    };
    connection.call_method(
        Some(DESKTOP),
        DESKTOP_PATH,
        Some(CHOOSER),
        method,
        &(bande_app::parent_window(), title, options),
    )?;
    let Some(response) = responses.next() else {
        return Ok(None);
    };
    let (code, results): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
    if code != SUCCESS {
        return Ok(None);
    }
    Ok(results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.try_clone().ok()?).ok())
        .and_then(|uris| {
            uris.first()
                .and_then(|uri| bande_xdg::path_from_file_uri(uri))
        }))
}
