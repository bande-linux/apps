use std::rc::Rc;

use bande_l10n::tr;
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
        "exited" => tr!("Exited ({code})", code = state.exit_code),
        status => {
            let mut chars = status.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    };
    if state.oom_killed {
        text = tr!("{state}, out of memory", state = text);
    }
    text
}

fn container_lines(inspect: &Inspect, intents: &Intents) -> Vec<Box<dyn Widget>> {
    let mut lines = vec![section(&tr!("CONTAINER"))];
    lines.push(fact(&tr!("Image"), &inspect.config.image));
    lines.push(fact(&tr!("State"), &state_text(inspect)));
    if !inspect.state.error.is_empty() {
        lines.push(note(&inspect.state.error));
    }
    if let Some(created) = when(&inspect.created) {
        lines.push(fact(&tr!("Created"), &created));
    }
    lines.push(fact(&tr!("Restarts"), &inspect.restart_count.to_string()));
    if inspect.state.restarting || inspect.restart_count >= CRASH_LOOP {
        lines.push(note(&tr!("It keeps restarting. Check its logs.")));
    }
    lines.push(section(&tr!("PROCESS")));
    lines.push(fact(&tr!("Command"), &inspect.command()));
    if !inspect.config.working_dir.is_empty() {
        lines.push(fact(&tr!("Working Folder"), &inspect.config.working_dir));
    }
    let environment = inspect.config.env.as_deref().unwrap_or_default();
    lines.push(section(&tr!(
        "ENVIRONMENT ({count})",
        count = environment.len()
    )));
    lines.extend(environment.iter().map(|variable| note(variable)));
    lines.push(section(&tr!("NETWORK")));
    if let Some(ip) = inspect.ip() {
        lines.push(fact(&tr!("IP Address"), ip));
    }
    let ports = inspect.ports();
    if ports.is_empty() {
        lines.push(note(&tr!("No published ports")));
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
    lines.push(section(&tr!(
        "MOUNTS ({count})",
        count = inspect.mounts.len()
    )));
    for mount in &inspect.mounts {
        let source = match (&mount.name, mount.kind.as_str()) {
            (Some(name), "volume") => name.as_str(),
            _ => mount.source.as_str(),
        };
        let mode = match mount.writable {
            true => String::new(),
            false => format!(" · {}", tr!("read-only")),
        };
        lines.push(fact(&mount.destination, &format!("{source}{mode}")));
    }
    let labels = inspect.labels();
    lines.push(section(&tr!("LABELS ({count})", count = labels.len())));
    lines.extend(
        labels
            .into_iter()
            .map(|(key, value)| note(&format!("{key}={value}"))),
    );
    lines
}

fn graph(title: &str, values: &(Vec<Series>, String)) -> Graph {
    let mut graph = Graph::new(title, GraphKind::Area, HISTORY).caption(title, &tr!("1 min"));
    graph.set_series(values.0.clone());
    graph.set_note(Some(&values.1));
    graph
}

pub fn graph_widgets(usage: &Usage) -> (Vec<Box<dyn Widget>>, GraphIds) {
    let graphs = usage.graphs();
    let shown = [
        graph(&tr!("CPU"), &graphs.cpu),
        graph(&tr!("Memory"), &graphs.memory),
        graph(&tr!("Network"), &graphs.network),
        graph(&tr!("Disk"), &graphs.disk),
    ];
    let ids = GraphIds {
        cpu: shown[0].id(),
        memory: shown[1].id(),
        network: shown[2].id(),
        disk: shown[3].id(),
    };
    (shown.into_iter().map(boxed).collect(), ids)
}

pub fn panel(title: &str, note_text: &str, facts: &[(String, String)]) -> Box<dyn Widget> {
    let mut lines: Vec<Box<dyn Widget>> = Vec::new();
    if !title.is_empty() {
        lines.push(boxed(Text::new(title, TextRole::Heading)));
    }
    if !note_text.is_empty() {
        lines.push(note(note_text));
    }
    lines.extend(facts.iter().map(|(key, value)| fact(key, value)));
    boxed(ScrollView::new(Stack::new(
        Axis::Vertical,
        SPACING,
        PADDING,
        lines,
    )))
}

pub fn build(
    subject: Subject<'_>,
    usage: Option<&Usage>,
    intents: &Intents,
) -> (Box<dyn Widget>, Option<GraphIds>) {
    let mut lines: Vec<Box<dyn Widget>> = Vec::new();
    let mut ids = None;
    match subject {
        Subject::Nothing => lines.push(note(&tr!("No Selection"))),
        Subject::Many(count) => {
            lines.push(note(&tr!("{count} Selected", count = count)));
        }
        Subject::Project {
            name,
            running,
            total,
            directory,
        } => {
            lines.push(boxed(Text::new(name, TextRole::Heading)));
            lines.push(note(&tr!(
                "{running} of {total} running",
                running = running,
                total = total
            )));
            if let Some(directory) = directory {
                lines.push(fact(&tr!("Folder"), directory));
            }
        }
        Subject::Container { container, inspect } => {
            lines.push(boxed(Text::new(container.name(), TextRole::Heading)));
            lines.push(note(&container.status));
            if container.running() {
                let (shown, graph_ids) = graph_widgets(&usage.cloned().unwrap_or_default());
                ids = Some(graph_ids);
                lines.push(section(&tr!("STATS")));
                lines.extend(shown);
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
