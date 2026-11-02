use std::rc::Rc;

use bande_toolkit::TextRole;
use bande_toolkit::ui::{
    Axis, Button, Graph, GraphKind, ScrollView, Series, Stack, Ui, Widget, WidgetId,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::inspect::Inspect;
use super::menu::url;
use super::pane::{Intent, Intents, push};
use super::stats::{Graphs, HISTORY, Usage};
use super::widgets::Text;
use crate::L10N;
use crate::engine::Container;

pub const WIDTH: f32 = 280.0;
const SPACING: f32 = 10.0;
const PADDING: f32 = 16.0;
const FACT_GAP: f32 = 2.0;
const CRASH_LOOP: u32 = 3;

pub enum Subject<'a> {
    Nothing,
    Many(usize),
    Project {
        name: &'a str,
        running: usize,
        total: usize,
        directory: Option<&'a str>,
    },
    Container {
        container: &'a Container,
        inspect: Option<&'a Inspect>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphIds {
    cpu: WidgetId,
    memory: WidgetId,
    network: WidgetId,
    disk: WidgetId,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn section(title: &str) -> Box<dyn Widget> {
    boxed(Text::new(title, TextRole::Label).tertiary())
}

fn fact(key: &str, value: &str) -> Box<dyn Widget> {
    boxed(Stack::new(
        Axis::Vertical,
        FACT_GAP,
        0.0,
        vec![
            boxed(Text::new(key, TextRole::Callout).secondary()),
            boxed(Text::new(value, TextRole::Body)),
        ],
    ))
}

fn note(text: &str) -> Box<dyn Widget> {
    boxed(Text::new(text, TextRole::Callout).secondary())
}

fn when(stamp: &str) -> Option<String> {
    let stamp: Timestamp = stamp.parse().ok()?;
    (stamp.as_second() > 0).then(|| {
        stamp
            .to_zoned(TimeZone::system())
            .strftime("%-d %b %Y, %H:%M")
            .to_string()
    })
}

fn state_text(inspect: &Inspect) -> String {
    let state = &inspect.state;
    let mut text = match state.status.as_str() {
        "exited" => L10N.tr_args("state-exited", [("code", state.exit_code.into())]),
        status => {
            let mut chars = status.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    };
    if state.oom_killed {
        text = L10N.tr_args("state-oom", [("state", text.into())]);
    }
    text
}

fn container_lines(inspect: &Inspect, intents: &Intents) -> Vec<Box<dyn Widget>> {
    let mut lines = vec![section(&L10N.tr("inspector-container"))];
    lines.push(fact(&L10N.tr("inspector-image"), &inspect.config.image));
    lines.push(fact(&L10N.tr("inspector-state"), &state_text(inspect)));
    if !inspect.state.error.is_empty() {
        lines.push(note(&inspect.state.error));
    }
    if let Some(created) = when(&inspect.created) {
        lines.push(fact(&L10N.tr("inspector-created"), &created));
    }
    lines.push(fact(
        &L10N.tr("inspector-restarts"),
        &inspect.restart_count.to_string(),
    ));
    if inspect.state.restarting || inspect.restart_count >= CRASH_LOOP {
        lines.push(note(&L10N.tr("inspector-crash-loop")));
    }
    lines.push(section(&L10N.tr("inspector-process")));
    lines.push(fact(&L10N.tr("inspector-command"), &inspect.command()));
    if !inspect.config.working_dir.is_empty() {
        lines.push(fact(
            &L10N.tr("inspector-workdir"),
            &inspect.config.working_dir,
        ));
    }
    let environment = inspect.config.env.as_deref().unwrap_or_default();
    lines.push(section(&L10N.tr_args(
        "inspector-environment",
        [("count", environment.len().into())],
    )));
    lines.extend(environment.iter().map(|variable| note(variable)));
    lines.push(section(&L10N.tr("inspector-network")));
    if let Some(ip) = inspect.ip() {
        lines.push(fact(&L10N.tr("inspector-ip"), ip));
    }
    let ports = inspect.ports();
    if ports.is_empty() {
        lines.push(note(&L10N.tr("inspector-no-ports")));
    }
    for (host, inside) in ports {
        let Ok(port) = host.parse::<u16>() else {
            continue;
        };
        let open = Rc::clone(intents);
        let title = format!(":{host} → {inside}");
        lines.push(boxed(Stack::new(
            Axis::Horizontal,
            0.0,
            0.0,
            vec![boxed(
                Button::new(&title, move || push(&open, Intent::Open(url(port)))).link(),
            )],
        )));
    }
    lines.push(section(&L10N.tr_args(
        "inspector-mounts",
        [("count", inspect.mounts.len().into())],
    )));
    for mount in &inspect.mounts {
        let source = match (&mount.name, mount.kind.as_str()) {
            (Some(name), "volume") => name.as_str(),
            _ => mount.source.as_str(),
        };
        let mode = match mount.writable {
            true => String::new(),
            false => format!(" · {}", L10N.tr("inspector-read-only")),
        };
        lines.push(fact(&mount.destination, &format!("{source}{mode}")));
    }
    let labels = inspect.labels();
    lines.push(section(
        &L10N.tr_args("inspector-labels", [("count", labels.len().into())]),
    ));
    lines.extend(
        labels
            .into_iter()
            .map(|(key, value)| note(&format!("{key}={value}"))),
    );
    lines
}

fn graph(title: &str, values: &(Vec<Series>, String)) -> Graph {
    let mut graph =
        Graph::new(title, GraphKind::Area, HISTORY).caption(title, &L10N.tr("inspector-minute"));
    graph.set_series(values.0.clone());
    graph.set_note(Some(&values.1));
    graph
}

pub fn build(
    subject: Subject<'_>,
    usage: Option<&Usage>,
    intents: &Intents,
) -> (Box<dyn Widget>, Option<GraphIds>) {
    let mut lines: Vec<Box<dyn Widget>> = Vec::new();
    let mut ids = None;
    match subject {
        Subject::Nothing => lines.push(note(&L10N.tr("inspector-none"))),
        Subject::Many(count) => {
            lines.push(note(
                &L10N.tr_args("inspector-many", [("count", count.into())]),
            ));
        }
        Subject::Project {
            name,
            running,
            total,
            directory,
        } => {
            lines.push(boxed(Text::new(name, TextRole::Heading)));
            lines.push(note(&L10N.tr_args(
                "project-running",
                [("running", running.into()), ("total", total.into())],
            )));
            if let Some(directory) = directory {
                lines.push(fact(&L10N.tr("inspector-project-dir"), directory));
            }
        }
        Subject::Container { container, inspect } => {
            lines.push(boxed(Text::new(container.name(), TextRole::Heading)));
            lines.push(note(&container.status));
            if container.running() {
                let graphs = usage.cloned().unwrap_or_default().graphs();
                let shown = [
                    graph(&L10N.tr("stats-cpu"), &graphs.cpu),
                    graph(&L10N.tr("stats-memory"), &graphs.memory),
                    graph(&L10N.tr("stats-network"), &graphs.network),
                    graph(&L10N.tr("stats-disk"), &graphs.disk),
                ];
                ids = Some(GraphIds {
                    cpu: shown[0].id(),
                    memory: shown[1].id(),
                    network: shown[2].id(),
                    disk: shown[3].id(),
                });
                lines.push(section(&L10N.tr("inspector-stats")));
                lines.extend(shown.into_iter().map(boxed));
            }
            if let Some(inspect) = inspect {
                lines.extend(container_lines(inspect, intents));
            }
        }
    }
    let content = Stack::new(Axis::Vertical, SPACING, PADDING, lines);
    (boxed(ScrollView::new(content)), ids)
}

pub fn update(ui: &mut Ui, ids: GraphIds, graphs: Graphs) {
    for (id, (series, note)) in [
        (ids.cpu, graphs.cpu),
        (ids.memory, graphs.memory),
        (ids.network, graphs.network),
        (ids.disk, graphs.disk),
    ] {
        if let Some(graph) = ui.widget_as::<Graph>(id) {
            graph.set_series(series);
            graph.set_note(Some(&note));
        }
    }
}
