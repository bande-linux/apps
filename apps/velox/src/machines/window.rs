use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_toolkit::ui::{
    Axis, Button, Context, Form, FormLine, Label, ScrollView, SegmentedControl, Stack, Stepper,
    TextField, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, FrameRegions, HEADER_LEADING, Header, Palette, Rect, Scene, TextSystem,
    WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};

use super::backend::{Act, Change};
use super::fleet::{Fleet, Status};
use super::present;
use super::sheet::port_free;
use super::widgets::{Bar, Slot};
use super::{Intent, Intents, Request, push};
use crate::L10N;
use crate::ui::{Centered, symbol};

pub const MACHINE_WINDOW_SIZE: (u32, u32) = (720, 520);
const TOOLBAR: f32 = WINDOW_HEADER_HEIGHT as f32;
const TITLE_ROOM: f32 = 200.0;
const CENTER_WIDTH: f32 = 360.0;
const START_SYMBOL: &str = include_str!("../../data/symbols/start.svg");
const STOP_SYMBOL: &str = include_str!("../../data/symbols/stop.svg");
const MIB_PER_GIB: u64 = 1024;
const LOWEST_PORT: u16 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Console,
    Info,
    Snapshots,
    Settings,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Console, Tab::Info, Tab::Snapshots, Tab::Settings];

    fn title(self) -> String {
        L10N.tr(match self {
            Tab::Console => "machine-tab-console",
            Tab::Info => "machine-tab-info",
            Tab::Snapshots => "machine-tab-snapshots",
            Tab::Settings => "machine-tab-settings",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    tab: Tab,
    running: bool,
    pending: bool,
}

pub struct MachineWindow {
    id: String,
    fleet: Rc<RefCell<Fleet>>,
    ui: Ui,
    bar: WidgetId,
    start: WidgetId,
    stop: WidgetId,
    tab: Tab,
    shown: Option<Shown>,
    console: Option<WidgetId>,
    intents: Intents,
    requests: Vec<Request>,
    full_screen: bool,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

impl MachineWindow {
    pub fn new(id: &str, hub: &super::Hub, appearance: Appearance) -> Self {
        let fleet = hub.fleet();
        let palette = appearance.palette();
        let intents = Intents::default();
        let bar = Bar::new(
            super::page::blank(),
            Vec::new(),
            TOOLBAR,
            HEADER_LEADING + TITLE_ROOM,
        );
        let bar_id = bar.id();
        let mut ui = Ui::new(Box::new(bar), Context::new(TextSystem::new(), palette));
        ui.set_appearance(appearance);
        ui.set_sheet_top(TOOLBAR);
        let mut window = Self {
            id: id.to_owned(),
            fleet,
            ui,
            bar: bar_id,
            start: WidgetId::next(),
            stop: WidgetId::next(),
            tab: Tab::Info,
            shown: None,
            console: None,
            intents,
            requests: Vec::new(),
            full_screen: false,
        };
        window.make_tools(palette);
        window.refresh();
        window
    }

    fn make_tools(&mut self, palette: Palette) {
        let intents = &self.intents;
        let tabs_intents = Rc::clone(intents);
        let titles = Tab::ALL.map(Tab::title);
        let tabs = SegmentedControl::new(
            &L10N.tr("machine-tabs"),
            &titles,
            Tab::ALL.iter().position(|shown| *shown == self.tab),
            move |index| push(&tabs_intents, Intent::Tab(index)),
        );
        let button = |svg: &str, label: &str, act: Act| {
            let intents = Rc::clone(intents);
            let click = move || push(&intents, Intent::Act(act));
            let label = L10N.tr(label);
            match symbol(svg, palette.ink) {
                Some(image) => Button::symbol(image, &label, click).tooltip(&label),
                None => Button::new(&label, click),
            }
        };
        let start = button(START_SYMBOL, "machine-start", Act::Start);
        let stop = button(STOP_SYMBOL, "machine-shut-down", Act::ShutDown);
        self.start = start.id();
        self.stop = stop.id();
        if let Some(bar) = self.ui.widget_as::<Bar>(self.bar) {
            bar.set_tools(vec![boxed(tabs), boxed(start), boxed(stop)]);
        }
    }

    pub fn console_slot(&self) -> Option<WidgetId> {
        self.console
    }

    pub fn machine(&self) -> &str {
        &self.id
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn wants_stats(&self) -> bool {
        self.tab == Tab::Info
    }

    pub fn title(&self) -> String {
        let fleet = self.fleet.borrow();
        let Some(entry) = fleet.entry(&self.id) else {
            return L10N.tr("machine-gone");
        };
        L10N.tr_args(
            "machine-window-title",
            [
                ("name", entry.name().to_owned().into()),
                ("status", present::status_text(entry).into()),
            ],
        )
    }

    pub fn gone(&self) -> bool {
        let fleet = self.fleet.borrow();
        fleet.ready() && fleet.entry(&self.id).is_none()
    }

    pub fn can(&self, act: Act) -> bool {
        self.fleet
            .borrow()
            .entry(&self.id)
            .is_some_and(|entry| entry.can(act))
    }

    pub fn can_connect(&self) -> bool {
        self.fleet
            .borrow()
            .entry(&self.id)
            .is_some_and(|entry| entry.running() && entry.ssh_port().is_some())
    }

    pub fn act(&mut self, act: Act) {
        if act == Act::Delete || !self.can(act) {
            return;
        }
        self.fleet.borrow_mut().mark(&self.id, act);
        self.requests.push(Request::Act(self.id.clone(), act));
        self.refresh();
    }

    pub fn connect(&mut self) {
        self.requests.push(Request::Connect(self.id.clone()));
    }

    pub fn present_notice(&mut self, title: &str, message: &str) {
        self.ui.present_sheet(
            bande_toolkit::ui::Alert::new(title, message, |_| {})
                .primary(&L10N.tr("button-ok"), "ok"),
        );
    }

    fn pump(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if intents.is_empty() {
            return false;
        }
        for intent in intents {
            match intent {
                Intent::Tab(index) => {
                    if let Some(tab) = Tab::ALL.get(index) {
                        self.tab = *tab;
                    }
                }
                Intent::Act(act) => self.act(act),
                Intent::Connect => self.connect(),
                Intent::Change(change) => {
                    self.requests.push(Request::Change(self.id.clone(), change));
                }
                _ => {}
            }
        }
        self.refresh();
        true
    }

    pub fn refresh(&mut self) {
        let shown = {
            let fleet = self.fleet.borrow();
            fleet.entry(&self.id).map(|entry| Shown {
                tab: self.tab,
                running: entry.running(),
                pending: entry.pending.is_some(),
            })
        };
        let rebuild = match self.tab {
            Tab::Info => true,
            _ => shown != self.shown,
        };
        if rebuild {
            let content = self.content(shown.as_ref());
            if let Some(bar) = self.ui.widget_as::<Bar>(self.bar) {
                bar.set_content(content);
            }
            self.shown = shown;
        }
        let states = [
            (self.start, self.can(Act::Start)),
            (self.stop, self.can(Act::ShutDown)),
        ];
        for (id, enabled) in states {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }

    fn content(&mut self, shown: Option<&Shown>) -> Box<dyn Widget> {
        self.console = None;
        let Some(shown) = shown else {
            return centered(vec![boxed(
                Label::new(&L10N.tr("machine-gone")).secondary(),
            )]);
        };
        match self.tab {
            Tab::Console => {
                let slot = Slot::new(self.console_placeholder(shown));
                self.console = Some(slot.id());
                boxed(slot)
            }
            Tab::Info => {
                let fleet = self.fleet.borrow();
                let accel = match fleet.status {
                    Status::Ready(accel) => Some(accel),
                    _ => None,
                };
                let lines = fleet
                    .entry(&self.id)
                    .map(|entry| present::info_lines(entry, accel))
                    .unwrap_or_default();
                boxed(ScrollView::new(Form::new(&Tab::Info.title(), lines)))
            }
            Tab::Snapshots => {
                let disabled = |key: &str| {
                    let mut button = Button::new(&L10N.tr(key), || {});
                    button.set_enabled(false);
                    boxed(button)
                };
                centered(vec![
                    boxed(Label::new(&L10N.tr("machine-snapshots-later")).secondary()),
                    boxed(Stack::new(
                        Axis::Horizontal,
                        8.0,
                        0.0,
                        vec![
                            disabled("machine-take-snapshot"),
                            disabled("machine-revert"),
                            disabled("machine-delete"),
                        ],
                    )),
                ])
            }
            Tab::Settings => self.settings(shown),
        }
    }

    fn console_placeholder(&self, shown: &Shown) -> Box<dyn Widget> {
        let intents = Rc::clone(&self.intents);
        match shown.running {
            true => {
                let mut connect = Button::new(&L10N.tr("machine-connect"), move || {
                    push(&intents, Intent::Connect)
                });
                connect.set_enabled(self.can_connect());
                centered(vec![
                    boxed(Label::new(&L10N.tr("machine-no-display"))),
                    boxed(Label::new(&L10N.tr("machine-console-later")).secondary()),
                    row(boxed(connect)),
                ])
            }
            false => {
                let mut start = Button::new(&L10N.tr("machine-start"), move || {
                    push(&intents, Intent::Act(Act::Start))
                });
                start.set_enabled(self.can(Act::Start));
                centered(vec![row(boxed(start))])
            }
        }
    }

    fn settings(&self, shown: &Shown) -> Box<dyn Widget> {
        let fleet = self.fleet.borrow();
        let Some(entry) = fleet.entry(&self.id) else {
            return super::page::blank();
        };
        let manifest = &entry.listed.manifest;
        let locked = shown.running || shown.pending;
        let change = |make: fn(f64) -> Change| {
            let intents = Rc::clone(&self.intents);
            move |value: f64| push(&intents, Intent::Change(make(value)))
        };
        let gigabytes = |value: f64| L10N.tr_args("machine-gigabytes", [("count", value.into())]);
        let cpus = std::thread::available_parallelism().map_or(1, |cpus| cpus.get()) as f64;
        let lockable = |label: &str, value: String, control: Box<dyn Widget>| {
            let widget = match locked {
                true => boxed(Stack::new(
                    Axis::Vertical,
                    2.0,
                    0.0,
                    vec![
                        boxed(Label::new(&value)),
                        boxed(Label::new(&L10N.tr("machine-shut-down-to-change")).secondary()),
                    ],
                )),
                false => control,
            };
            FormLine::Row(L10N.tr(label), widget)
        };
        let name_intents = Rc::clone(&self.intents);
        let port_fleet = Rc::clone(&self.fleet);
        let port_intents = Rc::clone(&self.intents);
        let port = manifest.ssh_port();
        let lines = vec![
            FormLine::Heading(L10N.tr("machine-section-machine")),
            FormLine::Row(
                L10N.tr("machine-name"),
                boxed(TextField::new(
                    &L10N.tr("machine-name"),
                    &manifest.name,
                    move |text| {
                        let text = text.trim();
                        if !text.is_empty() {
                            push(&name_intents, Intent::Change(Change::Name(text.to_owned())));
                        }
                    },
                )),
            ),
            lockable(
                "machine-cpus",
                manifest.cpus.to_string(),
                boxed(Stepper::new(
                    &L10N.tr("machine-cpus"),
                    f64::from(manifest.cpus),
                    (1.0, cpus),
                    1.0,
                    change(|value| Change::Cpus(value as u32)),
                )),
            ),
            lockable(
                "machine-memory",
                gigabytes((manifest.memory_mib / MIB_PER_GIB) as f64),
                boxed(
                    Stepper::new(
                        &L10N.tr("machine-memory"),
                        (manifest.memory_mib / MIB_PER_GIB) as f64,
                        (1.0, 1024.0),
                        1.0,
                        change(|value| Change::Memory(value as u64 * MIB_PER_GIB)),
                    )
                    .format(gigabytes),
                ),
            ),
            FormLine::Row(
                L10N.tr("machine-disk-size"),
                boxed(Label::new(&gigabytes(manifest.disk_gib as f64))),
            ),
            FormLine::Heading(L10N.tr("machine-section-network")),
            lockable(
                "machine-ssh-port",
                port.map_or_else(|| "—".to_owned(), |port| port.to_string()),
                boxed(TextField::new(
                    &L10N.tr("machine-ssh-port"),
                    &port.map(|port| port.to_string()).unwrap_or_default(),
                    move |text| {
                        let Some(port) = text
                            .trim()
                            .parse::<u16>()
                            .ok()
                            .filter(|port| *port >= LOWEST_PORT)
                        else {
                            return;
                        };
                        if port_free(&port_fleet.borrow(), port) {
                            push(&port_intents, Intent::Change(Change::SshPort(port)));
                        }
                    },
                )),
            ),
        ];
        boxed(ScrollView::new(Form::new(&Tab::Settings.title(), lines)))
    }
}

fn centered(children: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
    boxed(Centered::new(
        boxed(Stack::new(Axis::Vertical, 10.0, 0.0, children)),
        CENTER_WIDTH,
    ))
}

fn row(widget: Box<dyn Widget>) -> Box<dyn Widget> {
    boxed(Stack::new(Axis::Horizontal, 0.0, 0.0, vec![widget]))
}

impl View for MachineWindow {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        if full_screen != self.full_screen {
            self.full_screen = full_screen;
            let start = match full_screen {
                true => TITLE_ROOM,
                false => HEADER_LEADING + TITLE_ROOM,
            };
            if let Some(bar) = self.ui.widget_as::<Bar>(self.bar) {
                bar.set_start(start);
            }
        }
        window.set_frame_regions(FrameRegions {
            sidebar: 0.0,
            toolbar: match full_screen {
                true => TOOLBAR,
                false => 0.0,
            },
        });
        window.set_title(&self.title());
        let scene = View::scene(&mut self.ui, window);
        let items = self
            .ui
            .widget_ref::<Bar>(self.bar)
            .map(Bar::tool_frames)
            .unwrap_or_default();
        window.set_header(Header {
            title_x: match full_screen {
                true => 16.0,
                false => HEADER_LEADING,
            },
            items,
            tabs: Vec::new(),
        });
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(&mut self.ui, window, event);
        let pumped = self.pump();
        if let Some(request) = self.ui.take_clipboard_request()
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot copy: {error}");
        }
        handled || pumped
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
        self.make_tools(appearance.palette());
        self.shown = None;
        self.refresh();
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
        if active {
            self.refresh();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
