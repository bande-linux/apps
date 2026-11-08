use std::time::{SystemTime, UNIX_EPOCH};

use bande_toolkit::Palette;
use bande_toolkit::ui::{Cell, Column, FormLine, Label, Row, Widget};
use bande_vm::Accel;

use super::backend::{Act, Phase};
use super::fleet::{Entry, Fleet, Job, Power};
use crate::L10N;
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
        Column::new(&L10N.tr("column-name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&L10N.tr("machine-column-os"), OS_WIDTH).unsortable(),
        Column::new(&L10N.tr("column-status"), STATUS_WIDTH).unsortable(),
        Column::new(&L10N.tr("machine-column-address"), ADDRESS_WIDTH).unsortable(),
        Column::new(&L10N.tr("machine-column-usage"), USAGE_WIDTH)
            .numeric()
            .unsortable(),
    ]
}

pub fn pending_text(act: Act) -> String {
    L10N.tr(match act {
        Act::Start => "pending-start",
        Act::ShutDown => "machine-pending-shut-down",
        Act::ForceStop => "pending-stop",
        Act::Restart => "machine-pending-restart",
        Act::Delete => "machine-pending-delete",
        Act::Pause => "machine-pending-pause",
        Act::Resume => "machine-pending-resume",
        Act::TakeSnapshot => "machine-pending-snapshot",
        Act::Revert => "machine-pending-revert",
        Act::DeleteSnapshot => "machine-pending-delete-snapshot",
        Act::Clone => "machine-pending-clone",
        Act::Suspend => "machine-pending-suspend",
    })
}

fn uptime(since: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(since)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (key, count) = match seconds {
        seconds if seconds < MINUTE => ("machine-uptime-seconds", 0),
        seconds if seconds < HOUR => ("machine-uptime-minutes", seconds / MINUTE),
        seconds if seconds < DAY => ("machine-uptime-hours", seconds / HOUR),
        seconds => ("machine-uptime-days", seconds / DAY),
    };
    L10N.tr_args(key, [("count", count.into())])
}

pub fn status_text(entry: &Entry) -> String {
    if let Some(act) = entry.pending {
        return pending_text(act);
    }
    match entry.power() {
        Power::Running => match entry.listed.since {
            Some(since) => L10N.tr_args("machine-running-for", [("uptime", uptime(since).into())]),
            None => L10N.tr("machine-running"),
        },
        Power::Paused => L10N.tr("machine-paused"),
        Power::Starting => L10N.tr("pending-start"),
        Power::Stopping => L10N.tr("pending-stop"),
        Power::Stopped if entry.listed.manifest.is_suspended() => L10N.tr("machine-suspended"),
        Power::Stopped => L10N.tr("machine-stopped"),
        Power::Failed => L10N.tr("machine-failed"),
    }
}

pub fn status_cell(palette: &Palette, entry: &Entry) -> Cell {
    let text = status_text(entry);
    match (entry.pending, entry.power()) {
        (Some(_), _) | (None, Power::Starting | Power::Stopping) => {
            Cell::new(text).tinted(palette.warning)
        }
        (None, Power::Failed) => Cell::new(text).tinted(palette.danger),
        (None, Power::Stopped) => Cell::new(text).muted(),
        _ => Cell::new(text),
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

pub fn short_ssh(user: &str, port: u16) -> String {
    match user.is_empty() {
        true => format!("ssh -p {port} localhost"),
        false => format!("ssh -p {port} {user}@localhost"),
    }
}

pub fn address(entry: &Entry) -> Option<String> {
    entry.ssh_port().map(|port| format!("localhost:{port}"))
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
        Phase::Downloading(Some(fraction)) => L10N.tr_args(
            "machine-downloading-percent",
            [("percent", ((fraction * 100.0).round() as u32).into())],
        ),
        Phase::Downloading(None) => L10N.tr("machine-downloading"),
        Phase::Creating => L10N.tr("machine-creating"),
    }
}

fn job_row(palette: &Palette, key: u64, job: &Job) -> Row {
    Row::new(
        format!("{JOB_PREFIX}{key}"),
        vec![
            Cell::new(job.name.as_str()).icon(dot(palette.warning, false)),
            Cell::new(job.os.as_str()).muted(),
            Cell::new(job_text(job)).tinted(palette.warning),
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
    L10N.tr_args("machine-gigabytes", [("count", count.into())])
}

fn date(seconds: u64) -> String {
    let created = UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    let days = SystemTime::now()
        .duration_since(created)
        .map_or(0, |elapsed| elapsed.as_secs() / DAY);
    match days {
        0 => L10N.tr("machine-created-today"),
        days => L10N.tr_args("machine-created-days", [("count", days.into())]),
    }
}

fn value(text: impl AsRef<str>) -> Box<dyn Widget> {
    Box::new(Label::new(text.as_ref()))
}

pub fn info_lines(entry: &Entry, accel: Option<Accel>) -> Vec<FormLine> {
    let manifest = &entry.listed.manifest;
    let row = |key: &str, text: String| FormLine::Row(L10N.tr(key), value(text));
    let mut lines = vec![
        FormLine::Heading(L10N.tr("machine-section-machine")),
        row("machine-os", manifest.os.clone()),
        row("column-status", status_text(entry)),
        row("machine-cpus", manifest.cpus.to_string()),
        row(
            "machine-memory",
            gigabytes(manifest.memory_mib / MIB_PER_GIB),
        ),
        row("machine-created", date(manifest.created)),
        row(
            "machine-snapshot-count",
            manifest.snapshots.len().to_string(),
        ),
    ];
    if let Some(accel) = accel {
        lines.push(row(
            "machine-acceleration",
            L10N.tr(match accel {
                Accel::Kvm => "machine-kvm",
                Accel::Tcg => "machine-emulated",
            }),
        ));
    }
    lines.push(FormLine::Heading(L10N.tr("machine-section-disk")));
    lines.push(row("machine-disk-size", gigabytes(manifest.disk_gib)));
    let used = std::fs::metadata(entry.listed.dir.join("disk.qcow2"))
        .map(|meta| std::os::unix::fs::MetadataExt::blocks(&meta) * 512)
        .unwrap_or_default();
    lines.push(row("machine-disk-used", bande_fs::format_size(used)));
    lines.push(FormLine::Heading(L10N.tr("machine-section-network")));
    for forward in &manifest.forwards {
        lines.push(row(
            "machine-forward",
            L10N.tr_args(
                "machine-forward-value",
                [
                    ("host", forward.host.into()),
                    ("guest", forward.guest.into()),
                ],
            ),
        ));
    }
    if let Some(address) = entry.address {
        lines.push(row("machine-ip", address.to_string()));
    }
    if let Some(port) = entry.ssh_port() {
        lines.push(row(
            "machine-ssh",
            short_ssh(&entry.listed.login.user, port),
        ));
    }
    if let Some(usage) = entry.usage {
        lines.push(FormLine::Heading(L10N.tr("machine-section-usage")));
        lines.push(row("machine-cpu-usage", format!("{:.0} %", usage.cpu)));
        lines.push(row(
            "machine-memory-usage",
            bande_fs::format_size(usage.memory),
        ));
    }
    lines
}
