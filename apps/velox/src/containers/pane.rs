use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use bande_app_menu::Entry;
use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Axis, Button, Label, SearchField, Stack, Table, Ui, Widget, WidgetId,
};

use super::compose::{Compose, Verb};
use super::inspect::Inspect;
use super::inspector::{self, GraphIds, Subject};
use super::logs::LogsTarget;
use super::menu::{self, Facts};
use super::stats::Usage;
use super::table::{self, PROJECT_PREFIX, Shown};
use super::widgets::{PortLinks, Split};
use super::work::{Event, Request as Work};
use crate::L10N;
use crate::engine::{Action, Container, Engine, EngineInfo, Flavor, Sample};
use crate::ui::{Centered, Request, symbol};

const START_SYMBOL: &str = include_str!("../../data/symbols/start.svg");
const STOP_SYMBOL: &str = include_str!("../../data/symbols/stop.svg");
const RESTART_SYMBOL: &str = include_str!("../../data/symbols/restart.svg");
const DELETE_SYMBOL: &str = include_str!("../../data/symbols/delete.svg");
const INSPECTOR_SYMBOL: &str = include_str!("../../data/symbols/inspector.svg");
const EMPTY_WIDTH: f32 = 360.0;
const SHELL: &str = "command -v bash >/dev/null 2>&1 && exec bash || exec sh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    Delete(Vec<String>),
    Down(Compose),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Select(Vec<String>),
    Expand(Vec<String>, bool),
    Filter(String),
    Command(&'static str),
    Menu(String, Vec<String>),
    Activate(Vec<String>),
    Open(String),
    Confirmed(Confirm),
}

pub type Intents = Rc<RefCell<Vec<Intent>>>;

