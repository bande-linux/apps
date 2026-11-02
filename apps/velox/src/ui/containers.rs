use std::collections::{BTreeMap, HashMap};

use bande_toolkit::Palette;
use bande_toolkit::ui::{Cell, Column, Row};

use super::widgets::dot;
use crate::L10N;
use crate::engine::{Action, Container};

pub const PROJECT_PREFIX: &str = "project:";
const NAME_WIDTH: f32 = 220.0;
const IMAGE_WIDTH: f32 = 180.0;
const STATUS_WIDTH: f32 = 170.0;
const PORTS_WIDTH: f32 = 120.0;

pub fn columns() -> Vec<Column> {
    vec![
        Column::new(&L10N.tr("column-name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&L10N.tr("column-image"), IMAGE_WIDTH).unsortable(),
        Column::new(&L10N.tr("column-status"), STATUS_WIDTH).unsortable(),
        Column::new(&L10N.tr("column-ports"), PORTS_WIDTH).unsortable(),
    ]
}

pub fn pending_text(action: Action) -> String {
    L10N.tr(match action {
        Action::Start => "pending-start",
        Action::Stop => "pending-stop",
    })
}

fn state_dot(palette: &Palette, container: &Container, pending: Option<Action>) -> Cell {
    let icon = match (pending, container.state.as_str()) {
        (Some(_), _) | (None, "restarting") => dot(palette.warning, true),
        (None, "running") => dot(palette.success, true),
        (None, "paused") => dot(palette.ink3, true),
        _ => dot(palette.ink3, false),
    };
    Cell::new(container.name()).icon(icon)
}

fn ports(container: &Container) -> Cell {
    let ports = container.published_ports();
    match ports.is_empty() {
        true => Cell::new("—").muted(),
        false => Cell::new(
            ports
                .iter()
                .map(|port| format!(":{port}"))
                .collect::<Vec<_>>()
                .join(" "),
        )
        .tabular(),
    }
}

fn container_row(palette: &Palette, container: &Container, pending: Option<Action>) -> Row {
    let status = match pending {
        Some(action) => Cell::new(pending_text(action)).tinted(palette.warning),
        None => Cell::new(container.status.as_str()),
    };
    Row::new(
        container.id.clone(),
        vec![
            state_dot(palette, container, pending),
            Cell::new(container.image.as_str()).muted().cut_in_middle(),
            status,
            ports(container),
        ],
    )
}

fn project_row(
    palette: &Palette,
    project: &str,
    members: &[&Container],
    pending: &HashMap<String, Action>,
) -> Row {
    let running = members.iter().filter(|member| member.running()).count();
    let actions: Vec<Action> = members
        .iter()
        .filter_map(|member| pending.get(&member.id).copied())
        .collect();
    let shared = actions
        .first()
        .filter(|first| actions.iter().all(|action| action == *first));
    let icon = match (shared, running) {
        (Some(_), _) => dot(palette.warning, true),
        (None, 0) => dot(palette.ink3, false),
        (None, _) => dot(palette.success, true),
    };
    let status = match shared {
        Some(action) => Cell::new(pending_text(*action)).tinted(palette.warning),
        None => Cell::new(L10N.tr_args(
            "project-running",
            [("running", running.into()), ("total", members.len().into())],
        ))
        .muted(),
    };
    Row::new(
        format!("{PROJECT_PREFIX}{project}"),
        vec![
            Cell::new(project).icon(icon),
            Cell::new(""),
            status,
            Cell::new(""),
        ],
    )
    .children(
        members
            .iter()
            .map(|member| container_row(palette, member, pending.get(&member.id).copied()))
            .collect(),
    )
}

pub fn matches(container: &Container, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || [
            container.name(),
            container.image.as_str(),
            container.project().unwrap_or_default(),
        ]
        .iter()
        .any(|text| text.to_lowercase().contains(&filter))
}

pub fn rows(
    palette: &Palette,
    containers: &BTreeMap<String, Container>,
    pending: &HashMap<String, Action>,
    filter: &str,
) -> Vec<Row> {
    let mut projects: BTreeMap<&str, Vec<&Container>> = BTreeMap::new();
    let mut loose = Vec::new();
    for container in containers
        .values()
        .filter(|container| matches(container, filter))
    {
        match container.project() {
            Some(project) => projects.entry(project).or_default().push(container),
            None => loose.push(container),
        }
    }
    let mut named: Vec<(String, Row)> = projects
        .into_iter()
        .map(|(project, mut members)| {
            members.sort_by_key(|member| member.name().to_lowercase());
            (
                project.to_lowercase(),
                project_row(palette, project, &members, pending),
            )
        })
        .chain(loose.into_iter().map(|container| {
            (
                container.name().to_lowercase(),
                container_row(palette, container, pending.get(&container.id).copied()),
            )
        }))
        .collect();
    named.sort_by(|left, right| left.0.cmp(&right.0));
    named.into_iter().map(|(_, row)| row).collect()
}
