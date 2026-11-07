mod cards;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Axis, Button, CheckState, Checkbox, ScrollView, Series, SeriesColor, Stack, Ui, Widget,
    WidgetId,
};

use self::cards::{Caption, Card, Grid, Live};
use crate::L10N;
use crate::engine::{Flavor, Reclaim, Sample, Task};
use crate::machines::Fleet;
use crate::resources::{
    Act, Acts, Field, Fields, Pane, Scope, Store, Strip, boxed, push, size, tell,
};
use crate::ui::Section;

pub const RECLAIM: &str = "reclaim-space";
const SPAN: usize = 60;
const SPACING: f32 = 12.0;
const PADDING: f32 = 20.0;
const HEADROOM: f64 = 1.15;
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

type Line = (SeriesColor, fn(&Point) -> f64);

#[derive(Default)]
pub struct Overview {
    engine: Option<WidgetId>,
    cards: Vec<WidgetId>,
    live: Vec<WidgetId>,
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

    fn series(&self, floor: f64, values: &[Line]) -> Vec<Series> {
        let peak = self
            .history
            .iter()
            .flat_map(|point| values.iter().map(move |(_, value)| value(point)))
            .fold(floor, f64::max)
            * HEADROOM;
        values
            .iter()
            .map(|(color, value)| {
                Series::new(
                    *color,
                    self.history
                        .iter()
                        .map(|point| (value(point) / peak) as f32),
                )
            })
            .collect()
    }

    fn engine_line(scope: &Scope) -> String {
        let info = scope.info;
        let flavor = match info.flavor {
            Flavor::Docker => "engine-docker",
            Flavor::Podman => "engine-podman",
        };
        let mut parts = vec![
            L10N.tr_args(flavor, [("version", info.version.clone().into())]),
            L10N.tr_args("engine-api", [("version", info.api.clone().into())]),
        ];
        if info.rootless {
            parts.push(L10N.tr("rootless"));
        }
        parts.push(info.socket.display().to_string());
        parts.join(" · ")
    }
}

impl Pane for Overview {
    fn build(
        &mut self,
        acts: &Acts,
        _palette: &Palette,
    ) -> (Vec<Box<dyn Widget>>, Box<dyn Widget>) {
        let caption = Caption::new();
        self.engine = Some(caption.id());
        let reclaim = {
            let acts = Rc::clone(acts);
            Button::new(&L10N.tr("reclaim-space-menu"), move || {
                push(&acts, Act::Command(RECLAIM));
            })
        };
        let card = |title: &str, act: Act| {
            let acts = Rc::clone(acts);
            Card::new(&L10N.tr(title), move || push(&acts, act.clone()))
        };
        let stats = [
            card("pane-containers", Act::Navigate(Section::Containers)),
            card("pane-images", Act::Navigate(Section::Images)),
            card("pane-volumes", Act::Navigate(Section::Volumes)),
            card("card-disk", Act::Command(RECLAIM)),
            card("machine-pane", Act::Navigate(Section::Machines)),
        ];
        self.cards = stats.iter().map(Widget::id).collect();
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
                boxed(Strip::new(vec![boxed(caption), boxed(reclaim)])),
                boxed(Grid::new(
                    5,
                    SPACING,
                    stats.into_iter().map(boxed).collect(),
                )),
                boxed(Grid::new(2, SPACING, live.into_iter().map(boxed).collect())),
            ],
        );
        (Vec::new(), boxed(ScrollView::new(content)))
    }

    fn fill(&mut self, ui: &mut Ui, scope: &Scope) {
        if let Some(caption) = self.engine.and_then(|id| ui.widget_as::<Caption>(id)) {
            caption.set_text(&Self::engine_line(scope));
        }
        let containers = scope.containers.len();
        let running = scope
            .containers
            .values()
            .filter(|container| container.running())
            .count();
        let usage = scope.store.usage.as_ref();
        let known = |bytes: Option<u64>| bytes.map_or_else(|| "—".to_owned(), size);
        let images = scope.store.images.len();
        let volumes = scope.store.volumes.len();
        let machines = (
            self.fleet.borrow().running(),
            self.fleet.borrow().entries.len(),
        );
        let values = [
            (
                running.to_string(),
                L10N.tr_args("card-of-total", [("total", containers.into())]),
            ),
            (
                images.to_string(),
                L10N.tr_args(
                    "card-total",
                    [("size", known(usage.map(|usage| usage.images.total)).into())],
                ),
            ),
            (
                volumes.to_string(),
                L10N.tr_args(
                    "card-stored",
                    [("size", known(usage.map(|usage| usage.volumes.total)).into())],
                ),
            ),
            (
                known(usage.map(|usage| usage.total())),
                L10N.tr_args(
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
            ),
            (
                machines.0.to_string(),
                L10N.tr_args("card-of-total", [("total", machines.1.into())]),
            ),
        ];
        for (id, (value, caption)) in self.cards.iter().zip(values) {
            if let Some(card) = ui.widget_as::<Card>(*id) {
                card.set(&value, &caption);
            }
        }
        let last = self.history.back().copied().unwrap_or_default();
        let quiet = running == 0 && machines.0 == 0;
        let note = quiet.then(|| L10N.tr("no-running-containers"));
        let live = [
            (
                format!("{:.0}%", last.cpu),
                self.series(CPU_FLOOR, &[(SeriesColor::Success, |point| point.cpu)]),
            ),
            (
                size(last.memory as u64),
                self.series(
                    MEMORY_FLOOR,
                    &[(SeriesColor::Primary, |point| point.memory)],
                ),
            ),
            (
                L10N.tr_args(
                    "disk-rates",
                    [
                        ("read", speed(last.read).into()),
                        ("written", speed(last.written).into()),
                    ],
                ),
                self.series(
                    DISK_FLOOR,
                    &[
                        (SeriesColor::Primary, |point| point.read),
                        (SeriesColor::Secondary, |point| point.written),
                    ],
                ),
            ),
            (
                L10N.tr_args(
                    "network-rates",
                    [
                        ("received", speed(last.received).into()),
                        ("sent", speed(last.sent).into()),
                    ],
                ),
                self.series(
                    NETWORK_FLOOR,
                    &[
                        (SeriesColor::Primary, |point| point.received),
                        (SeriesColor::Secondary, |point| point.sent),
                    ],
                ),
            ),
        ];
        for (id, (value, series)) in self.live.iter().zip(live) {
            if let Some(card) = ui.widget_as::<Live>(*id) {
                let value = match quiet {
                    true => String::new(),
                    false => value,
                };
                card.set(&value, series, note.as_deref());
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
