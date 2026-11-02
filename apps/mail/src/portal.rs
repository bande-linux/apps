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

fn ask(method: &str, title: &str, option: (&str, Value<'_>)) -> zbus::Result<Vec<PathBuf>> {
    let connection = Connection::session()?;
    let sender = connection
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let token = format!("bande_mail_{:x}", RandomState::new().hash_one(title));
    let handle = OwnedObjectPath::try_from(format!("{DESKTOP_PATH}/request/{sender}/{token}"))
        .map_err(zbus::Error::from)?;
    let request = Proxy::new(&connection, DESKTOP, handle, REQUEST)?;
    let mut responses = request.receive_signal("Response")?;
    let options = HashMap::from([("handle_token", Value::from(token.as_str())), option]);
    connection.call_method(
        Some(DESKTOP),
        DESKTOP_PATH,
        Some(CHOOSER),
        method,
        &("", title, options),
    )?;
    let Some(response) = responses.next() else {
        return Ok(Vec::new());
    };
    let (code, results): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
    if code != SUCCESS {
        return Ok(Vec::new());
    }
    let uris = results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.try_clone().ok()?).ok())
        .unwrap_or_default();
    Ok(uris
        .iter()
        .filter_map(|uri| bande_xdg::path_from_file_uri(uri))
        .collect())
}

pub fn choose_files(title: &str) -> zbus::Result<Vec<PathBuf>> {
    ask("OpenFile", title, ("multiple", Value::from(true)))
}

pub fn choose_save_path(title: &str, name: &str) -> zbus::Result<Option<PathBuf>> {
    Ok(ask("SaveFile", title, ("current_name", Value::from(name)))?
        .into_iter()
        .next())
}