pub fn push(intents: &Intents, intent: Intent) {
    intents.borrow_mut().push(intent);
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

#[derive(Default)]
struct Model {
    info: Option<EngineInfo>,
    list: BTreeMap<String, Container>,
    pending: HashMap<String, Action>,
}

fn wanted(container: &Container, action: Action) -> bool {
    match action {
        Action::Start => !container.running() && !container.paused(),
        Action::Stop | Action::Kill => container.running() || container.paused(),
        Action::Pause => container.state == "running",
        Action::Resume => container.paused(),
        Action::Restart | Action::Remove => true,
    }
}

impl Model {
    fn members(&self, project: &str) -> Vec<&Container> {
        self.list
            .values()
            .filter(|container| container.project() == Some(project))
            .collect()
    }

    fn chosen(&self, keys: &[String]) -> Vec<&Container> {
        let mut chosen: Vec<&Container> = Vec::new();
        for key in keys {
            match key.strip_prefix(PROJECT_PREFIX) {
                Some(project) => chosen.extend(self.members(project)),
                None => chosen.extend(self.list.get(key)),
            }
        }
        chosen.sort_by(|left, right| left.id.cmp(&right.id));
        chosen.dedup_by(|left, right| left.id == right.id);
        chosen
    }

    fn targets(&self, keys: &[String], action: Action) -> Vec<String> {
        match self.info {
            None => Vec::new(),
            Some(_) => self
                .chosen(keys)
                .into_iter()
                .filter(|container| !self.pending.contains_key(&container.id))
                .filter(|container| wanted(container, action))
                .map(|container| container.id.clone())
                .collect(),
        }
    }

    fn single(&self, keys: &[String]) -> Option<&Container> {
        match keys {
            [key] => self.list.get(key),
            _ => None,
        }
    }

    fn project<'a>(&self, keys: &'a [String]) -> Option<&'a str> {
        match keys {
            [key] => key.strip_prefix(PROJECT_PREFIX),
            _ => None,
        }
    }

    fn compose(&self, keys: &[String]) -> Option<Compose> {
        let project = self.project(keys)?;
        Compose::of(project, &self.members(project), self.info.as_ref()?)
    }

    fn facts(&self, keys: &[String]) -> Facts {
        let single = self.single(keys);
        Facts {
            counts: menu::ACTIONS
                .iter()
                .map(|(action, _)| (*action, self.targets(keys, *action).len()))
                .collect(),
            removable: self.targets(keys, Action::Remove).len(),
            single: single.is_some(),
            running: single.is_some_and(Container::running),
            ip: single.and_then(Container::ip).is_some(),
            ports: single.map(Container::published_ports).unwrap_or_default(),
            logs: single.is_some() || self.compose(keys).is_some(),
            compose: self.compose(keys).is_some(),
        }
    }

    fn logs(&self, keys: &[String]) -> Option<LogsTarget> {
        let engine = Engine::new(&self.info.as_ref()?.socket);
        match self.single(keys) {
            Some(container) => Some(LogsTarget::Container {
                engine,
                id: container.id.clone(),
                name: container.name().to_owned(),
            }),
            None => self.compose(keys).map(LogsTarget::Project),
        }
    }

    fn terminal(&self, keys: &[String]) -> Option<Vec<String>> {
        let info = self.info.as_ref()?;
        let container = self.single(keys).filter(|container| container.running())?;
        let host = format!("unix://{}", info.socket.display());
        let mut argv: Vec<String> = match info.flavor {
            Flavor::Docker => vec![
                "env".to_owned(),
                format!("DOCKER_HOST={host}"),
                "docker".to_owned(),
            ],
            Flavor::Podman => vec!["podman".to_owned(), "--url".to_owned(), host],
        };
        argv.extend(
            [
                "exec",
                "--interactive",
                "--tty",
                &container.id,
                "sh",
                "-c",
                SHELL,
            ]
            .map(str::to_owned),
        );
        Some(argv)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tools {
    start: WidgetId,
    stop: WidgetId,
    restart: WidgetId,
    delete: WidgetId,
    search: WidgetId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Main {
    Empty,
    Table {
        links: WidgetId,
        table: WidgetId,
        compact: bool,
    },
}

pub struct Containers {
    model: Rc<RefCell<Model>>,
    selected: Vec<String>,
    collapsed: HashSet<String>,
    filter: String,
    split: Option<WidgetId>,
    main: Option<Main>,
    tools: Option<Tools>,
    inspector: bool,
    inspected: Option<Box<Inspect>>,
    subject: Option<Vec<String>>,
    graphs: Option<GraphIds>,
    usage: HashMap<String, Usage>,
    order: Vec<String>,
    hidden: bool,
    watched: Option<Vec<String>>,
    intents: Intents,
    requests: Vec<Request>,
}

impl Default for Containers {
    fn default() -> Self {
        Self::new()
    }
}

impl Containers {
    pub fn new() -> Self {
        Self {
            model: Rc::default(),
            selected: Vec::new(),
            collapsed: HashSet::new(),
            filter: String::new(),
            split: None,
            main: None,
            tools: None,
            inspector: false,
            inspected: None,
            subject: None,
            graphs: None,
            usage: HashMap::new(),
            order: Vec::new(),
            hidden: false,
            watched: None,
            intents: Intents::default(),
            requests: Vec::new(),
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn running(&self) -> (usize, usize) {
        let model = self.model.borrow();
        let running = model
            .list
            .values()
            .filter(|container| container.running())
            .count();
        (running, model.list.len())
    }

    pub fn set_engine(&mut self, info: Option<EngineInfo>) {
        let mut model = self.model.borrow_mut();
        if info.is_none() {
            model.list.clear();
            model.pending.clear();
            self.usage.clear();
            self.inspected = None;
            self.watched = None;
        }
        model.info = info;
    }

    pub fn reset(&mut self) {
        self.split = None;
        self.main = None;
        self.tools = None;
        self.subject = None;
        self.graphs = None;
    }

    fn act_on(&mut self, action: Action, keys: &[String]) {
        let mut model = self.model.borrow_mut();
        let ids = model.targets(keys, action);
        let Some(engine) = self.engine_of(&model) else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            model.pending.insert(id.clone(), action);
        }
        self.requests.push(Request::Act(engine, action, ids));
    }

    pub fn focus_filter(&mut self, ui: &mut Ui) {
        if let Some(tools) = self.tools {
            ui.set_focus(tools.search);
        }
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    pub fn apply(&mut self, ui: &mut Ui, update: crate::engine::Update) {
        use crate::engine::Update;
        let mut model = self.model.borrow_mut();
        match update {
            Update::Listed(listed) => {
                model.list = listed
                    .into_iter()
                    .map(|container| (container.id.clone(), container))
                    .collect();
                let Model { list, pending, .. } = &mut *model;
                pending.retain(|id, _| list.contains_key(id));
                self.subject = None;
            }
            Update::Changed(container) => {
                if self
                    .inspected
                    .as_ref()
                    .is_some_and(|shown| shown.id == container.id)
                {
                    if let Some(engine) = self.engine_of(&model) {
                        self.requests.push(Request::Containers(Work::Inspect(
                            engine,
                            container.id.clone(),
                        )));
                    }
                    self.subject = None;
                }
                model.list.insert(container.id.clone(), container);
            }
            Update::Removed(id) => {
                model.pending.remove(&id);
                model.list.remove(&id);
                self.usage.remove(&id);
            }
            Update::Done { id, .. } => {
                model.pending.remove(&id);
            }
            Update::Failed {
                id,
                action,
                message,
            } => {
                model.pending.remove(&id);
                let name = model
                    .list
                    .get(&id)
                    .map_or_else(|| id.clone(), |container| container.name().to_owned());
                drop(model);
                alert(ui, &failure_title(action, &name), &message);
            }
            Update::Containers(event) => {
                drop(model);
                self.event(ui, event);
            }
            _ => {}
        }
    }

    fn engine_of(&self, model: &Model) -> Option<Engine> {
        model.info.as_ref().map(|info| Engine::new(&info.socket))
    }

    fn event(&mut self, ui: &mut Ui, event: Event) {
        match event {
            Event::Inspected(id, found) => {
                if self
                    .model
                    .borrow()
                    .single(&self.selected)
                    .is_some_and(|shown| shown.id == id)
                {
                    self.inspected = Some(found);
                    self.subject = None;
                }
            }
            Event::Copied(text) => ui.context().copy(&text),
            Event::Composed {
                project,
                verb,
                failure,
            } => {
                let mut model = self.model.borrow_mut();
                let members: Vec<String> = model
                    .members(&project)
                    .into_iter()
                    .map(|member| member.id.clone())
                    .collect();
                for id in members {
                    model.pending.remove(&id);
                }
                drop(model);
                if let Some(message) = failure {
                    let title = match verb {
                        Verb::Up => "cannot-up",
                        Verb::Down => "cannot-down",
                        Verb::Restart => "cannot-restart-project",
                    };
                    alert(
                        ui,
                        &L10N.tr_args(title, [("name", project.into())]),
                        &message,
                    );
                }
            }
        }
    }

    pub fn pump(&mut self, ui: &mut Ui) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        for intent in &intents {
            match intent {
                Intent::Select(keys) => {
                    self.selected.clone_from(keys);
                    self.subject = None;
                }
                Intent::Expand(keys, expanded) => {
                    for key in keys {
                        match expanded {
                            true => self.collapsed.remove(key),
                            false => self.collapsed.insert(key.clone()),
                        };
                    }
                }
                Intent::Filter(text) => self.filter.clone_from(text),
                Intent::Command(command) => {
                    let keys = self.selected.clone();
                    self.run(ui, command, &keys);
                }
                Intent::Menu(command, keys) => self.run(ui, command, keys),
                Intent::Activate(keys) => self.run(ui, menu::LOGS, keys),
                Intent::Open(link) => self.requests.push(Request::OpenLink(link.clone())),
                Intent::Confirmed(Confirm::Delete(ids)) => self.act_on(Action::Remove, ids),
                Intent::Confirmed(Confirm::Down(compose)) => {
                    self.compose(compose.clone(), Verb::Down)
                }
            }
        }
        !intents.is_empty()
    }

    pub fn command(&mut self, ui: &mut Ui, command: &str) -> bool {
        let known = command == menu::INSPECTOR
            || command.starts_with("container-")
            || command.starts_with("project-");
        if known {
            let keys = self.selected.clone();
            self.run(ui, command, &keys);
        }
        known
    }

    fn compose(&mut self, compose: Compose, verb: Verb) {
        let mut model = self.model.borrow_mut();
        let action = match verb {
            Verb::Up => Action::Start,
            Verb::Down => Action::Remove,
            Verb::Restart => Action::Restart,
        };
        let members: Vec<String> = model
            .members(&compose.project)
            .into_iter()
            .map(|member| member.id.clone())
            .collect();
        for id in members {
            model.pending.insert(id, action);
        }
        self.requests
            .push(Request::Containers(Work::Compose(compose, verb)));
    }

    fn run(&mut self, ui: &mut Ui, command: &str, keys: &[String]) {
        if let Some(action) = menu::action_named(command) {
            return self.act_on(action, keys);
        }
        let model = self.model.borrow();
        let single = model.single(keys).cloned();
        let engine = self.engine_of(&model);
        let compose = model.compose(keys);
        let logs = model.logs(keys);
        let terminal = model.terminal(keys);
        let removable = model.targets(keys, Action::Remove);
        let running = removable.iter().any(|id| {
            model
                .list
                .get(id)
                .is_some_and(|container| container.running())
        });
        drop(model);
        if let Some(port) = command.strip_prefix(menu::COPY_URL) {
            return ui.context().copy(&format!("http://localhost:{port}"));
        }
        if let Some(port) = command.strip_prefix(menu::OPEN_URL) {
            return self
                .requests
                .push(Request::OpenLink(format!("http://localhost:{port}")));
        }
        match (command, single, engine) {
            (menu::INSPECTOR, _, _) => {
                self.inspector = !self.inspector;
                self.subject = None;
            }
            (menu::LOGS, _, _) => self
                .requests
                .extend(logs.map(|target| Request::Containers(Work::Logs(target)))),
            (menu::TERMINAL, _, _) => self
                .requests
                .extend(terminal.map(|argv| Request::Containers(Work::Terminal(argv)))),
            (menu::COPY_NAME, Some(container), _) => ui.context().copy(container.name()),
            (menu::COPY_ID, Some(container), _) => ui.context().copy(&container.id),
            (menu::COPY_IP, Some(container), _) => {
                if let Some(ip) = container.ip() {
                    ui.context().copy(ip);
                }
            }
            (menu::COPY_RUN, Some(container), Some(engine)) => self
                .requests
                .push(Request::Containers(Work::CopyRun(engine, container.id))),
            (menu::UP, _, _) => {
                if let Some(compose) = compose {
                    self.compose(compose, Verb::Up);
                }
            }
            (menu::RESTART_PROJECT, _, _) => {
                if let Some(compose) = compose {
                    self.compose(compose, Verb::Restart);
                }
            }
            (menu::DOWN, _, _) => {
                if let Some(compose) = compose {
                    self.confirm_down(ui, compose);
                }
            }
            (menu::DELETE, single, _) if !removable.is_empty() => {
                self.confirm_delete(ui, removable, single.as_ref(), running);
            }
            _ => {}
        }
    }

    fn confirm_delete(
        &mut self,
        ui: &mut Ui,
        ids: Vec<String>,
        single: Option<&Container>,
        running: bool,
    ) {
        let title = match (ids.len(), single) {
            (1, Some(container)) => {
                L10N.tr_args("delete-one", [("name", container.name().to_owned().into())])
            }
            (count, _) => L10N.tr_args("delete-many", [("count", count.into())]),
        };
        let mut message = L10N.tr_args("delete-kept", [("count", ids.len().into())]);
        if running {
            message = format!("{message} {}", L10N.tr("delete-running"));
        }
        let intents = Rc::clone(&self.intents);
        ui.present_sheet(
            Alert::new(&title, &message, move |choice| {
                if choice == "delete" {
                    push(&intents, Intent::Confirmed(Confirm::Delete(ids.clone())));
                }
            })
            .destructive(&L10N.tr("button-delete"), "delete")
            .cancel(&L10N.tr("button-cancel"), "cancel"),
        );
    }

    fn confirm_down(&mut self, ui: &mut Ui, compose: Compose) {
        let title = L10N.tr_args("down-title", [("name", compose.project.clone().into())]);
        let intents = Rc::clone(&self.intents);
        ui.present_sheet(
            Alert::new(&title, &L10N.tr("down-kept"), move |choice| {
                if choice == "down" {
                    push(&intents, Intent::Confirmed(Confirm::Down(compose.clone())));
                }
            })
            .destructive(&L10N.tr("button-down"), "down")
            .cancel(&L10N.tr("button-cancel"), "cancel"),
        );
    }

    pub fn menu(&self) -> Vec<Entry> {
        menu::items(&self.model.borrow().facts(&self.selected))
            .into_iter()
            .map(menu::entry)
            .collect()
    }

    pub fn inspector_entry(&self) -> Entry {
        let label = match self.inspector {
            true => "hide-inspector",
            false => "show-inspector",
        };
        Entry::item(&L10N.tr(label), menu::INSPECTOR).shortcut("Ctrl+I")
    }

    pub fn tools(&mut self, palette: &Palette) -> Vec<Box<dyn Widget>> {
        let button = |svg: &str, label: &str, command: &'static str| {
            let intents = Rc::clone(&self.intents);
            let click = move || push(&intents, Intent::Command(command));
            let label = L10N.tr(label);
            match symbol(svg, palette.ink) {
                Some(image) => Button::symbol(image, &label, click).tooltip(&label),
                None => Button::new(&label, click),
            }
        };
        let start = button(START_SYMBOL, "container-start", "container-start");
        let stop = button(STOP_SYMBOL, "container-stop", "container-stop");
        let restart = button(RESTART_SYMBOL, "container-restart", "container-restart");
        let delete = button(DELETE_SYMBOL, "container-delete", menu::DELETE);
        let inspector = button(INSPECTOR_SYMBOL, "toggle-inspector", menu::INSPECTOR);
        let intents = Rc::clone(&self.intents);
        let search = SearchField::new(&L10N.tr("filter-containers"), &self.filter, move |text| {
            push(&intents, Intent::Filter(text.to_owned()));
        });
        self.tools = Some(Tools {
            start: start.id(),
            stop: stop.id(),
            restart: restart.id(),
            delete: delete.id(),
            search: search.id(),
        });
        vec![
            boxed(start),
            boxed(stop),
            boxed(restart),
            boxed(delete),
            boxed(inspector),
            boxed(search),
        ]
    }

    pub fn drop_tools(&mut self) {
        self.tools = None;
    }

    pub fn has_tools(&self) -> bool {
        self.tools.is_some()
    }

    pub fn content(&mut self) -> Box<dyn Widget> {
        let split = Split::new(boxed(Label::new("")), inspector::WIDTH);
        self.split = Some(split.id());
        self.main = None;
        self.subject = None;
        boxed(split)
    }

    pub fn leave(&mut self) {
        self.split = None;
        self.main = None;
        self.selected.clear();
    }

    fn make_table(&self) -> PortLinks {
        let select = Rc::clone(&self.intents);
        let expand = Rc::clone(&self.intents);
        let activate = Rc::clone(&self.intents);
        let chosen = Rc::clone(&self.intents);
        let open = Rc::clone(&self.intents);
        let model = Rc::clone(&self.model);
        let table = Table::new(table::columns(self.inspector))
            .on_selection(move |keys| push(&select, Intent::Select(keys.to_vec())))
            .on_expand(move |keys, expanded, _| {
                push(&expand, Intent::Expand(keys.to_vec(), expanded));
            })
            .on_activate(move |keys| push(&activate, Intent::Activate(keys.to_vec())))
            .context_menu(
                move |keys| menu::context(menu::items(&model.borrow().facts(keys))),
                move |command, keys| {
                    push(&chosen, Intent::Menu(command.to_owned(), keys.to_vec()));
                },
            );
        PortLinks::new(table, table::ports_column(self.inspector), move |port| {
            push(&open, Intent::Open(menu::url(port)));
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

    pub fn refresh(&mut self, ui: &mut Ui, palette: &Palette) {
        let Some(split) = self.split else {
            return;
        };
        let empty = self.model.borrow().list.is_empty();
        let main = match (empty, self.main) {
            (true, Some(Main::Empty)) | (false, Some(Main::Table { .. })) => None,
            (true, _) => Some((Self::empty_page(), Main::Empty)),
            (false, _) => {
                let links = self.make_table();
                let table = links.children()[0].id();
                let id = links.id();
                let compact = self.inspector;
                Some((
                    boxed(links),
                    Main::Table {
                        links: id,
                        table,
                        compact,
                    },
                ))
            }
        };
        if let Some((widget, shown)) = main {
            if let Some(split) = ui.widget_as::<Split>(split) {
                split.set_main(widget);
            }
            self.main = Some(shown);
            if shown == Main::Empty {
                self.selected.clear();
            }
            self.subject = None;
        }
        self.fill_table(ui, palette);
        self.update_tools(ui);
        self.update_inspector(ui, split);
    }

    fn fill_table(&mut self, ui: &mut Ui, palette: &Palette) {
        let Some(Main::Table {
            links,
            table,
            compact,
        }) = self.main
        else {
            return;
        };
        if compact != self.inspector {
            if let Some(table) = ui.widget_as::<Table>(table) {
                table.set_columns(table::columns(self.inspector));
            }
            if let Some(links) = ui.widget_as::<PortLinks>(links) {
                links.set_column(table::ports_column(self.inspector));
            }
            self.main = Some(Main::Table {
                links,
                table,
                compact: self.inspector,
            });
        }
        let model = self.model.borrow();
        let listing = Shown {
            palette,
            containers: &model.list,
            pending: &model.pending,
            usage: &self.usage,
            collapsed: &self.collapsed,
            filter: &self.filter,
            compact: self.inspector,
        }
        .listing();
        drop(model);
        if let Some(table) = ui.widget_as::<Table>(table) {
            table.set_rows(listing.rows);
            table.set_expanded_rows(listing.expanded);
        }
        if let Some(links) = ui.widget_as::<PortLinks>(links) {
            links.set_ports(listing.ports);
        }
        self.order = listing.order;
    }

    fn update_tools(&mut self, ui: &mut Ui) {
        let Some(tools) = self.tools else {
            return;
        };
        let model = self.model.borrow();
        let can = |action| !model.targets(&self.selected, action).is_empty();
        let states = [
            (tools.start, can(Action::Start)),
            (tools.stop, can(Action::Stop)),
            (tools.restart, can(Action::Restart)),
            (tools.delete, can(Action::Remove)),
        ];
        drop(model);
        for (id, enabled) in states {
            if let Some(button) = ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }

    fn update_inspector(&mut self, ui: &mut Ui, split: WidgetId) {
        let shown = self.inspector && matches!(self.main, Some(Main::Table { .. }));
        if !shown {
            self.graphs = None;
            self.subject = None;
            if let Some(split) = ui.widget_as::<Split>(split)
                && split.has_side()
            {
                split.set_side(None);
            }
            return;
        }
        if self.subject.as_ref() == Some(&self.selected) {
            return;
        }
        let model = self.model.borrow();
        let single = model.single(&self.selected);
        if self.inspected.as_ref().map(|shown| &shown.id) != single.map(|container| &container.id) {
            self.inspected = None;
            if let (Some(container), Some(engine)) = (single, self.engine_of(&model)) {
                self.requests.push(Request::Containers(Work::Inspect(
                    engine,
                    container.id.clone(),
                )));
            }
        }
        let subject = match (single, model.project(&self.selected)) {
            (Some(container), _) => Subject::Container {
                container,
                inspect: self.inspected.as_deref(),
            },
            (None, Some(project)) => {
                let members = model.members(project);
                Subject::Project {
                    name: project,
                    running: members.iter().filter(|member| member.running()).count(),
                    total: members.len(),
                    directory: members
                        .iter()
                        .find_map(|member| member.label("com.docker.compose.project.working_dir")),
                }
            }
            (None, None) => match self.selected.len() {
                0 => Subject::Nothing,
                count => Subject::Many(count),
            },
        };
        let usage = single.and_then(|container| self.usage.get(&container.id));
        let (widget, graphs) = inspector::build(subject, usage, &self.intents);
        drop(model);
        self.graphs = graphs;
        self.subject = Some(self.selected.clone());
        if let Some(split) = ui.widget_as::<Split>(split) {
            split.set_side(Some(widget));
        }
    }

    pub fn sync(&mut self, ui: &Ui) {
        let mut wanted: Vec<String> = Vec::new();
        let model = self.model.borrow();
        if !self.hidden {
            if let Some(Main::Table { links, .. }) = self.main
                && let Some(links) = ui.widget_ref::<PortLinks>(links)
            {
                wanted.extend(
                    links
                        .visible()
                        .iter()
                        .filter_map(|index| self.order.get(*index))
                        .filter(|key| model.list.get(*key).is_some_and(Container::running))
                        .cloned(),
                );
            }
            if self.graphs.is_some()
                && let Some(container) =
                    model.single(&self.selected).filter(|shown| shown.running())
                && !wanted.contains(&container.id)
            {
                wanted.push(container.id.clone());
            }
        }
        wanted.sort();
        if self.watched.as_ref() == Some(&wanted) {
            return;
        }
        let Some(engine) = self.engine_of(&model) else {
            return;
        };
        drop(model);
        self.usage.retain(|id, _| wanted.contains(id));
        self.watched = Some(wanted.clone());
        self.requests
            .push(Request::Containers(Work::Watch(engine, wanted)));
    }

    pub fn sample(&mut self, ui: &mut Ui, id: &str, sample: Sample) {
        if !self
            .watched
            .as_ref()
            .is_some_and(|watched| watched.iter().any(|watched| watched == id))
        {
            return;
        }
        let usage = self.usage.entry(id.to_owned()).or_default();
        usage.push(sample);
        if let Some(ids) = self.graphs
            && self.inspected.as_ref().is_some_and(|shown| shown.id == id)
        {
            inspector::update(ui, ids, usage.graphs());
        }
    }

    pub fn list(&self) -> Ref<'_, BTreeMap<String, Container>> {
        Ref::map(self.model.borrow(), |model| &model.list)
    }
}

fn failure_title(action: Action, name: &str) -> String {
    let key = match action {
        Action::Start => "cannot-start",
        Action::Stop => "cannot-stop",
        Action::Restart => "cannot-restart",
        Action::Pause => "cannot-pause",
        Action::Resume => "cannot-resume",
        Action::Kill => "cannot-kill",
        Action::Remove => "cannot-delete",
    };
    L10N.tr_args(key, [("name", name.to_owned().into())])
}

fn alert(ui: &mut Ui, title: &str, message: &str) {
    ui.present_sheet(Alert::new(title, message, |_| {}).primary(&L10N.tr("button-ok"), "ok"));
}
