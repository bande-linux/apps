mod cards;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use std::time::{Duration, Instant, SystemTime};

use bande_l10n::{Text, text, tr, trn};
use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Axis, CheckState, Checkbox, ScrollView, Stack, Ui, Widget, WidgetId,
};

use self::cards::{Breakdown, Card, EngineCard, EngineText, Grid, Live, Part, Plot, Readout, Tone};
use crate::engine::{EngineInfo, Flavor, Reclaim, Sample, Task};
use crate::machines::Fleet;
use crate::resources::{
    Act, Acts, Field, Fields, Pane, Scope, Store, boxed, push, size, tell, tool,
};
use crate::ui::{Section, symbol};

pub const RECLAIM: &str = "reclaim-space";
const CONTAINERS_SYMBOL: &str = include_str!("../../data/symbols/containers.svg");
const IMAGES_SYMBOL: &str = include_str!("../../data/symbols/images.svg");
const VOLUMES_SYMBOL: &str = include_str!("../../data/symbols/volumes.svg");
const DISK_SYMBOL: &str = include_str!("../../data/symbols/disk.svg");
const ENGINE_SYMBOL: &str = include_str!("../../data/symbols/engine.svg");
const PRUNE_SYMBOL: &str = include_str!("../../data/symbols/prune.svg");
const SPAN: usize = 90;
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
const GIGABYTE: f64 = 1e9;
const SPACING: f32 = 12.0;
const PADDING: f32 = 20.0;
const HEADROOM: f64 = 1.3;
const CPU_FLOOR: f64 = 5.0;
const MEMORY_FLOOR: f64 = 64e6;
const DISK_FLOOR: f64 = 1e6;
const NETWORK_FLOOR: f64 = 1e5;

#[derive(Debug, Clone, Copy, Default)]
struct Point {
    cpu: f64,
    memory: f64,
    read: f64,
    written: f64,
    received: f64,
    sent: f64,
}

type Value = fn(&Point) -> f64;

#[derive(Default)]
pub struct Overview {
    engine: Option<WidgetId>,
    cards: Vec<WidgetId>,
    breakdown: Option<WidgetId>,
    live: Vec<WidgetId>,
    started: Option<SystemTime>,
    samples: HashMap<String, (Sample, Option<Sample>)>,
    round: HashSet<String>,
    history: VecDeque<Point>,
    fleet: Rc<RefCell<Fleet>>,
}

fn rate(now: &Sample, before: Option<&Sample>, value: fn(&Sample) -> u64) -> f64 {
    let Some(before) = before else {
        return 0.0;
    };
    let seconds = now.at.saturating_duration_since(before.at).as_secs_f64();
    match seconds > 0.0 {
        true => value(now).saturating_sub(value(before)) as f64 / seconds,
        false => 0.0,
    }
}

fn speed(bytes: f64) -> String {
    match bytes < 1.0 {
        true => tr!("0 KB/s"),
        false => tr!("{size}/s", size = size(bytes as u64)),
    }
}

impl Overview {
    pub fn set_fleet(&mut self, fleet: Rc<RefCell<Fleet>>) {
        self.fleet = fleet;
    }

    pub fn machines_sampled(&mut self) {
        if self.samples.is_empty() && self.fleet.borrow().running() > 0 {
            self.close_round();
        }
    }

    pub fn sample(&mut self, id: String, sample: Sample) {
        if self.round.contains(&id) {
            self.close_round();
        }
        self.round.insert(id.clone());
        let before = self.samples.get(&id).map(|(latest, _)| *latest);
        self.samples.insert(id, (sample, before));
    }

    pub fn keep(&mut self, ids: &[String]) {
        self.samples.retain(|id, _| ids.contains(id));
        self.round.retain(|id| ids.contains(id));
        if ids.is_empty() {
            self.history.clear();
        }
    }

