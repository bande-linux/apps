mod cards;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use std::time::{Duration, Instant, SystemTime};

use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Axis, CheckState, Checkbox, ScrollView, Stack, Ui, Widget, WidgetId,
};

use self::cards::{Breakdown, Card, EngineCard, EngineText, Grid, Live, Part, Plot, Readout, Tone};
use crate::L10N;
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
    quiet: bool,
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
        true => L10N.tr("idle-rate"),
        false => L10N.tr_args("per-second", [("size", size(bytes as u64).into())]),
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

    pub fn fill_live(&mut self, ui: &mut Ui) {
        let quiet = self.quiet;
        let last = self.history.back().copied().unwrap_or_default();
        let note = quiet.then(|| L10N.tr("no-running-containers"));
        let value = |text: String| Readout {
            label: None,
            value: text,
        };
        let labelled = |key: &str, tone: Tone, text: String| Readout {
            label: Some((L10N.tr(key), tone)),
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
                    labelled("rate-read", Tone::Blue, speed(last.read)),
                    labelled("rate-write", Tone::Red, speed(last.written)),
                ],
                self.mirrored(DISK_FLOOR, |point| point.read, |point| point.written),
            ),
            (
                vec![
                    labelled("rate-in", Tone::Blue, speed(last.received)),
                    labelled("rate-out", Tone::Red, speed(last.sent)),
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
        (0, 0, 0) => return L10N.tr("engine-up-now"),
        (0, 0, minutes) => L10N.tr_args("duration-minutes", [("minutes", minutes.into())]),
        (0, hours, minutes) => L10N.tr_args(
            "duration-hours",
            [("hours", hours.into()), ("minutes", minutes.into())],
        ),
        (days, hours, _) => L10N.tr_args(
            "duration-days",
            [("days", days.into()), ("hours", hours.into())],
        ),
    };
    L10N.tr_args("engine-up", [("time", time.into())])
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.0}", bytes as f64 / GIGABYTE)
}

