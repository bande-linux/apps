mod not_ready;
mod widgets;

use std::cell::RefCell;
use std::rc::Rc;

use bande_app_menu::Entry;
use bande_toolkit::ui::{Cell, Column, Context, Label, Row, Table, Ui, Widget, WidgetId};
use bande_toolkit::{Appearance, Header, Palette, SHEET_INSET, TextSystem};

pub use self::widgets::SIDEBAR_WIDTH;
pub(crate) use self::widgets::{Centered, dot, symbol};
use self::widgets::{Light, Pill, Root};
use crate::L10N;
use crate::containers::{self, Containers};
use crate::engine::{Action, Engine, EngineInfo, Flavor, Update};
use crate::readiness::Readiness;

const CONTAINERS_SYMBOL: &str = include_str!("../../data/symbols/containers.svg");
const CONTAINERS_KEY: &str = "containers";
const COUNT_WIDTH: f32 = 56.0;
const TITLE_INSET: f32 = 16.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Act(Engine, Action, Vec<String>),
    Relist(Engine),
    OpenLink(String),
    Containers(containers::Request),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
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
    Containers,
}

pub struct VeloxView {
    ui: Ui,
    root: WidgetId,
    sidebar: WidgetId,
    shown: Shown,
    page_for: Option<Readiness>,
    palette: Palette,
    connection: Connection,
    containers: Containers,
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
            shown: Shown::Nothing,
            page_for: None,
            palette,
            connection: Connection::Connecting,
            containers: Containers::new(),
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
        self.containers.reset();
        if let Some(root) = self.ui.widget_as::<Root>(self.root) {
            root.set_tools(Vec::new());
        }
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
        let mut requests = std::mem::take(&mut self.requests);
        requests.extend(self.containers.take_requests());
        requests
    }

    pub fn engine(&self) -> Option<Engine> {
        match &self.connection {
            Connection::Ready(info) => Some(Engine::new(&info.socket)),
            _ => None,
        }
    }

    pub fn command(&mut self, command: &str) -> bool {
        let known = self.containers.command(&mut self.ui, command);
        if known {
            self.refresh();
        }
        known
    }

    pub fn container_menu(&self) -> Vec<Entry> {
        self.containers.menu()
    }

    pub fn view_entries(&self) -> Vec<Entry> {
        vec![self.containers.inspector_entry()]
    }

    pub fn focus_filter(&mut self) {
        self.containers.focus_filter(&mut self.ui);
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        self.containers.set_hidden(hidden);
    }

    pub fn sync_streams(&mut self) {
        self.containers.sync(&self.ui);
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
                self.containers.set_engine(None);
            }
            Update::Ready(info) => {
                self.containers.set_engine(Some(info.clone()));
                self.connection = Connection::Ready(info);
            }
            update => self.containers.apply(&mut self.ui, update),
        }
        self.refresh();
    }

    pub fn pump(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        let pumped = self.containers.pump(&mut self.ui);
        for intent in &intents {
            match intent {
                Intent::Copy(text) => self.ui.context().copy(text),
                Intent::Open(link) => self.requests.push(Request::OpenLink((*link).to_owned())),
                Intent::Alternative(open) => self.alternative = *open,
            }
        }
        if intents.is_empty() && !pumped {
            return false;
        }
        self.refresh();
        true
    }

    fn sidebar_rows(&self) -> Vec<Row> {
        let (running, total) = self.containers.running();
        let count = match &self.connection {
            Connection::Ready(_) => format!("{running}/{total}"),
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
            Connection::Ready(_) => Shown::Containers,
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
        let ready = wanted == Shown::Containers;
        let tools = match (ready, self.containers.has_tools()) {
            (true, false) => Some(self.containers.tools(&self.palette)),
            _ => None,
        };
        let content = match (wanted, self.shown) {
            (Shown::Containers, Shown::Containers) => None,
            (Shown::Containers, _) => Some(self.containers.content()),
            _ => None,
        };
        let Some(root) = self.ui.widget_as::<Root>(self.root) else {
            return;
        };
        root.set_pill(boxed(pill));
        if let Some(widgets) = tools {
            root.set_tools(widgets);
        }
        if !ready && self.containers.has_tools() {
            root.set_tools(Vec::new());
            self.containers.drop_tools();
        }
        if let Some(page) = page {
            root.set_content(page);
        } else if let Some(content) = content {
            root.set_content(content);
        } else if wanted != self.shown {
            root.set_content(boxed(Label::new("")));
        }
        if wanted != Shown::Containers {
            self.containers.leave();
        }
        self.shown = wanted;
        self.containers.refresh(&mut self.ui, &self.palette);
    }
}
