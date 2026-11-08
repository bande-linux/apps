use std::collections::{BTreeMap, HashMap, HashSet};

use bande_toolkit::Palette;
use bande_toolkit::ui::{Cell, Column, Row};

use super::stats::{Usage, percent};
use super::widgets::port_text;
use crate::L10N;
use crate::engine::{Action, Container};
use crate::resources::size;
use crate::ui::dot;

pub const PROJECT_PREFIX: &str = "project:";
const IMAGE_COLUMN: usize = 1;
const PORTS_COLUMN: usize = 3;
const NAME_WIDTH: f32 = 180.0;
const IMAGE_WIDTH: f32 = 130.0;
const STATUS_WIDTH: f32 = 140.0;
const PORTS_WIDTH: f32 = 96.0;
const CPU_WIDTH: f32 = 56.0;
const MEMORY_WIDTH: f32 = 70.0;

pub fn columns(compact: bool) -> Vec<Column> {
    let mut columns = vec![
        Column::new(&L10N.tr("column-name"), NAME_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&L10N.tr("column-image"), IMAGE_WIDTH).unsortable(),
        Column::new(&L10N.tr("column-status"), STATUS_WIDTH).unsortable(),
        Column::new(&L10N.tr("column-ports"), PORTS_WIDTH).unsortable(),
        Column::new(&L10N.tr("column-cpu"), CPU_WIDTH)
            .numeric()
            .unsortable(),
        Column::new(&L10N.tr("column-memory"), MEMORY_WIDTH)
            .numeric()
            .unsortable(),
    ];
    if compact {
        columns.remove(PORTS_COLUMN);
        columns.remove(IMAGE_COLUMN);
    }
    columns
}

pub fn ports_column(compact: bool) -> Option<usize> {
    (!compact).then_some(PORTS_COLUMN)
}

fn compacted(compact: bool, mut cells: Vec<Cell>) -> Vec<Cell> {
    if compact {
        cells.remove(PORTS_COLUMN);
        cells.remove(IMAGE_COLUMN);
    }
    cells
}

pub fn pending_text(action: Action) -> String {
    L10N.tr(match action {
        Action::Start => "pending-start",
        Action::Stop => "pending-stop",
        Action::Restart => "pending-restart",
        Action::Pause => "pending-pause",
        Action::Resume => "pending-resume",
        Action::Kill => "pending-kill",
        Action::Remove => "pending-remove",
    })
}

pub struct Shown<'a> {
    pub palette: &'a Palette,
    pub containers: &'a BTreeMap<String, Container>,
    pub pending: &'a HashMap<String, Action>,
    pub usage: &'a HashMap<String, Usage>,
    pub collapsed: &'a HashSet<String>,
    pub filter: &'a str,
    pub compact: bool,
}

#[derive(Debug, Default)]
pub struct Listing {
    pub rows: Vec<Row>,
    pub expanded: Vec<String>,
    pub order: Vec<String>,
    pub ports: Vec<Vec<u16>>,
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

fn status_cell(palette: &Palette, container: &Container) -> Cell {
    let cell = Cell::new(container.status.as_str());
    let unhealthy = container.status.contains("unhealthy");
    match container.state.as_str() {
        "running" if unhealthy => cell.tinted(palette.warning),
        "running" => cell.tinted(palette.success),
        "paused" | "restarting" => cell.tinted(palette.warning),
        "exited" | "dead" => cell.tinted(palette.danger_text),
        _ => cell.muted(),
    }
}

fn ports(palette: &Palette, ports: &[u16]) -> Cell {
    match ports.is_empty() {
        true => Cell::new("—").muted(),
        false => Cell::new(port_text(ports))
            .tabular()
            .tinted(palette.accent_text),
    }
}

impl Shown<'_> {
    fn usage_cells(&self, container: &Container) -> [Cell; 2] {
        match self
            .usage
            .get(&container.id)
            .and_then(Usage::latest)
            .filter(|_| container.running())
        {
            Some(sample) => [
                Cell::new(percent(sample.cpu)),
                Cell::new(size(sample.memory)),
            ],
            None => [Cell::new("—").muted(), Cell::new("—").muted()],
        }
    }

    fn container_row(&self, container: &Container) -> Row {
        let pending = self.pending.get(&container.id).copied();
        let status = match pending {
            Some(action) => Cell::new(pending_text(action)).tinted(self.palette.warning),
            None => status_cell(self.palette, container),
        };
        let [cpu, memory] = self.usage_cells(container);
        Row::new(
            container.id.clone(),
            compacted(
                self.compact,
                vec![
                    state_dot(self.palette, container, pending),
                    Cell::new(container.image.as_str()).muted().cut_in_middle(),
                    status,
                    ports(self.palette, &container.published_ports()),
                    cpu,
                    memory,
                ],
            ),
        )
    }

    fn project_row(&self, project: &str, members: &[&Container]) -> Row {
        let palette = self.palette;
        let running = members.iter().filter(|member| member.running()).count();
        let actions: Vec<Action> = members
            .iter()
            .filter_map(|member| self.pending.get(&member.id).copied())
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
            compacted(
                self.compact,
                vec![
                    Cell::new(project).icon(icon),
                    Cell::new(""),
                    status,
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                ],
            ),
        )
        .children(
            members
                .iter()
                .map(|member| self.container_row(member))
                .collect(),
        )
    }

    pub fn listing(&self) -> Listing {
        let mut projects: BTreeMap<&str, Vec<&Container>> = BTreeMap::new();
        let mut loose = Vec::new();
        for container in self
            .containers
            .values()
            .filter(|container| matches(container, self.filter))
        {
            match container.project() {
                Some(project) => projects.entry(project).or_default().push(container),
                None => loose.push(container),
            }
        }
        let mut entries: Vec<(String, Vec<&Container>, Option<&str>)> = projects
            .into_iter()
            .map(|(project, mut members)| {
                members.sort_by_key(|member| member.name().to_lowercase());
                (project.to_lowercase(), members, Some(project))
            })
            .chain(
                loose
                    .into_iter()
                    .map(|container| (container.name().to_lowercase(), vec![container], None)),
            )
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut listing = Listing::default();
        for (_, members, project) in entries {
            match project {
                Some(project) => {
                    let row = self.project_row(project, &members);
                    listing.order.push(row.key.clone());
                    listing.ports.push(Vec::new());
                    if !self.collapsed.contains(&row.key) {
                        listing.expanded.push(row.key.clone());
                        for member in &members {
                            listing.order.push(member.id.clone());
                            listing.ports.push(member.published_ports());
                        }
                    }
                    listing.rows.push(row);
                }
                None => {
                    for member in members {
                        listing.order.push(member.id.clone());
                        listing.ports.push(member.published_ports());
                        listing.rows.push(self.container_row(member));
                    }
                }
            }
        }
        listing
    }
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
