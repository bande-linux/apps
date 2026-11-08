use std::time::{SystemTime, UNIX_EPOCH};

use bande_l10n::{Text, text, tr, trn};
use bande_toolkit::Palette;
use bande_toolkit::ui::{Cell, Column, FormLine, Label, Row, Widget};
use bande_vm::Accel;

use super::backend::{Act, Phase};
use super::fleet::{Entry, Fleet, Job, Power};
use crate::ui::dot;

pub const JOB_PREFIX: &str = "job:";
const NAME_WIDTH: f32 = 200.0;
const OS_WIDTH: f32 = 120.0;
const STATUS_WIDTH: f32 = 170.0;
const ADDRESS_WIDTH: f32 = 130.0;
const USAGE_WIDTH: f32 = 140.0;
const MIB_PER_GIB: u64 = 1024;
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

pub fn columns() -> Vec<Column> {
    vec![
        Column::new(&tr!("Name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&tr!("OS"), OS_WIDTH).unsortable(),
        Column::new(&tr!("Status"), STATUS_WIDTH).unsortable(),
        Column::new(&tr!("IP / SSH"), ADDRESS_WIDTH).unsortable(),
        Column::new(&tr!("CPU / MEM"), USAGE_WIDTH)
            .numeric()
            .unsortable(),
    ]
}

pub fn pending_text(act: Act) -> String {
    match act {
        Act::Start => tr!("Starting…"),
        Act::ShutDown => tr!("Shutting Down…"),
        Act::ForceStop => tr!("Stopping…"),
        Act::Restart => tr!("Restarting…"),
        Act::Delete => tr!("Deleting…"),
        Act::Pause => tr!("Pausing…"),
        Act::Resume => tr!("Resuming…"),
        Act::TakeSnapshot => tr!("Saving…"),
        Act::Revert => tr!("Reverting…"),
        Act::DeleteSnapshot => tr!("Deleting Snapshot…"),
        Act::Clone => tr!("Cloning…"),
        Act::Suspend => tr!("Suspending…"),
    }
}

fn uptime(since: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(since)
        .map_or(0, |elapsed| elapsed.as_secs());
    match seconds {
        seconds if seconds < MINUTE => tr!("just started"),
        seconds if seconds < HOUR => {
            trn!("1 minute", "{count} minutes", count = seconds / MINUTE)
        }
        seconds if seconds < DAY => trn!("1 hour", "{count} hours", count = seconds / HOUR),
        seconds => trn!("1 day", "{count} days", count = seconds / DAY),
    }
}

pub fn status_text(entry: &Entry) -> String {
    if let Some(act) = entry.pending {
        return pending_text(act);
    }
    match entry.power() {
        Power::Running => match entry.listed.since {
            Some(since) => tr!("Running · {uptime}", uptime = uptime(since)),
            None => tr!("Running"),
        },
        Power::Paused => tr!("Paused"),
        Power::Starting => tr!("Starting…"),
        Power::Stopping => tr!("Stopping…"),
        Power::Stopped if entry.listed.manifest.is_suspended() => tr!("Suspended"),
        Power::Stopped => tr!("Stopped"),
        Power::Failed => tr!("Failed"),
    }
}

pub fn status_cell(palette: &Palette, entry: &Entry) -> Cell {
    let text = status_text(entry);
    match (entry.pending, entry.power()) {
        (Some(_), _) | (None, Power::Starting | Power::Stopping) => {
            Cell::new(text).badge(palette.warning)
        }
        (None, Power::Failed) => Cell::new(text).badge(palette.danger_text),
        (None, Power::Stopped) => Cell::new(text).badge(palette.ink2),
        (None, Power::Paused) => Cell::new(text).badge(palette.warning),
        _ => Cell::new(text).badge(palette.success),
    }
}

fn state_dot(palette: &Palette, entry: &Entry) -> Cell {
    let icon = match (entry.pending, entry.power()) {
        (Some(_), _) | (None, Power::Starting | Power::Stopping) => dot(palette.warning, true),
        (None, Power::Running) => dot(palette.success, true),
        (None, Power::Paused) => dot(palette.ink3, true),
        (None, Power::Failed) => dot(palette.danger, false),
        (None, Power::Stopped) => dot(palette.ink3, false),
    };
    Cell::new(entry.name()).icon(icon)
}

pub fn short_ssh(user: &str, host: &str, port: u16) -> String {
    let port = match port {
        22 => String::new(),
        port => format!("-p {port} "),
    };
    match user.is_empty() {
        true => format!("ssh {port}{host}"),
        false => format!("ssh {port}{user}@{host}"),
    }
}

pub fn address(entry: &Entry) -> Option<String> {
    entry.ssh_target().map(|(host, port)| match port {
        22 => host,
        port => format!("{host}:{port}"),
    })
}

fn usage(entry: &Entry) -> Cell {
    match entry.usage {
        Some(usage) => Cell::new(format!(
            "{:.0} % · {}",
            usage.cpu,
            bande_fs::format_size(usage.memory)
        ))
        .tabular(),
        None => Cell::new("—").muted(),
    }
}

fn machine_row(palette: &Palette, entry: &Entry) -> Row {
    let address = match (entry.running(), address(entry)) {
        (true, Some(address)) => Cell::new(address).tabular(),
        _ => Cell::new("—").muted(),
    };
    Row::new(
        entry.id(),
        vec![
            state_dot(palette, entry),
            Cell::new(entry.listed.manifest.os.as_str()).muted(),
            status_cell(palette, entry),
            address,
            usage(entry),
        ],
    )
}

pub fn job_text(job: &Job) -> String {
    match job.phase {
        Phase::Downloading(Some(fraction)) => tr!(
            "Downloading… {percent} %",
            percent = ((fraction * 100.0).round() as u32)
        ),
        Phase::Downloading(None) => tr!("Downloading…"),
        Phase::Creating => tr!("Creating…"),
    }
}

fn job_row(palette: &Palette, key: u64, job: &Job) -> Row {
    Row::new(
        format!("{JOB_PREFIX}{key}"),
        vec![
            Cell::new(job.name.as_str()).icon(dot(palette.warning, false)),
            Cell::new(job.os.as_str()).muted(),
            Cell::new(job_text(job)).badge(palette.warning),
            Cell::new(""),
            Cell::new(""),
        ],
    )
}

pub fn matches(entry: &Entry, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || [entry.name(), entry.listed.manifest.os.as_str()]
            .iter()
            .any(|text| text.to_lowercase().contains(&filter))
}

pub fn rows(palette: &Palette, fleet: &Fleet, filter: &str) -> Vec<Row> {
    let mut entries: Vec<&Entry> = fleet
        .entries
        .values()
        .filter(|entry| matches(entry, filter))
        .collect();
    entries.sort_by_key(|entry| entry.name().to_lowercase());
    entries
        .into_iter()
        .map(|entry| machine_row(palette, entry))
        .chain(
            fleet
                .jobs
                .iter()
                .map(|(key, job)| job_row(palette, *key, job)),
        )
        .collect()
}

pub fn gigabytes(count: u64) -> String {
    tr!("{count} GB", count = count)
}

fn date(seconds: u64) -> String {
    let created = UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    let days = SystemTime::now()
        .duration_since(created)
        .map_or(0, |elapsed| elapsed.as_secs() / DAY);
    match days {
        0 => tr!("Today"),
        days => trn!(
            context = "days ago",
            "Yesterday",
            "{count} days ago",
            count = days
        ),
    }
}

fn value(text: impl AsRef<str>) -> Box<dyn Widget> {
    Box::new(Label::new(text.as_ref()))
}

pub fn info_lines(entry: &Entry, accel: Option<Accel>) -> Vec<FormLine> {
    let manifest = &entry.listed.manifest;
    let row = |key: Text, text: String| FormLine::Row(key.tr(), value(text));
    let mut lines = vec![
        FormLine::Heading(tr!("Machine")),
        row(text!("OS"), manifest.os.clone()),
        row(text!("Status"), status_text(entry)),
        row(text!("CPUs"), manifest.cpus.to_string()),
        row(
            text!("Memory"),
            gigabytes(manifest.memory_mib / MIB_PER_GIB),
        ),
        row(text!("Created"), date(manifest.created)),
        row(text!("Snapshots"), manifest.snapshots.len().to_string()),
    ];
    if let Some(accel) = accel {
        lines.push(row(
            text!("Acceleration"),
            match accel {
                Accel::Kvm => tr!("KVM"),
                Accel::Tcg => tr!("Emulated"),
            },
        ));
    }
    lines.push(FormLine::Heading(tr!("Disk")));
    lines.push(row(text!("Disk"), gigabytes(manifest.disk_gib)));
    let used = std::fs::metadata(entry.listed.dir.join("disk.qcow2"))
        .map(|meta| std::os::unix::fs::MetadataExt::blocks(&meta) * 512)
        .unwrap_or_default();
    lines.push(row(text!("Used on disk"), bande_fs::format_size(used)));
    lines.push(FormLine::Heading(tr!("Network")));
    for forward in &manifest.forwards {
        lines.push(row(
            text!("Forward"),
            tr!(
                "localhost:{host} → {guest}",
                host = forward.host,
                guest = forward.guest
            ),
        ));
    }
    if let Some(address) = entry.address {
        lines.push(row(text!("IP address"), address.to_string()));
    }
    if let Some((host, port)) = entry.ssh_target() {
        lines.push(row(
            text!("SSH"),
            short_ssh(&entry.listed.login.user, &host, port),
        ));
    }
    if let Some(usage) = entry.usage {
        lines.push(FormLine::Heading(tr!("Usage")));
        lines.push(row(text!("CPU"), format!("{:.0} %", usage.cpu)));
        lines.push(row(text!("Memory"), bande_fs::format_size(usage.memory)));
    }
    lines
}