    fn close_round(&mut self) {
        self.round.clear();
        let mut point = Point::default();
        for (now, before) in self.samples.values() {
            point.cpu += now.cpu;
            point.memory += now.memory as f64;
            point.read += rate(now, before.as_ref(), |sample| sample.read);
            point.written += rate(now, before.as_ref(), |sample| sample.written);
            point.received += rate(now, before.as_ref(), |sample| sample.received);
            point.sent += rate(now, before.as_ref(), |sample| sample.sent);
        }
        for usage in self
            .fleet
            .borrow()
            .entries
            .values()
            .filter_map(|entry| entry.usage)
        {
            point.cpu += f64::from(usage.cpu);
            point.memory += usage.memory as f64;
        }
        if self.history.len() == SPAN {
            self.history.pop_front();
        }
        self.history.push_back(point);
    }

    fn scaled(&self, floor: f64, values: &[Value]) -> Vec<Vec<f32>> {
        let peak = self
            .history
            .iter()
            .flat_map(|point| values.iter().map(move |value| value(point)))
            .fold(floor, f64::max)
            * HEADROOM;
        values
            .iter()
            .map(|value| {
                self.history
                    .iter()
                    .map(|point| (value(point) / peak) as f32)
                    .collect()
            })
            .collect()
    }

    fn single(&self, floor: f64, tone: Tone, value: Value) -> Plot {
        let mut scaled = self.scaled(floor, &[value]);
        Plot::Single(tone, scaled.pop().unwrap_or_default())
    }

    fn mirrored(&self, floor: f64, up: Value, down: Value) -> Plot {
        let mut scaled = self.scaled(floor, &[up, down]).into_iter();
        Plot::Mirrored {
            up: (Tone::Blue, scaled.next().unwrap_or_default()),
            down: (Tone::Red, scaled.next().unwrap_or_default()),
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        let up = SystemTime::now()
            .duration_since(self.started?)
            .ok()?
            .as_secs();
        Some(Instant::now() + Duration::from_secs(MINUTE - up % MINUTE))
    }
}

fn uptime(started: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(started)
        .map_or(0, |up| up.as_secs());
    let (days, hours, minutes) = (seconds / DAY, seconds % DAY / HOUR, seconds % HOUR / MINUTE);
    let time = match (days, hours, minutes) {
        (0, 0, 0) => return tr!("· up just now"),
        (0, 0, minutes) => tr!("{minutes} min", minutes = minutes),
        (0, hours, minutes) => tr!("{hours} h {minutes} min", hours = hours, minutes = minutes),
        (days, hours, _) => tr!("{days} d {hours} h", days = days, hours = hours),
    };
    tr!("· up {time}", time = time)
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.0}", bytes as f64 / GIGABYTE)
}

fn engine_text(info: &EngineInfo) -> EngineText {
    let flavor = match info.flavor {
        Flavor::Docker => text!("Docker {version}"),
        Flavor::Podman => text!("Podman {version}"),
    };
    let engine = flavor.tr_args(&[("version", &info.version.clone() as &dyn std::fmt::Display)]);
    let mut detail = vec![
        trn!("1 CPU", "{count} CPUs", count = info.cpus),
        tr!("{size} GB memory", size = gigabytes(info.memory)),
    ];
    if let Some(disk) = info.disk {
        detail.push(tr!("{size} GB disk", size = gigabytes(disk)));
    }
    let mut versions = vec![tr!("API {version}", version = info.api.clone())];
    if info.rootless {
        versions.push(tr!("rootless"));
    }
    EngineText {
        headline: tr!("{engine} running", engine = engine),
        uptime: info.started.map(uptime).unwrap_or_default(),
        detail: detail.join(" · "),
        app: tr!("Velox {version}", version = env!("CARGO_PKG_VERSION")),
        engine: versions.join(" · "),
        running: true,
    }
}

