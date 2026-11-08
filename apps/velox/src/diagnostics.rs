use std::fs;
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};

use crate::engine::{EngineInfo, Flavor};

const JOURNAL_LINES: &str = "40";

#[derive(Debug, Clone, Default)]
struct Engine {
    summary: String,
    journal: Option<(bool, &'static str)>,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

pub fn engine_connected(info: &EngineInfo, host: Option<&str>) {
    let (name, unit) = match info.flavor {
        Flavor::Docker => ("Docker", "docker.service"),
        Flavor::Podman => ("Podman", "podman.service"),
    };
    let rootless = match info.rootless {
        true => ", rootless",
        false => "",
    };
    let summary = format!(
        "Engine: {name} {} (API {}{rootless}) at {} on {}, {} CPUs, {} MiB",
        info.version,
        info.api,
        info.socket.display(),
        host.unwrap_or("this computer"),
        info.cpus,
        info.memory / (1024 * 1024),
    );
    let local = host.is_none() && info.started.is_some();
    *ENGINE.lock().unwrap_or_else(PoisonError::into_inner) = Some(Engine {
        summary,
        journal: local.then_some((info.rootless, unit)),
    });
}

pub fn engine_missing(reason: &str) {
    *ENGINE.lock().unwrap_or_else(PoisonError::into_inner) = Some(Engine {
        summary: format!("Engine: none ({reason})"),
        journal: None,
    });
}

fn os_name() -> String {
    fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn kernel() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|release| release.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn journal(user: bool, unit: &str) -> String {
    let mut command = Command::new("journalctl");
    if user {
        command.arg("--user");
    }
    command
        .args([
            "--unit",
            unit,
            "--no-pager",
            "--output=short-iso",
            "--lines",
            JOURNAL_LINES,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_else(|error| format!("cannot read the journal: {error}\n"))
}

pub fn diagnostics() -> String {
    let engine = ENGINE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| Engine {
            summary: "Engine: not connected".to_owned(),
            journal: None,
        });
    let mut text = format!(
        "Velox {}\n{}, kernel {}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        os_name(),
        kernel(),
        engine.summary,
    );
    if let Some((user, unit)) = engine.journal {
        text.push_str(&format!("\n-- {unit} (last {JOURNAL_LINES} lines) --\n"));
        text.push_str(&journal(user, unit));
    }
    text
}
