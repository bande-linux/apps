mod not_ready;
mod widgets;

use std::cell::RefCell;
use std::rc::Rc;

use bande_app_menu::Entry;
use bande_toolkit::ui::{Cell, Column, Context, Label, Row, Table, Ui, Widget, WidgetId};
use bande_toolkit::{Appearance, Header, Palette, SHEET_INSET, TextSystem};

pub use self::widgets::SIDEBAR_WIDTH;
pub(crate) use self::widgets::{Centered, Code, Ends, Root, dot, symbol};
use self::widgets::{Light, Pill};
use crate::L10N;
use crate::containers::{self, Containers};
use crate::engine::{Action, Engine, EngineInfo, Flavor, Task, Update};
use crate::machines::{Hub, MACHINES_KEY, MACHINES_SYMBOL, MachinesPane};
use crate::readiness::Readiness;
use crate::resources::Resources;

const CONTAINERS_SYMBOL: &str = include_str!("../../data/symbols/containers.svg");
const OVERVIEW_SYMBOL: &str = include_str!("../../data/symbols/overview.svg");
const IMAGES_SYMBOL: &str = include_str!("../../data/symbols/images.svg");
const VOLUMES_SYMBOL: &str = include_str!("../../data/symbols/volumes.svg");
const NETWORKS_SYMBOL: &str = include_str!("../../data/symbols/networks.svg");
const COUNT_WIDTH: f32 = 60.0;
const TITLE_INSET: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Overview,
    Containers,
    Images,
    Volumes,
    Networks,
    Machines,
}