impl Pane for Overview {
    fn build(&mut self, acts: &Acts, palette: &Palette) -> (Vec<Box<dyn Widget>>, Box<dyn Widget>) {
        let engine = EngineCard::new(symbol(ENGINE_SYMBOL, palette.success));
        self.engine = Some(engine.id());
        let reclaim = tool(
            PRUNE_SYMBOL,
            text!("Reclaim Space…"),
            palette,
            acts,
            Act::Command(RECLAIM),
        );
        let card = |title: Text, svg: &str, tone: Tone, act: Act| {
            let acts = Rc::clone(acts);
            Card::new(&title.tr(), symbol(svg, tone.color(palette)), move || {
                push(&acts, act.clone())
            })
        };
        let stats = [
            card(
                text!("Containers"),
                CONTAINERS_SYMBOL,
                Tone::Green,
                Act::Navigate(Section::Containers),
            ),
            card(
                text!("Images"),
                IMAGES_SYMBOL,
                Tone::Blue,
                Act::Navigate(Section::Images),
            ),
            card(
                text!("Volumes"),
                VOLUMES_SYMBOL,
                Tone::Orange,
                Act::Navigate(Section::Volumes),
            ),
            card(
                text!("Disk"),
                DISK_SYMBOL,
                Tone::Purple,
                Act::Command(RECLAIM),
            ),
        ];
        self.cards = stats.iter().map(Widget::id).collect();
        let breakdown = Breakdown::new(&tr!("Disk Breakdown"));
        self.breakdown = Some(breakdown.id());
        let live = [
            Live::new(&tr!("CPU"), SPAN),
            Live::new(&tr!("Memory"), SPAN),
            Live::new(&tr!("Disk I/O"), SPAN),
            Live::new(&tr!("Network"), SPAN),
        ];
        self.live = live.iter().map(Widget::id).collect();
        let content = Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            vec![
                boxed(engine),
                boxed(Grid::new(
                    4,
                    SPACING,
                    stats.into_iter().map(boxed).collect(),
                )),
                boxed(breakdown),
                boxed(Grid::new(2, SPACING, live.into_iter().map(boxed).collect())),
            ],
        );
        (vec![boxed(reclaim)], boxed(ScrollView::new(content)))
    }

    fn fill(&mut self, ui: &mut Ui, scope: &Scope) {
        self.started = scope.info.started;
        if let Some(card) = self.engine.and_then(|id| ui.widget_as::<EngineCard>(id)) {
            card.set(engine_text(scope.info));
        }
        let containers = scope.containers.len();
        let running = scope
            .containers
            .values()
            .filter(|container| container.running())
            .count();
        let usage = scope.store.usage.as_ref();
        let known = |bytes: Option<u64>| bytes.map_or_else(|| "—".to_owned(), size);
        let disk_caption = match scope.info.disk {
            Some(disk) => tr!("of {size} GB disk", size = gigabytes(disk)),
            None => tr!(
                "{size} reclaimable",
                size = known(usage.map(|usage| {
                    usage.images.reclaimable
                        + usage.containers.reclaimable
                        + usage.build_cache.reclaimable
                })),
            ),
        };
        let values = [
            (
                running.to_string(),
                tr!("of {total} total", total = containers),
            ),
            (
                scope.store.images.len().to_string(),
                tr!(
                    "{size} total",
                    size = known(usage.map(|usage| usage.images.total))
                ),
            ),
            (
                scope.store.volumes.len().to_string(),
                tr!(
                    "{size} stored",
                    size = known(usage.map(|usage| usage.volumes.total))
                ),
            ),
            (known(usage.map(|usage| usage.total())), disk_caption),
        ];
        for (id, (value, caption)) in self.cards.iter().zip(values) {
            if let Some(card) = ui.widget_as::<Card>(*id) {
                card.set(&value, &caption);
            }
        }
        if let Some(breakdown) = self.breakdown.and_then(|id| ui.widget_as::<Breakdown>(id)) {
            let part = |key: Text, bytes: u64, tone: Tone| Part {
                label: key.tr_args(&[("size", &size(bytes) as &dyn std::fmt::Display)]),
                bytes,
                tone,
            };
            breakdown.set(
                usage
                    .map(|usage| {
                        vec![
                            part(text!("Images {size}"), usage.images.total, Tone::Blue),
                            part(
                                text!("Containers {size}"),
                                usage.containers.total,
                                Tone::Green,
                            ),
                            part(text!("Volumes {size}"), usage.volumes.total, Tone::Purple),
                            part(
                                text!("Build cache {size}"),
                                usage.build_cache.total,
                                Tone::Orange,
                            ),
                        ]
                    })
                    .unwrap_or_default(),
            );
        }
        let machines = self.fleet.borrow().running();
        let last = self.history.back().copied().unwrap_or_default();
        let quiet = running == 0 && machines == 0;
        let note = quiet.then(|| tr!("No running containers"));
        let value = |text: String| Readout {
            label: None,
            value: text,
        };
        let labelled = |key: Text, tone: Tone, text: String| Readout {
            label: Some((key.tr(), tone)),
            value: text,
        };
        let live = [
            (
                vec![value(format!("{:.0}%", last.cpu))],
                self.single(CPU_FLOOR, Tone::Green, |point| point.cpu),
            ),
            (
                vec![value(size(last.memory as u64))],
                self.single(MEMORY_FLOOR, Tone::Blue, |point| point.memory),
            ),
            (
                vec![
                    labelled(text!("Read"), Tone::Blue, speed(last.read)),
                    labelled(text!("Write"), Tone::Red, speed(last.written)),
                ],
                self.mirrored(DISK_FLOOR, |point| point.read, |point| point.written),
            ),
            (
                vec![
                    labelled(text!("In"), Tone::Blue, speed(last.received)),
                    labelled(text!("Out"), Tone::Red, speed(last.sent)),
                ],
                self.mirrored(NETWORK_FLOOR, |point| point.received, |point| point.sent),
            ),
        ];
        for (id, (readouts, plot)) in self.live.iter().zip(live) {
            if let Some(card) = ui.widget_as::<Live>(*id) {
                let readouts = match quiet {
                    true => Vec::new(),
                    false => readouts,
                };
                card.set(readouts, plot, note.as_deref());
            }
        }
    }

    fn command(&mut self, _ui: &mut Ui, _acts: &Acts, _scope: &Scope, _command: &str) -> bool {
        false
    }

    fn menu_action(
        &mut self,
        _ui: &mut Ui,
        _acts: &Acts,
        _scope: &Scope,
        _action: &str,
        _keys: &[String],
    ) {
    }

    fn finished(&mut self, ui: &mut Ui, task: &Task, result: &Result<u64, String>) -> bool {
        let Task::Reclaim(_) = task else {
            return false;
        };
        match result {
            Ok(reclaimed) => tell(ui, &tr!("Reclaimed {size}", size = size(*reclaimed)), ""),
            Err(message) => tell(ui, &tr!("Cannot reclaim space"), message),
        }
        true
    }
}

