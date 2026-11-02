mod containers;
mod not_ready;
mod widgets;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use bande_toolkit::ui::{
    Alert, Axis, Button, Cell, Column, Context, Label, Row, SearchField, Stack, Table, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{Appearance, Header, Palette, SHEET_INSET, TextSystem};

use self::containers::PROJECT_PREFIX;
pub use self::widgets::SIDEBAR_WIDTH;
use self::widgets::{Centered, Light, Pill, Root, symbol};
use crate::L10N;
use crate::engine::{Action, Container, Engine, EngineInfo, Flavor, Update};
use crate::readiness::Readiness;

const START_SYMBOL: &str = include_str!("../../data/symbols/start.svg");
const STOP_SYMBOL: &str = include_str!("../../data/symbols/stop.svg");
const CONTAINERS_SYMBOL: &str = include_str!("../../data/symbols/containers.svg");
const CONTAINERS_KEY: &str = "containers";
const COUNT_WIDTH: f32 = 48.0;
const TITLE_INSET: f32 = 16.0;
const EMPTY_WIDTH: f32 = 360.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Act(Engine, Action, Vec<String>),
    Relist(Engine),
    OpenLink(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Select(Vec<String>),
    Expand(Vec<String>, bool),
    Filter(String),
    Act(Action),
    Copy(&'static str),
    Open(&'static str),
    Alternative(bool),
}

type Intents = Rc<RefCell<Vec<Intent>>>;

fn push(intents: &Intents, intent: Intent) {
    intents.borrow_mut().push(intent);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Connection {
    Connecting,
    Blocked(Readiness),
    Ready(EngineInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Nothing,
    Page,
    Empty,
    Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tools {
    start: WidgetId,
    stop: WidgetId,
    search: WidgetId,
}

pub struct VeloxView {
    ui: Ui,
    root: WidgetId,
    sidebar: WidgetId,
    table: Option<WidgetId>,
    tools: Option<Tools>,
    shown: Shown,
    page_for: Option<Readiness>,
    palette: Palette,
    connection: Connection,
    containers: BTreeMap<String, Container>,
    pending: HashMap<String, Action>,
    selected: Vec<String>,
    collapsed: HashSet<String>,
    filter: String,
    alternative: bool,
    intents: Intents,
    requests: Vec<Request>,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn short_version(version: &str) -> &str {
    let mut dots = version.match_indices('.').map(|(index, _)| index);
    match (dots.next(), dots.next()) {
        (Some(_), Some(second)) => &version[..second],
        _ => version,
    }
}

fn engine_name(info: &EngineInfo) -> String {
    let flavor = match info.flavor {
        Flavor::Docker => "engine-docker",
        Flavor::Podman => "engine-podman",
    };
    let name = L10N.tr_args(flavor, [("version", short_version(&info.version).into())]);
    match info.rootless {
        true => L10N.tr_args("engine-rootless", [("engine", name.into())]),
        false => name,
    }
}

impl VeloxView {
    pub fn new(appearance: Appearance) -> Self {
        let palette = appearance.palette();
        let intents = Intents::default();
        let sidebar = Table::new(vec![
            Column::new("", SIDEBAR_WIDTH).flexible(),
            Column::new("", COUNT_WIDTH).numeric(),
        ])
        .without_header()
        .sidebar();
        let sidebar_id = sidebar.id();
        let root = Root::new(
            boxed(sidebar),
            boxed(Pill::new(&L10N.tr("engine-connecting"), Light::Waiting)),
            boxed(Label::new("")),
        );
        let root_id = root.id();
        let mut ui = Ui::new(Box::new(root), Context::new(TextSystem::new(), palette));
        ui.set_appearance(appearance);
        ui.set_sheet_top(widgets::TOOLBAR_HEIGHT);
        let mut view = Self {
            ui,
            root: root_id,
            sidebar: sidebar_id,
            table: None,
            tools: None,
            shown: Shown::Nothing,
            page_for: None,
            palette,
            connection: Connection::Connecting,
            containers: BTreeMap::new(),
            pending: HashMap::new(),
            selected: Vec::new(),
            collapsed: HashSet::new(),
            filter: String::new(),
            alternative: false,
            intents,
            requests: Vec::new(),
        };
        view.refresh();
        view
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn ui_ref(&self) -> &Ui {
        &self.ui
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.shown = Shown::Nothing;
        self.tools = None;
        self.refresh();
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        let inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
        if let Some(root) = self.ui.widget_as::<Root>(self.root) {
            root.set_inset(inset);
        }
    }

    pub fn title(&self) -> String {
        L10N.tr("pane-containers")
    }

    pub fn header(&self) -> Header {
        Header {
            title_x: SIDEBAR_WIDTH + TITLE_INSET,
            items: self
                .ui
                .widget_ref::<Root>(self.root)
                .map(Root::tool_frames)
                .unwrap_or_default(),
            tabs: Vec::new(),
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn engine(&self) -> Option<Engine> {
        match &self.connection {
            Connection::Ready(info) => Some(Engine::new(&info.socket)),
            _ => None,
        }
    }

    fn selected_containers(&self) -> Vec<&Container> {
        let mut chosen: Vec<&Container> = Vec::new();
        for key in &self.selected {
            match key.strip_prefix(PROJECT_PREFIX) {
                Some(project) => chosen.extend(
                    self.containers
                        .values()
                        .filter(|container| container.project() == Some(project)),
                ),
                None => chosen.extend(self.containers.get(key)),
            }
        }
        chosen.sort_by(|left, right| left.id.cmp(&right.id));
        chosen.dedup_by(|left, right| left.id == right.id);
        chosen
    }

    fn targets(&self, action: Action) -> Vec<String> {
        self.selected_containers()
            .into_iter()
            .filter(|container| !self.pending.contains_key(&container.id))
            .filter(|container| match action {
                Action::Start => !container.running(),
                Action::Stop => container.running(),
            })
            .map(|container| container.id.clone())
            .collect()
    }

    pub fn can(&self, action: Action) -> bool {
        self.engine().is_some() && !self.targets(action).is_empty()
    }

    pub fn act(&mut self, action: Action) {
        let ids = self.targets(action);
        let Some(engine) = self.engine() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            self.pending.insert(id.clone(), action);
        }
        self.requests.push(Request::Act(engine, action, ids));
        self.refresh();
    }

    pub fn focus_filter(&mut self) {
        if let Some(tools) = self.tools {
            self.ui.set_focus(tools.search);
        }
    }

    pub fn relist(&mut self) {
        if let Some(engine) = self.engine() {
            self.requests.push(Request::Relist(engine));
        }
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Blocked(readiness) => {
                self.connection = Connection::Blocked(readiness);
                self.containers.clear();
                self.pending.clear();
            }
            Update::Ready(info) => self.connection = Connection::Ready(info),
            Update::Listed(listed) => {
                self.containers = listed
                    .into_iter()
                    .map(|container| (container.id.clone(), container))
                    .collect();
                let containers = &self.containers;
                self.pending.retain(|id, action| {
                    containers
                        .get(id)
                        .is_some_and(|container| !settled(container, *action))
                });
            }
            Update::Changed(container) => {
                if self
                    .pending
                    .get(&container.id)
                    .is_some_and(|action| settled(&container, *action))
                {
                    self.pending.remove(&container.id);
                }
                self.containers.insert(container.id.clone(), container);
            }
            Update::Removed(id) => {
                self.pending.remove(&id);
                self.containers.remove(&id);
            }
            Update::Failed {
                id,
                action,
                message,
            } => {
                self.pending.remove(&id);
                let name = self
                    .containers
                    .get(&id)
                    .map_or_else(|| id.clone(), |container| container.name().to_owned());
                let title = match action {
                    Action::Start => "cannot-start",
                    Action::Stop => "cannot-stop",
                };
                self.ui.present_sheet(
                    Alert::new(
                        &L10N.tr_args(title, [("name", name.into())]),
                        &message,
                        |_| {},
                    )
                    .primary(&L10N.tr("button-ok"), "ok"),
                );
            }
        }
        self.refresh();
    }

    pub fn pump(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if intents.is_empty() {
            return false;
        }
        for intent in intents {
            match intent {
                Intent::Select(keys) => self.selected = keys,
                Intent::Expand(keys, expanded) => {
                    for key in keys {
                        match expanded {
                            true => self.collapsed.remove(&key),
                            false => self.collapsed.insert(key),
                        };
                    }
                }
                Intent::Filter(text) => self.filter = text,
                Intent::Act(action) => self.act(action),
                Intent::Copy(text) => self.ui.context().copy(text),
                Intent::Open(link) => self.requests.push(Request::OpenLink(link)),
                Intent::Alternative(open) => self.alternative = open,
            }
        }
        self.refresh();
        true
    }

    fn sidebar_rows(&self) -> Vec<Row> {
        let running = self
            .containers
            .values()
            .filter(|container| container.running())
            .count();
        let count = match &self.connection {
            Connection::Ready(_) => format!("{running}/{}", self.containers.len()),
            _ => String::new(),
        };
        vec![
            Row::new(
                "heading",
                vec![Cell::new(L10N.tr("sidebar-containers")).muted().label()],
            ),
            Row::new(
                CONTAINERS_KEY,
                vec![
                    Cell::new(L10N.tr("pane-containers"))
                        .icon(symbol(CONTAINERS_SYMBOL, self.palette.accent)),
                    Cell::new(count).muted().tabular(),
                ],
            ),
        ]
    }

    fn pill(&self) -> Pill {
        match &self.connection {
            Connection::Connecting => Pill::new(&L10N.tr("engine-connecting"), Light::Waiting),
            Connection::Blocked(_) => Pill::new(&L10N.tr("engine-none"), Light::Off),
            Connection::Ready(info) => Pill::new(&engine_name(info), Light::On),
        }
    }

    fn make_tools(&self) -> (Tools, Vec<Box<dyn Widget>>) {
        let button = |svg: &str, label: &str, action: Action| {
            let intents = Rc::clone(&self.intents);
            let click = move || push(&intents, Intent::Act(action));
            let label = L10N.tr(label);
            match symbol(svg, self.palette.ink) {
                Some(image) => Button::symbol(image, &label, click).tooltip(&label),
                None => Button::new(&label, click),
            }
        };
        let start = button(START_SYMBOL, "action-start", Action::Start);
        let stop = button(STOP_SYMBOL, "action-stop", Action::Stop);
        let intents = Rc::clone(&self.intents);
        let search = SearchField::new(&L10N.tr("filter-containers"), &self.filter, move |text| {
            push(&intents, Intent::Filter(text.to_owned()));
        });
        let tools = Tools {
            start: start.id(),
            stop: stop.id(),
            search: search.id(),
        };
        (tools, vec![boxed(start), boxed(stop), boxed(search)])
    }

    fn make_table(&self) -> Table {
        let select = Rc::clone(&self.intents);
        let expand = Rc::clone(&self.intents);
        Table::new(containers::columns())
            .on_selection(move |keys| push(&select, Intent::Select(keys.to_vec())))
            .on_expand(move |keys, expanded, _| {
                push(&expand, Intent::Expand(keys.to_vec(), expanded));
            })
    }

    fn empty_page() -> Box<dyn Widget> {
        boxed(Centered::new(
            boxed(Stack::new(
                Axis::Vertical,
                6.0,
                0.0,
                vec![
                    boxed(Label::new(&L10N.tr("empty-title"))),
                    boxed(Label::new(&L10N.tr("empty-hint")).secondary()),
                ],
            )),
            EMPTY_WIDTH,
        ))
    }

    fn refresh(&mut self) {
        let rows = self.sidebar_rows();
        if let Some(sidebar) = self.ui.widget_as::<Table>(self.sidebar) {
            sidebar.set_rows(rows);
            sidebar.select(&[CONTAINERS_KEY.to_owned()]);
        }
        let pill = self.pill();
        let wanted = match &self.connection {
            Connection::Connecting => Shown::Nothing,
            Connection::Blocked(_) => Shown::Page,
            Connection::Ready(_) if self.containers.is_empty() => Shown::Empty,
            Connection::Ready(_) => Shown::Table,
        };
        let page = match &self.connection {
            Connection::Blocked(readiness)
                if self.shown != Shown::Page || self.page_for.as_ref() != Some(readiness) =>
            {
                self.page_for = Some(readiness.clone());
                Some(not_ready::page(readiness, &self.intents, self.alternative))
            }
            Connection::Blocked(_) => None,
            _ => {
                self.page_for = None;
                None
            }
        };
        let ready = matches!(self.connection, Connection::Ready(_));
        let tools = match (ready, self.tools) {
            (true, None) => Some(self.make_tools()),
            _ => None,
        };
        let table = match (wanted, self.shown) {
            (Shown::Table, Shown::Table) => None,
            (Shown::Table, _) => Some(self.make_table()),
            _ => None,
        };
        let Some(root) = self.ui.widget_as::<Root>(self.root) else {
            return;
        };
        root.set_pill(boxed(pill));
        if let Some((ids, widgets)) = tools {
            root.set_tools(widgets);
            self.tools = Some(ids);
        }
        if !ready && self.tools.is_some() {
            root.set_tools(Vec::new());
            self.tools = None;
        }
        if let Some(page) = page {
            root.set_content(page);
        } else if let Some(table) = table {
            self.table = Some(table.id());
            root.set_content(boxed(table));
        } else if wanted != self.shown {
            root.set_content(match wanted {
                Shown::Empty => Self::empty_page(),
                _ => boxed(Label::new("")),
            });
        }
        if wanted != Shown::Table {
            self.table = None;
            self.selected.clear();
        }
        self.shown = wanted;
        self.fill_table();
        self.update_tools();
    }

    fn fill_table(&mut self) {
        let Some(table) = self.table else {
            return;
        };
        let rows = containers::rows(&self.palette, &self.containers, &self.pending, &self.filter);
        let expanded: Vec<String> = rows
            .iter()
            .filter(|row| !row.children.is_empty() && !self.collapsed.contains(&row.key))
            .map(|row| row.key.clone())
            .collect();
        if let Some(table) = self.ui.widget_as::<Table>(table) {
            table.set_rows(rows);
            table.set_expanded_rows(expanded);
        }
    }

    fn update_tools(&mut self) {
        let Some(tools) = self.tools else {
            return;
        };
        let states = [
            (tools.start, self.can(Action::Start)),
            (tools.stop, self.can(Action::Stop)),
        ];
        for (id, enabled) in states {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }
}

fn settled(container: &Container, action: Action) -> bool {
    match action {
        Action::Start => container.state == "running",
        Action::Stop => !container.running(),
    }
}