impl Section {
    pub const ALL: [Self; 6] = [
        Self::Overview,
        Self::Containers,
        Self::Images,
        Self::Volumes,
        Self::Networks,
        Self::Machines,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Containers => "containers",
            Self::Images => "images",
            Self::Volumes => "volumes",
            Self::Networks => "networks",
            Self::Machines => MACHINES_KEY,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Overview => "show-overview",
            Self::Containers => "show-containers",
            Self::Images => "show-images",
            Self::Volumes => "show-volumes",
            Self::Networks => "show-networks",
            Self::Machines => "show-machines",
        }
    }

    pub fn title(self) -> String {
        L10N.tr(match self {
            Self::Overview => "pane-overview",
            Self::Containers => "pane-containers",
            Self::Images => "pane-images",
            Self::Volumes => "pane-volumes",
            Self::Networks => "pane-networks",
            Self::Machines => "machine-pane",
        })
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Overview => OVERVIEW_SYMBOL,
            Self::Containers => CONTAINERS_SYMBOL,
            Self::Images => IMAGES_SYMBOL,
            Self::Volumes => VOLUMES_SYMBOL,
            Self::Networks => NETWORKS_SYMBOL,
            Self::Machines => MACHINES_SYMBOL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Act(Engine, Action, Vec<String>),
    Relist(Engine),
    OpenLink(String),
    Task(Engine, String, Task),
    Watch(Engine, Vec<String>),
    Containers(containers::Request),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Navigate(String),
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
    section: Section,
    resources: Resources,
    machines: MachinesPane,
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
        let navigate = Rc::clone(&intents);
        let sidebar = Table::new(vec![
            Column::new("", SIDEBAR_WIDTH).flexible(),
            Column::new("", COUNT_WIDTH).numeric(),
        ])
        .without_header()
        .sidebar()
        .on_selection(move |keys| {
            if let Some(key) = keys.first() {
                push(&navigate, Intent::Navigate(key.clone()));
            }
        });
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
            section: Section::Containers,
            resources: Resources::new(),
            machines: MachinesPane::new(Rc::default(), palette),
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
        self.resources.rebuild();
        self.machines.set_palette(self.palette);
        self.machines.hide();
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
        self.section.title()
    }

    pub fn set_hub(&mut self, hub: &Hub) {
        self.machines.set_fleet(hub.fleet());
        self.refresh();
    }

    pub fn machines(&mut self) -> (&mut MachinesPane, &mut Ui) {
        (&mut self.machines, &mut self.ui)
    }

    pub fn machines_ref(&self) -> &MachinesPane {
        &self.machines
    }

    pub fn machines_changed(&mut self) {
        self.refresh();
    }

    pub fn present_notice(&mut self, title: &str, message: &str) {
        self.ui.present_sheet(
            bande_toolkit::ui::Alert::new(title, message, |_| {})
                .primary(&L10N.tr("button-ok"), "ok"),
        );
    }

    pub fn ready(&self) -> bool {
        matches!(self.connection, Connection::Ready(_))
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn show(&mut self, section: Section) {
        if self.section != section {
            self.section = section;
            self.refresh();
        }
    }

    pub fn command(&mut self, command: &str) -> bool {
        if let Some(section) = Section::ALL
            .into_iter()
            .find(|section| section.command() == command)
        {
            self.show(section);
            return true;
        }
        if self.containers.command(&mut self.ui, command) {
            self.refresh();
            return true;
        }
        let Connection::Ready(info) = &self.connection else {
            return false;
        };
        let info = info.clone();
        if crate::images::opens_images(command) {
            self.section = Section::Images;
            self.refresh();
        }
        let done = self
            .resources
            .command(&mut self.ui, &self.containers.list(), &info, command);
        self.pump();
        self.refresh();
        done
    }

    pub fn resource_menu(&self) -> Option<Entry> {
        let info = match &self.connection {
            Connection::Ready(info) => Some(info),
            _ => None,
        };
        self.resources
            .menu(self.section, &self.containers.list(), info, &self.palette)
    }

    pub fn container_menu(&self) -> Vec<Entry> {
        self.containers.menu()
    }

    pub fn view_entries(&self) -> Vec<Entry> {
        match self.section {
            Section::Containers => vec![self.containers.inspector_entry()],
            Section::Machines => vec![self.machines.inspector_entry()],
            _ => Vec::new(),
        }
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
        if let Connection::Ready(info) = &self.connection {
            let engine = Engine::new(&info.socket);
            for task in self.resources.take_tasks() {
                self.requests
                    .push(Request::Task(engine.clone(), info.api.clone(), task));
            }
            if let Some(ids) = self.resources.take_watch() {
                self.requests.push(Request::Watch(engine, ids));
            }
        }
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

    pub fn focus_filter(&mut self) {
        if self.section == Section::Machines {
            return self.machines.focus_filter(&mut self.ui);
        }
        if !self.resources.focus_filter(&mut self.ui) {
            self.containers.focus_filter(&mut self.ui);
        }
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
        if let Update::Stats(id, sample) = &update {
            self.containers.sample(&mut self.ui, id, *sample);
        }
        let Some(update) = self.resources.apply(&mut self.ui, update) else {
            self.refresh();
            return;
        };
        match update {
            Update::Blocked(readiness) => {
                self.connection = Connection::Blocked(readiness);
                self.containers.set_engine(None);
                self.resources.clear();
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
        let info = match &self.connection {
            Connection::Ready(info) => Some(info.clone()),
            _ => None,
        };
        let acted = self
            .resources
            .pump(&mut self.ui, &self.containers.list(), info.as_ref());
        if let Some(section) = self.resources.take_navigation() {
            self.section = section;
        }
        let pumped = self.containers.pump(&mut self.ui) | self.machines.pump(&mut self.ui);
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        for intent in &intents {
            match intent {
                Intent::Navigate(key) => {
                    if let Some(section) = Section::ALL
                        .into_iter()
                        .find(|section| section.key() == key)
                    {
                        self.section = section;
                    }
                }
                Intent::Copy(text) => self.ui.context().copy(text),
                Intent::Open(link) => self.requests.push(Request::OpenLink((*link).to_owned())),
                Intent::Alternative(open) => self.alternative = *open,
            }
        }
        if intents.is_empty() && !pumped && !acted {
            return false;
        }
        self.refresh();
        true
    }

    fn sidebar_rows(&self) -> Vec<Row> {
        let (running, total) = self.containers.running();
        let ready = matches!(self.connection, Connection::Ready(_));
        let row = |section: Section| {
            let count = match (ready, section) {
                (false, _) => String::new(),
                (true, Section::Containers) => format!("{running}/{total}"),
                (true, _) => self
                    .resources
                    .count(section)
                    .map(|count| count.to_string())
                    .unwrap_or_default(),
            };
            Row::new(
                section.key(),
                vec![
                    Cell::new(section.title()).icon(symbol(section.symbol(), self.palette.accent)),
                    Cell::new(count).muted().tabular(),
                ],
            )
        };
        let mut rows = vec![
            row(Section::Overview),
            Row::new(
                "heading",
                vec![Cell::new(L10N.tr("sidebar-containers")).muted().label()],
            ),
            row(Section::Containers),
            row(Section::Images),
            row(Section::Volumes),
            row(Section::Networks),
        ];
        rows.extend(self.machines.sidebar_rows(&self.palette));
        rows
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
            sidebar.select(&[self.section.key().to_owned()]);
        }
        let pill = self.pill();
        if self.section == Section::Machines {
            if let Some(root) = self.ui.widget_as::<Root>(self.root) {
                root.set_pill(boxed(pill));
            }
            if !self.machines.shown() {
                self.containers.reset();
                self.containers.leave();
                self.resources.hide(&mut self.ui, self.root);
                self.shown = Shown::Nothing;
                self.page_for = None;
                let (content, tools) = self.machines.show();
                if let Some(root) = self.ui.widget_as::<Root>(self.root) {
                    root.set_content(content);
                    root.set_tools(tools);
                }
            }
            self.machines.refresh(&mut self.ui);
            return;
        }
        if self.machines.shown() {
            self.machines.hide();
            if let Some(root) = self.ui.widget_as::<Root>(self.root) {
                root.set_tools(Vec::new());
                root.set_content(boxed(Label::new("")));
            }
        }
        if let (Connection::Ready(info), false) =
            (&self.connection, self.section == Section::Containers)
        {
            if let Some(root) = self.ui.widget_as::<Root>(self.root) {
                root.set_pill(boxed(pill));
            }
            self.containers.reset();
            self.containers.leave();
            self.shown = Shown::Nothing;
            self.page_for = None;
            let info = info.clone();
            self.resources.show(
                &mut self.ui,
                self.root,
                self.section,
                &self.containers.list(),
                &info,
            );
            return;
        }
        self.resources.hide(&mut self.ui, self.root);
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