pub fn reclaim_sheet(ui: &mut Ui, acts: &Acts, store: &Store) {
    let chosen = Rc::new(RefCell::new(Reclaim {
        containers: true,
        images: true,
        build_cache: true,
        volumes: false,
    }));
    let usage = store.usage.clone().unwrap_or_default();
    let toggle = |label: Text, on: bool, reclaimable: u64, set: fn(&mut Reclaim, bool)| {
        let chosen = Rc::clone(&chosen);
        let state = match on {
            true => CheckState::On,
            false => CheckState::Off,
        };
        let note = match reclaimable {
            0 => "—".to_owned(),
            bytes => tr!("≈ {size}", size = size(bytes)),
        };
        Field::new(
            "",
            Checkbox::new(&label.tr(), state, move |on| {
                set(&mut chosen.borrow_mut(), on);
            }),
        )
        .note(&note)
    };
    let fields = Fields::new(vec![
        toggle(
            text!("Stopped containers"),
            true,
            usage.containers.reclaimable,
            |chosen, on| chosen.containers = on,
        ),
        toggle(
            text!("Unused images"),
            true,
            usage.images.reclaimable,
            |chosen, on| chosen.images = on,
        ),
        toggle(
            text!("Build cache"),
            true,
            usage.build_cache.reclaimable,
            |chosen, on| chosen.build_cache = on,
        ),
        toggle(
            text!("Unused volumes"),
            false,
            usage.volumes.reclaimable,
            |chosen, on| chosen.volumes = on,
        ),
    ]);
    let acts = Rc::clone(acts);
    ui.present_sheet(
        Alert::new(
            &tr!("Reclaim Space"),
            &tr!("Removes engine data nothing uses. Volumes hold real data: leave them off unless you are sure."),
            move |choice| {
                let chosen = *chosen.borrow();
                let any =
                    chosen.containers || chosen.images || chosen.build_cache || chosen.volumes;
                if choice == "reclaim" && any {
                    push(&acts, Act::Task(Task::Reclaim(chosen)));
                }
            },
        )
        .accessory(fields)
        .destructive(&tr!("Reclaim"), "reclaim")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}