fn engine_text(info: &EngineInfo) -> EngineText {
    let flavor = match info.flavor {
        Flavor::Docker => "engine-docker",
        Flavor::Podman => "engine-podman",
    };
    let engine = L10N.tr_args(flavor, [("version", info.version.clone().into())]);
    let mut detail = vec![
        L10N.tr_args("engine-cpus", [("count", info.cpus.into())]),
        L10N.tr_args("engine-memory", [("size", gigabytes(info.memory).into())]),
    ];
    if let Some(disk) = info.disk {
        detail.push(L10N.tr_args("engine-disk", [("size", gigabytes(disk).into())]));
    }
    let mut versions = vec![L10N.tr_args("engine-api", [("version", info.api.clone().into())])];
    if info.rootless {
        versions.push(L10N.tr("rootless"));
    }
    EngineText {
        headline: L10N.tr_args("engine-running", [("engine", engine.into())]),
        uptime: info.started.map(uptime).unwrap_or_default(),
        detail: detail.join(" · "),
        app: L10N.tr_args(
            "velox-version",
            [("version", env!("CARGO_PKG_VERSION").into())],
        ),
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
            "reclaim-space-menu",
            palette,
            acts,
            Act::Command(RECLAIM),
        );
        let card = |title: &str, svg: &str, tone: Tone, act: Act| {
            let acts = Rc::clone(acts);
            Card::new(
                &L10N.tr(title),
                symbol(svg, tone.color(palette)),
                move || push(&acts, act.clone()),
            )
        };
        let stats = [
            card(
                "pane-containers",
                CONTAINERS_SYMBOL,
                Tone::Green,
                Act::Navigate(Section::Containers),
            ),
            card(
                "pane-images",
                IMAGES_SYMBOL,
                Tone::Blue,
                Act::Navigate(Section::Images),
            ),
            card(
                "pane-volumes",
                VOLUMES_SYMBOL,
                Tone::Orange,
                Act::Navigate(Section::Volumes),
            ),
            card(
                "card-disk",
                DISK_SYMBOL,
                Tone::Purple,
                Act::Command(RECLAIM),
            ),
        ];
        self.cards = stats.iter().map(Widget::id).collect();
        let breakdown = Breakdown::new(&L10N.tr("card-disk-breakdown"));
        self.breakdown = Some(breakdown.id());
        let live = [
            Live::new(&L10N.tr("card-cpu"), SPAN),
            Live::new(&L10N.tr("card-memory"), SPAN),
            Live::new(&L10N.tr("card-disk-io"), SPAN),
            Live::new(&L10N.tr("card-network"), SPAN),
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
            Some(disk) => L10N.tr_args("card-of-disk", [("size", gigabytes(disk).into())]),
            None => L10N.tr_args(
                "card-reclaimable",
                [(
                    "size",
                    known(usage.map(|usage| {
                        usage.images.reclaimable
                            + usage.containers.reclaimable
                            + usage.build_cache.reclaimable
                    }))
                    .into(),
                )],
            ),
        };
        let values = [
            (
                running.to_string(),
                L10N.tr_args("card-of-total", [("total", containers.into())]),
            ),
            (
                scope.store.images.len().to_string(),
                L10N.tr_args(
                    "card-total",
                    [("size", known(usage.map(|usage| usage.images.total)).into())],
                ),
            ),
            (
                scope.store.volumes.len().to_string(),
                L10N.tr_args(
                    "card-stored",
                    [("size", known(usage.map(|usage| usage.volumes.total)).into())],
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
            let part = |key: &str, bytes: u64, tone: Tone| Part {
                label: L10N.tr_args(key, [("size", size(bytes).into())]),
                bytes,
                tone,
            };
            breakdown.set(
                usage
                    .map(|usage| {
                        vec![
                            part("breakdown-images", usage.images.total, Tone::Blue),
                            part("breakdown-containers", usage.containers.total, Tone::Green),
                            part("breakdown-volumes", usage.volumes.total, Tone::Purple),
                            part(
                                "breakdown-build-cache",
                                usage.build_cache.total,
                                Tone::Orange,
                            ),
                        ]
                    })
                    .unwrap_or_default(),
            );
        }
        let machines = self.fleet.borrow().running();
        self.quiet = running == 0 && machines == 0;
        self.fill_live(ui);
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
            Ok(reclaimed) => tell(
                ui,
                &L10N.tr_args("reclaimed-title", [("size", size(*reclaimed).into())]),
                "",
            ),
            Err(message) => tell(ui, &L10N.tr("cannot-reclaim"), message),
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
    let toggle = |label: &str, on: bool, reclaimable: u64, set: fn(&mut Reclaim, bool)| {
        let chosen = Rc::clone(&chosen);
        let state = match on {
            true => CheckState::On,
            false => CheckState::Off,
        };
        let note = match reclaimable {
            0 => "—".to_owned(),
            bytes => L10N.tr_args("about-size", [("size", size(bytes).into())]),
        };
        Field::new(
            "",
            Checkbox::new(&L10N.tr(label), state, move |on| {
                set(&mut chosen.borrow_mut(), on);
            }),
        )
        .note(&note)
    };
    let fields = Fields::new(vec![
        toggle(
            "reclaim-containers",
            true,
            usage.containers.reclaimable,
            |chosen, on| chosen.containers = on,
        ),
        toggle(
            "reclaim-images",
            true,
            usage.images.reclaimable,
            |chosen, on| chosen.images = on,
        ),
        toggle(
            "reclaim-build-cache",
            true,
            usage.build_cache.reclaimable,
            |chosen, on| chosen.build_cache = on,
        ),
        toggle(
            "reclaim-volumes",
            false,
            usage.volumes.reclaimable,
            |chosen, on| chosen.volumes = on,
        ),
    ]);
    let acts = Rc::clone(acts);
    ui.present_sheet(
        Alert::new(
            &L10N.tr("reclaim-title"),
            &L10N.tr("reclaim-message"),
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
        .destructive(&L10N.tr("reclaim"), "reclaim")
        .cancel(&L10N.tr("button-cancel"), "cancel"),
    );
}
