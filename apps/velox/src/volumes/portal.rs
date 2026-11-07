use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::path::PathBuf;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::L10N;

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SUCCESS: u32 = 0;

struct Filter<'a> {
    label: String,
    patterns: Vec<(u32, &'a str)>,
}

fn ask(
    method: &str,
    title: &str,
    filter: Option<Filter<'_>>,
    option: (&str, Value<'_>),
) -> zbus::Result<Option<PathBuf>> {
    let connection = match option_env!("VELOX_TEST_PORTAL_BUS") {
        Some(address) => zbus::blocking::connection::Builder::address(address)?.build()?,
        None => Connection::session()?,
    };
    let sender = connection
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let token = format!("bande_velox_{:x}", RandomState::new().hash_one(title));
    let handle = OwnedObjectPath::try_from(format!("{DESKTOP_PATH}/request/{sender}/{token}"))
        .map_err(zbus::Error::from)?;
    let request = Proxy::new(&connection, DESKTOP, handle, REQUEST)?;
    let mut responses = request.receive_signal("Response")?;
    let mut options = HashMap::from([("handle_token", Value::from(token.as_str())), option]);
    if let Some(filter) = &filter {
        options.insert(
            "filters",
            Value::from(vec![(filter.label.as_str(), filter.patterns.clone())]),
        );
    }
    connection.call_method(
        Some(DESKTOP),
        DESKTOP_PATH,
        Some(CHOOSER),
        method,
        &("", title, options),
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

fn tar_filter() -> Filter<'static> {
    Filter {
        label: L10N.tr("filter-tar"),
        patterns: vec![(0, "*.tar"), (0, "*.tar.gz"), (1, "application/x-tar")],
    }
}

pub fn choose_archive(title: &str) -> zbus::Result<Option<PathBuf>> {
    ask(
        "OpenFile",
        title,
        Some(tar_filter()),
        ("multiple", Value::from(false)),
    )
}

pub fn choose_save_path(title: &str, name: &str) -> zbus::Result<Option<PathBuf>> {
    ask(
        "SaveFile",
        title,
        Some(tar_filter()),
        ("current_name", Value::from(name)),
    )
}

pub fn choose_iso(title: &str) -> zbus::Result<Option<PathBuf>> {
    ask(
        "OpenFile",
        title,
        Some(Filter {
            label: L10N.tr("filter-iso"),
            patterns: vec![(0, "*.iso"), (1, "application/x-iso9660-image")],
        }),
        ("multiple", Value::from(false)),
    )
}

pub fn choose_folder(title: &str) -> zbus::Result<Option<PathBuf>> {
    ask("OpenFile", title, None, ("directory", Value::from(true)))
}
