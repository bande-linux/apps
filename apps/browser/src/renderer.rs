use std::io;
use std::path::PathBuf;

pub(crate) use web_host::{
    Command, Host as Renderer, Link, Links, LoadId, Outcome, Session, Update, Verdict, ViewId,
};
use web_host::{Helpers, Program};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

const LIBEXEC: &str = "/usr/libexec/bande-browser";
const RENDERER: &str = "bande-browser-renderer";
const NETWORK: &str = "bande-browser-network";
const MEMORY_MAX: u64 = 512 << 20;

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

pub(crate) fn start() -> io::Result<Renderer> {
    let helpers = Helpers {
        renderer: helper(RENDERER)?,
        network: helper(NETWORK)?,
    };
    let mut bus = None;
    Renderer::new(
        helpers,
        Box::new(move |pid| {
            if bus.is_none() {
                bus = Connection::session().ok();
            }
            let placed = bus.as_ref().map(|bus| place(bus, pid));
            if let Some(Err(error)) = placed {
                tracing::warn!("cannot give renderer {pid} its memory limit: {error}");
            }
        }),
    )
}

fn place(bus: &Connection, pid: u32) -> zbus::Result<()> {
    let properties = vec![
        ("PIDs", Value::from(vec![pid])),
        ("Slice", Value::from("app.slice")),
        ("CollectMode", Value::from("inactive-or-failed")),
        ("MemoryMax", Value::from(MEMORY_MAX)),
        ("MemorySwapMax", Value::from(0u64)),
    ];
    bus.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StartTransientUnit",
        &(
            format!("app-bande-org.bande.Browser.Renderer-{pid}.scope"),
            "fail",
            properties,
            Vec::<(&str, Vec<(&str, Value<'_>)>)>::new(),
        ),
    )?;
    Ok(())
}
