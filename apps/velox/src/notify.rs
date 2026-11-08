use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::L10N;

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const DEFAULT_TIMEOUT: i32 = -1;
const OPEN: &str = "default";

fn send(app_id: &str, title: &str, body: &str) -> zbus::Result<()> {
    let hints: HashMap<&str, Value> = HashMap::from([
        ("desktop-entry", Value::from(app_id)),
        ("category", Value::from("x-velox.crash")),
    ]);
    Connection::session()?.call_method(
        Some(NOTIFICATIONS),
        NOTIFICATIONS_PATH,
        Some(NOTIFICATIONS),
        "Notify",
        &(
            L10N.tr("app-name"),
            0u32,
            app_id,
            title,
            body,
            vec![OPEN, ""],
            hints,
            DEFAULT_TIMEOUT,
        ),
    )?;
    Ok(())
}

pub fn notify_crash(app_id: &'static str, name: &str, code: i64) {
    let title = L10N.tr_args("crash-title", [("name", name.to_owned().into())]);
    let body = L10N.tr_args("crash-body", [("code", code.into())]);
    let sent = std::thread::Builder::new()
        .name("crash notice".to_owned())
        .spawn(move || {
            if let Err(error) = send(app_id, &title, &body) {
                tracing::warn!("cannot show the crash notice: {error}");
            }
        });
    if let Err(error) = sent {
        tracing::warn!("cannot show the crash notice: {error}");
    }
}
