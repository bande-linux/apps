use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_toolkit::ui::{
    Axis, Button, Context, Form, FormLine, Label, PopUpButton, ScrollView, SegmentedControl, Stack,
    Stepper, Table, TextField, Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, FrameRegions, HEADER_LEADING, Header, Palette, Rect, Scene, TextSystem,
    WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};

use super::backend::{Act, Change};
use super::console::{Console, Outcome};
use super::fleet::{Fleet, Status};
use super::present;
use super::sheet::{CONTINUE, port_free};
use super::snapshots::{self, Sheet, Tool, ToolIds};
use super::widgets::{Bar, Slot, ToolRow};
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
    slot: Option<WidgetId>,
    session: Option<Console>,
    failure: Option<String>,
    hidden: bool,
    snapshot_tools: Option<ToolIds>,
    graphs: Option<crate::containers::GraphIds>,
    selected_snapshot: Option<String>,
    sheet: Option<Sheet>,
    intents: Intents,
    requests: Vec<Request>,
    full_screen: bool,
    terminal: bool,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

impl MachineWindow {
    pub fn new(id: &str, hub: &super::Hub, appearance: Appearance, terminal: bool) -> Self {
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
            tab: Tab::Console,
            shown: None,
            slot: None,
            session: None,
            failure: None,
            hidden: false,
            snapshot_tools: None,
            graphs: None,
            selected_snapshot: None,
            sheet: None,
            intents,
            requests: Vec::new(),
            full_screen: false,
            terminal,
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

    pub fn machine(&self) -> &str {
        &self.id
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    fn network_row(&self, network: &bande_vm::Network, locked: bool) -> FormLine {
        let mut bridges = bande_vm::bridges();
        if let bande_vm::Network::Bridged(bridge) = network
            && !bridges.contains(bridge)
        {
            bridges.push(bridge.clone());
        }
        let label = |choice: Option<&String>| match choice {
            None => L10N.tr("machine-network-shared"),
            Some(bridge) => L10N.tr_args(
                "machine-network-bridged",
                [("bridge", bridge.clone().into())],
            ),
        };
        let current = match network {
            bande_vm::Network::Shared => 0,
            bande_vm::Network::Bridged(bridge) => bridges
                .iter()
                .position(|known| known == bridge)
                .map_or(0, |index| index + 1),
        };
        let mut choices = vec![label(None)];
        choices.extend(bridges.iter().map(|bridge| label(Some(bridge))));
        let hint = match (locked, bridges.is_empty()) {
            (true, _) => Some("machine-shut-down-to-change"),
            (false, true) => Some("machine-network-no-bridge"),
            (false, false) => None,
        };
        let control: Box<dyn Widget> = match locked {
            true => boxed(Label::new(&choices[current])),
            false => {
                let intents = Rc::clone(&self.intents);
                boxed(PopUpButton::new(
                    &L10N.tr("machine-network"),
                    &choices,
                    current,
                    move |index| {
                        let network =
                            match index.checked_sub(1).and_then(|index| bridges.get(index)) {
                                Some(bridge) => bande_vm::Network::Bridged(bridge.clone()),
                                None => bande_vm::Network::Shared,
                            };
                        push(&intents, Intent::Change(Change::Network(network)));
                    },
                ))
            }
        };
        let mut column = vec![control];
        column.extend(hint.map(|key| boxed(Label::new(&L10N.tr(key)).secondary())));
        FormLine::Row(
            L10N.tr("machine-network"),
            boxed(Stack::new(Axis::Vertical, 2.0, 0.0, column)),
        )
    }

    pub fn wants_stats(&self) -> bool {
        self.tab == Tab::Info
    }

    pub fn displays(&self) -> (String, u32) {
        let fleet = self.fleet.borrow();
        let Some(entry) = fleet.entry(&self.id) else {
            return (String::new(), 0);
        };
        let manifest = &entry.listed.manifest;
        let shown = match entry.running() && manifest.preset != bande_vm::Preset::Windows {
            true => manifest.displays.clamp(1, bande_vm::MAX_DISPLAYS),
            false => 0,
        };
        (entry.name().to_owned(), shown)
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
            .is_some_and(|entry| entry.running() && entry.ssh_target().is_some())
    }

    pub fn act(&mut self, act: Act) {
        let act = match act {
            Act::Start if self.can(Act::Resume) => Act::Resume,
            act => act,
        };
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

    pub fn set_terminal(&mut self, installed: bool) {
        if self.terminal != installed {
            self.terminal = installed;
            self.shown = None;
            self.refresh();
        }
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
                Intent::RetryConsole => self.failure = None,
                Intent::Snapshot(keys) => self.selected_snapshot = keys.first().cloned(),
                Intent::SnapshotTool(tool) => self.snapshot_tool(tool),
                Intent::Text(text) => match &mut self.sheet {
                    Some(Sheet::Take { name }) => *name = text,
                    Some(Sheet::Clone(draft)) => draft.name = text,
                    _ => {}
                },
                Intent::Port(text) => {
                    if let Some(Sheet::Clone(draft)) = &mut self.sheet {
                        draft.port = text;
                    }
                }
                Intent::Linked(linked) => {
                    if let Some(Sheet::Clone(draft)) = &mut self.sheet {
                        draft.linked = linked;
                    }
                }
                Intent::Choice(choice) => self.sheet_choice(&choice),
                Intent::Graphics3d(on) => {
                    self.requests
                        .push(Request::Change(self.id.clone(), Change::Graphics3d(on)));
                }
                Intent::StartAtLogin(on) => {
                    self.requests
                        .push(Request::Change(self.id.clone(), Change::StartAtLogin(on)));
                }
                Intent::Eject => {
                    self.requests
                        .push(Request::Change(self.id.clone(), Change::Eject));
                }
                Intent::AddShare => self.requests.push(Request::ChooseFolder(self.id.clone())),
                Intent::RemoveShare(tag) => {
                    self.requests
                        .push(Request::Change(self.id.clone(), Change::RemoveShare(tag)));
                }
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
                pending: match self.tab {
                    Tab::Console => !entry.running() && entry.pending.is_some(),
                    _ => entry.pending.is_some(),
                },
            })
        };
        self.sync_console();
        let rebuild = shown != self.shown;
        if rebuild {
            let content = self.content(shown.as_ref());
            if let Some(bar) = self.ui.widget_as::<Bar>(self.bar) {
                bar.set_content(content);
            }
            self.shown = shown;
        }
        self.fill_snapshots();
        self.fill_graphs();
        let states = [
            (self.start, self.can(Act::Start) || self.can(Act::Resume)),
            (self.stop, self.can(Act::ShutDown)),
        ];
        for (id, enabled) in states {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }

    fn sync_console(&mut self) {
        let (running, settled) = {
            let fleet = self.fleet.borrow();
            let entry = fleet.entry(&self.id);
            (
                entry.is_some_and(|entry| entry.running()),
                entry.is_some_and(|entry| entry.pending.is_none()),
            )
        };
        let wanted = self.tab == Tab::Console && running && !self.hidden;
        if !wanted {
            if self.session.take().is_some() {
                self.show_in_slot(None);
            }
            if !running {
                self.failure = None;
            }
            return;
        }
        if self.session.is_none() && settled && self.failure.is_none() {
            match Console::open(&self.id, 0) {
                Ok(console) => {
                    self.session = Some(console);
                    self.shown = None;
                }
                Err(error) => self.failure = Some(error.to_string()),
            }
        }
    }

    fn show_in_slot(&mut self, view: Option<Box<dyn Widget>>) {
        let content = view.unwrap_or_else(|| match self.shown.clone() {
            Some(shown) => self.console_placeholder(&shown),
            None => super::page::blank(),
        });
        if let Some(slot) = self.slot.and_then(|slot| self.ui.widget_as::<Slot>(slot)) {
            slot.set(content);
        }
    }

    fn console_settled(&mut self, window: &mut Window, outcome: Outcome) -> bool {
        match outcome {
            Outcome::Nothing => false,
            Outcome::Redraw => true,
            Outcome::Show(view) => {
                self.show_in_slot(Some(view));
                true
            }
            Outcome::NoPicture => {
                self.show_in_slot(None);
                true
            }
            Outcome::Closed(reason) => {
                self.session = None;
                self.failure = Some(reason.unwrap_or_default());
                self.show_in_slot(None);
                window.set_cursor(self.ui.cursor());
                true
            }
        }
    }

    pub fn send_ctrl_alt_delete(&self) {
        if let Some(console) = &self.session {
            console.send_ctrl_alt_delete();
        }
    }

    pub fn has_console(&self) -> bool {
        self.session.is_some()
    }

    fn fill_graphs(&mut self) {
        let Some(ids) = self.graphs.filter(|_| self.tab == Tab::Info) else {
            return;
        };
        let graphs = self
            .fleet
            .borrow()
            .entry(&self.id)
            .map(|entry| entry.history.graphs());
        if let Some(graphs) = graphs {
            crate::containers::update_graphs(&mut self.ui, ids, graphs);
        }
    }

    fn fill_snapshots(&mut self) {
        let Some(tools) = self.snapshot_tools.filter(|_| self.tab == Tab::Snapshots) else {
            return;
        };
        let fleet = Rc::clone(&self.fleet);
        let fleet = fleet.borrow();
        let Some(entry) = fleet.entry(&self.id) else {
            return;
        };
        if let Some(table) = self.ui.widget_as::<Table>(tools.table) {
            table.set_rows(snapshots::rows(entry));
            table.set_expanded_rows(snapshots::expanded(entry));
        }
        let selected = self.selected_snapshot.as_deref().filter(|id| {
            entry
                .listed
                .manifest
                .snapshots
                .iter()
                .any(|snapshot| snapshot.id == *id)
        });
        let states = [
            (tools.take, entry.can(Act::TakeSnapshot)),
            (
                tools.revert,
                selected.is_some_and(|id| snapshots::can_revert(entry, id)),
            ),
            (
                tools.delete,
                selected.is_some() && entry.can(Act::DeleteSnapshot),
            ),
            (tools.clone, entry.can(Act::Clone)),
        ];
        for (id, enabled) in states {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }

    fn present_sheet(&mut self, sheet: Sheet, error: Option<&str>) {
        let fleet = Rc::clone(&self.fleet);
        let fleet = fleet.borrow();
        let Some(entry) = fleet.entry(&self.id) else {
            return;
        };
        if let Some(alert) = snapshots::present(&sheet, &self.intents, entry, error) {
            self.ui.present_sheet(alert);
            self.sheet = Some(sheet);
        }
    }

    pub fn present_take_snapshot(&mut self) {
        let name = {
            let fleet = self.fleet.borrow();
            fleet.entry(&self.id).map(snapshots::next_snapshot_name)
        };
        if let Some(name) = name.filter(|_| self.can(Act::TakeSnapshot)) {
            self.present_sheet(Sheet::Take { name }, None);
        }
    }

    pub fn present_clone(&mut self, snapshot: Option<String>) {
        let draft = {
            let fleet = self.fleet.borrow();
            fleet
                .entry(&self.id)
                .map(|entry| snapshots::draft_clone(&fleet, entry, snapshot))
        };
        if let Some(draft) = draft.filter(|_| self.can(Act::Clone)) {
            self.present_sheet(Sheet::Clone(draft), None);
        }
    }

    fn snapshot_tool(&mut self, tool: Tool) {
        let selected = self.selected_snapshot.clone().filter(|id| {
            self.fleet.borrow().entry(&self.id).is_some_and(|entry| {
                entry
                    .listed
                    .manifest
                    .snapshots
                    .iter()
                    .any(|snapshot| snapshot.id == *id)
            })
        });
        match (tool, selected) {
            (Tool::Take, _) => self.present_take_snapshot(),
            (Tool::Revert, Some(id)) => self.present_sheet(Sheet::Revert(id), None),
            (Tool::Delete, Some(id)) => self.present_sheet(Sheet::Delete(id), None),
            (Tool::Clone, selected) => self.present_clone(selected),
            _ => {}
        }
    }

    fn sheet_choice(&mut self, choice: &str) {
        let Some(sheet) = self.sheet.take() else {
            return;
        };
        if choice != CONTINUE {
            return;
        }
        if let Sheet::Clone(draft) = &sheet {
            let made = snapshots::clone_request(draft, &self.fleet.borrow());
            match made {
                Ok(new) => self.requests.push(Request::Clone(self.id.clone(), new)),
                Err(message) => self.present_sheet(sheet, Some(&message)),
            }
            return;
        }
        if let Some(op) = snapshots::snapshot_op(sheet) {
            self.requests.push(Request::Snapshot(self.id.clone(), op));
        }
    }

    pub fn open_serial(&mut self) {
        self.requests.push(Request::Serial(self.id.clone()));
    }

    pub fn suspend(&mut self) {
        self.act(Act::Suspend);
    }

    pub fn toggle_pause(&mut self) {
        match self.can(Act::Resume) {
            true => self.act(Act::Resume),
            false => self.act(Act::Pause),
        }
    }

    fn content(&mut self, shown: Option<&Shown>) -> Box<dyn Widget> {
        self.slot = None;
        self.snapshot_tools = None;
        self.graphs = None;
        let Some(shown) = shown else {
            return centered(vec![boxed(
                Label::new(&L10N.tr("machine-gone")).secondary(),
            )]);
        };
        match self.tab {
            Tab::Console => {
                let view = self.session.as_mut().and_then(Console::view);
                let slot = Slot::new(view.unwrap_or_else(|| self.console_placeholder(shown)));
                self.slot = Some(slot.id());
                boxed(slot)
            }
            Tab::Info => {
                let fleet = self.fleet.borrow();
                let accel = match fleet.status {
                    Status::Ready(accel) => Some(accel),
                    _ => None,
                };
                let Some(entry) = fleet.entry(&self.id) else {
                    return super::page::blank();
                };
                let mut children: Vec<Box<dyn Widget>> = Vec::new();
                if entry.running() {
                    let (graphs, ids) = crate::containers::graph_widgets(&entry.history);
                    self.graphs = Some(ids);
                    children.push(boxed(Stack::new(Axis::Vertical, 8.0, 16.0, graphs)));
                }
                children.push(boxed(Form::new(
                    &Tab::Info.title(),
                    present::info_lines(entry, accel),
                )));
                boxed(ScrollView::new(Stack::new(
                    Axis::Vertical,
                    0.0,
                    0.0,
                    children,
                )))
            }
            Tab::Snapshots => {
                let table = snapshots::table(&self.intents);
                let button =
                    |key: &str, tool: Tool| snapshots::tool_button(&self.intents, key, tool);
                let take = button("machine-take-snapshot-ellipsis", Tool::Take);
                let revert = button("machine-revert-ellipsis", Tool::Revert);
                let delete = button("machine-delete-snapshot-ellipsis", Tool::Delete);
                let clone = button("machine-clone-ellipsis", Tool::Clone);
                self.snapshot_tools = Some(ToolIds {
                    table: table.id(),
                    take: take.id(),
                    revert: revert.id(),
                    delete: delete.id(),
                    clone: clone.id(),
                });
                boxed(ToolRow::new(
                    boxed(table),
                    vec![boxed(take), boxed(revert), boxed(delete), boxed(clone)],
                    TOOLBAR,
                ))
            }
            Tab::Settings => self.settings(shown),
        }
    }

    fn console_placeholder(&self, shown: &Shown) -> Box<dyn Widget> {
        let intents = Rc::clone(&self.intents);
        if !shown.running {
            let mut start = Button::new(&L10N.tr("machine-start"), move || {
                push(&intents, Intent::Act(Act::Start))
            });
            start.set_enabled(self.can(Act::Start));
            return centered(vec![row(boxed(start))]);
        }
        let message = match (&self.failure, &self.session) {
            (Some(reason), _) if !reason.is_empty() => L10N.tr_args(
                "machine-console-failed",
                [("reason", reason.clone().into())],
            ),
            (Some(_), _) => L10N.tr("machine-console-off"),
            (None, Some(console)) if console.has_closed_picture() => L10N.tr("machine-console-off"),
            (None, _) => L10N.tr("machine-console-connecting"),
        };
        let retry_intents = Rc::clone(&self.intents);
        let mut children = vec![boxed(Label::new(&message).secondary())];
        let mut buttons = Vec::new();
        if self.terminal {
            let mut connect = Button::new(&L10N.tr("machine-connect"), move || {
                push(&intents, Intent::Connect)
            });
            connect.set_enabled(self.can_connect());
            buttons.push(boxed(connect));
        }
        if self.failure.is_some() {
            buttons.push(boxed(Button::new(
                &L10N.tr("machine-console-retry"),
                move || push(&retry_intents, Intent::RetryConsole),
            )));
        }
        children.push(boxed(Stack::new(Axis::Horizontal, 8.0, 0.0, buttons)));
        centered(children)
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
        let mut lines = vec![
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
            self.network_row(&manifest.network, locked),
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
        if let bande_vm::Network::Bridged(_) = manifest.network {
            lines.pop();
        }
        if let Some(iso) = &manifest.installer {
            let intents = Rc::clone(&self.intents);
            let name = std::path::Path::new(iso)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut eject = Button::new(&L10N.tr("machine-eject"), move || {
                push(&intents, Intent::Eject)
            });
            eject.set_enabled(!locked);
            lines.push(FormLine::Row(
                L10N.tr("machine-iso"),
                boxed(Stack::new(
                    Axis::Horizontal,
                    8.0,
                    0.0,
                    vec![boxed(Label::new(&name)), boxed(eject)],
                )),
            ));
        }
        lines.push(FormLine::Heading(L10N.tr("machine-section-display")));
        let has_memory =
            manifest.snapshots.iter().any(|snapshot| snapshot.memory) || manifest.is_suspended();
        let available =
            bande_vm::Host::render_node().is_some() && manifest.preset != bande_vm::Preset::Windows;
        let toggle_intents = Rc::clone(&self.intents);
        let changeable = !locked && !has_memory && available;
        let toggle: Box<dyn Widget> = match changeable {
            true => boxed(Toggle::new(
                &L10N.tr("machine-graphics-3d"),
                manifest.graphics_3d,
                move |on| push(&toggle_intents, Intent::Graphics3d(on)),
            )),
            false => boxed(Label::new(&L10N.tr(match manifest.graphics_3d {
                true => "machine-on",
                false => "machine-off",
            }))),
        };
        let hint = match (available, has_memory, locked) {
            (false, _, _) => Some("machine-graphics-3d-unavailable"),
            (true, true, _) => Some("machine-graphics-3d-snapshots"),
            (true, false, true) => Some("machine-shut-down-to-change"),
            (true, false, false) => None,
        };
        let mut column: Vec<Box<dyn Widget>> = vec![toggle];
        column.extend(hint.map(|key| boxed(Label::new(&L10N.tr(key)).secondary())));
        lines.push(FormLine::Row(
            L10N.tr("machine-graphics-3d"),
            boxed(Stack::new(Axis::Vertical, 2.0, 0.0, column)),
        ));
        let displays = manifest.displays.clamp(1, bande_vm::MAX_DISPLAYS);
        let windows = manifest.preset == bande_vm::Preset::Windows;
        lines.push(match (windows, has_memory) {
            (true, _) => FormLine::Row(L10N.tr("machine-displays"), boxed(Label::new("1"))),
            (false, true) => FormLine::Row(
                L10N.tr("machine-displays"),
                boxed(Stack::new(
                    Axis::Vertical,
                    2.0,
                    0.0,
                    vec![
                        boxed(Label::new(&displays.to_string())),
                        boxed(Label::new(&L10N.tr("machine-graphics-3d-snapshots")).secondary()),
                    ],
                )),
            ),
            (false, false) => lockable(
                "machine-displays",
                displays.to_string(),
                boxed(Stepper::new(
                    &L10N.tr("machine-displays"),
                    f64::from(displays),
                    (1.0, f64::from(bande_vm::MAX_DISPLAYS)),
                    1.0,
                    change(|value| Change::Displays(value as u32)),
                )),
            ),
        });
        lines.push(FormLine::Heading(L10N.tr("machine-section-startup")));
        let login_intents = Rc::clone(&self.intents);
        lines.push(FormLine::Row(
            L10N.tr("machine-start-at-login"),
            boxed(Toggle::new(
                &L10N.tr("machine-start-at-login"),
                entry.listed.starts_at_login,
                move |on| push(&login_intents, Intent::StartAtLogin(on)),
            )),
        ));
        lines.push(FormLine::Heading(L10N.tr("machine-section-shares")));
        for share in &manifest.shares {
            let intents = Rc::clone(&self.intents);
            let tag = share.tag.clone();
            let mut remove = Button::new(&L10N.tr("machine-share-remove"), move || {
                push(&intents, Intent::RemoveShare(tag.clone()))
            });
            remove.set_enabled(!locked);
            lines.push(FormLine::Row(
                share.path.clone(),
                boxed(Stack::new(
                    Axis::Vertical,
                    2.0,
                    0.0,
                    vec![
                        boxed(remove),
                        boxed(
                            Label::new(&L10N.tr_args(
                                "machine-share-mount",
                                [("tag", share.tag.clone().into())],
                            ))
                            .secondary(),
                        ),
                    ],
                )),
            ));
        }
        let intents = Rc::clone(&self.intents);
        let mut add = Button::new(&L10N.tr("machine-share-add"), move || {
            push(&intents, Intent::AddShare)
        });
        add.set_enabled(!locked && bande_vm::Host::virtiofsd().is_some());
        lines.push(FormLine::Row(String::new(), boxed(add)));
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
        if let (Some(console), Some(frame)) = (
            self.session.as_mut(),
            self.slot
                .and_then(|slot| self.ui.widget_ref::<Slot>(slot))
                .map(Widget::frame),
        ) {
            console.place(frame, window.scale());
        }
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

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.session.iter().map(Console::fd).collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        let Some(console) = self.session.as_mut() else {
            return false;
        };
        let outcome = console.pump(window, &mut self.ui);
        let settled = self.console_settled(window, outcome);
        window.set_cursor(self.ui.cursor());
        settled
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match &event {
            WindowEvent::ToplevelChanged(state) if state.suspended != self.hidden => {
                self.hidden = state.suspended;
                self.refresh();
            }
            WindowEvent::ClipboardChanged => {
                if let Some(console) = &mut self.session {
                    console.clipboard_changed(window);
                }
            }
            WindowEvent::Pasted(text) => {
                if let Some(console) = &mut self.session {
                    console.pasted(text);
                }
            }
            _ => {}
        }
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
        let console = self.session.as_ref().and_then(Console::deadline);
        match (self.ui.deadline(), console) {
            (Some(ui), Some(console)) => Some(ui.min(console)),
            (ui, console) => ui.or(console),
        }
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        if let Some(console) = &mut self.session {
            console.tick(now);
        }
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
