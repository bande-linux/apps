use std::io;
use std::path::PathBuf;

pub(crate) use web_host::{
    Command, Host as Renderer, Link, Links, LoadId, Outcome, Session, Update, Verdict, ViewId,
};
use web_host::{Helpers, Program};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, Value};

const LIBEXEC: &str = "/usr/libexec/bande-browser";
const RENDERER: &str = "bande-browser-renderer";
const NETWORK: &str = "bande-browser-network";
const MEMORY_MAX: u64 = 512 << 20;
const UNRESPONSIVE_AFTER: std::time::Duration = std::time::Duration::from_secs(15);
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";

fn helper(name: &str) -> io::Result<Program> {
    let installed = PathBuf::from(LIBEXEC).join(name);
    if installed.exists() {
        return Ok(Program {
            path: installed,
            arguments: Vec::new(),
        });
    }
    Program::beside_executable(name)
}

fn storage() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(data.join(crate::INFO.id))
}

pub(crate) fn start() -> io::Result<Renderer> {
    let mut network = helper(NETWORK)?;
    if let Some(storage) = storage() {
        network.arguments = vec![
            web_netd::STORAGE_FLAG.to_owned(),
            storage.to_string_lossy().into_owned(),
        ];
    }
    let helpers = Helpers {
        renderer: helper(RENDERER)?,
        network,
        unresponsive_after: UNRESPONSIVE_AFTER,
    };
    let mut bus = None;
    Renderer::new(
        helpers,
        Box::new(move |pid| {
            if bus.is_none() {
                bus = Connection::session()
                    .and_then(|connection| {
                        let manager = Proxy::new(&connection, SYSTEMD, SYSTEMD_PATH, MANAGER)?;
                        manager.call_method("Subscribe", &())?;
                        Ok(connection)
                    })
                    .ok();
            }
            let placed = bus.as_ref().map(|bus| place(bus, pid));
            if let Some(Err(error)) = placed {
                tracing::warn!("cannot give renderer {pid} its memory limit: {error}");
            }
        }),
    )
}

fn place(bus: &Connection, pid: u32) -> zbus::Result<()> {
    let manager = Proxy::new(bus, SYSTEMD, SYSTEMD_PATH, MANAGER)?;
    let removed = manager.receive_signal("JobRemoved")?;
    let properties = vec![
        ("PIDs", Value::from(vec![pid])),
        ("Slice", Value::from("app.slice")),
        ("CollectMode", Value::from("inactive-or-failed")),
        ("MemoryMax", Value::from(MEMORY_MAX)),
        ("MemorySwapMax", Value::from(0u64)),
    ];
    let job: OwnedObjectPath = manager.call(
        "StartTransientUnit",
        &(
            format!("app-bande-org.bande.Browser.Renderer-{pid}.scope"),
            "fail",
            properties,
            Vec::<(&str, Vec<(&str, Value<'_>)>)>::new(),
        ),
    )?;
    for signal in removed {
        let (_, path, _, result): (u32, OwnedObjectPath, String, String) =
            signal.body().deserialize()?;
        if path == job {
            return match result.as_str() {
                "done" => Ok(()),
                _ => Err(zbus::Error::Failure(format!(
                    "the scope job ended with {result}"
                ))),
            };
        }
    }
    Err(zbus::Error::Failure(
        "the bus closed before the scope started".into(),
    ))
}
