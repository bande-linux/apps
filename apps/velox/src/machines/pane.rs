use std::cell::RefCell;
use std::rc::Rc;

use bande_app_menu::Entry;
use bande_l10n::{Text, text, tr};
use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Button, Cell, Form, Heading, Menu, MenuItem, Row, ScrollView, SearchField, Table, Ui,
    Widget, WidgetId,
};

use super::backend::Act;
use super::fleet::{Fleet, Status};
use super::present::{self, JOB_PREFIX};
use super::readiness::Blocker;
use super::sheet::{BACK, CANCEL, CONTINUE, Draft, Step};
use super::widgets::{Slot, Split};
use super::{Intent, Intents, MACHINES_SYMBOL, Request, push};
use crate::ui::symbol;

const START_SYMBOL: &str = include_str!("../../data/symbols/start.svg");
const STOP_SYMBOL: &str = include_str!("../../data/symbols/stop.svg");
const NEW_SYMBOL: &str = include_str!("../../data/symbols/add.svg");
const DELETE_SYMBOL: &str = include_str!("../../data/symbols/delete.svg");
const INSPECTOR_SYMBOL: &str = include_str!("../../data/symbols/inspector.svg");
const INSPECTOR_WIDTH: f32 = 280.0;
pub const MACHINES_KEY: &str = "machines";
pub const INSPECTOR_COMMAND: &str = "machine-inspector";
const MENU_START: &str = "machine-start";
const MENU_SHUT_DOWN: &str = "machine-shut-down";
const MENU_FORCE_STOP: &str = "machine-force-stop";
const MENU_RESTART: &str = "machine-restart";
const MENU_CONNECT: &str = "machine-connect";
const MENU_OPEN: &str = "machine-open";
const MENU_COPY_SSH: &str = "machine-copy-ssh";
const MENU_DELETE: &str = "machine-delete";
const MENU_PAUSE: &str = "machine-pause";
const MENU_RESUME: &str = "machine-resume";
const MENU_SUSPEND: &str = "machine-suspend";
const MENU_SNAPSHOT: &str = "machine-take-snapshot";
const MENU_CLONE: &str = "machine-clone";
const MENU_SERIAL: &str = "machine-serial";
const MENU_CANCEL: &str = "machine-cancel-job";
const CONFIRM_DELETE: &str = "delete";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    Nothing,
    Page(Blocker),
    Empty,
    Table,
}

#[derive(Debug, Clone, Copy)]
struct Tools {
    new: WidgetId,
    start: WidgetId,
    stop: WidgetId,
    delete: WidgetId,
    search: WidgetId,
}

pub struct MachinesPane {
    fleet: Rc<RefCell<Fleet>>,
    intents: Intents,
    requests: Vec<Request>,
    palette: Palette,
    slot: Option<WidgetId>,
    split: Option<WidgetId>,
    table: Option<WidgetId>,
    tools: Option<Tools>,
    shown: Shown,
    selected: Vec<String>,
    filter: String,
    inspector: bool,
    draft: Option<Draft>,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn menu_item(key: Text, action: &str, enabled: bool) -> MenuItem {
    MenuItem::new(&key.tr(), action).enabled(enabled)
}

impl MachinesPane {
    pub(crate) fn new(fleet: Rc<RefCell<Fleet>>, palette: Palette) -> Self {
        Self {
            fleet,
            intents: Intents::default(),
            requests: Vec::new(),
            palette,
            slot: None,
            split: None,
            table: None,
            tools: None,
            shown: Shown::Nothing,
            selected: Vec::new(),
            filter: String::new(),
            inspector: false,
            draft: None,
        }
    }

    pub(crate) fn set_fleet(&mut self, fleet: Rc<RefCell<Fleet>>) {
        self.fleet = fleet;
        self.shown = Shown::Nothing;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn title(&self) -> String {
        tr!("Machines")
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn sidebar_section(&self, palette: &Palette) -> (Heading, Vec<Row>) {
        let fleet = self.fleet.borrow();
        let count = match fleet.ready() {
            true => format!("{}/{}", fleet.running(), fleet.entries.len()),
            false => String::new(),
        };
        (
            Heading::new(0, tr!("Machines")),
            vec![Row::new(
                MACHINES_KEY,
                vec![
                    Cell::new(tr!("Machines")).icon(symbol(MACHINES_SYMBOL, palette.accent)),
                    Cell::new(count).muted().tabular(),
                ],
            )],
        )
    }

    pub fn show(&mut self) -> (Box<dyn Widget>, Vec<Box<dyn Widget>>) {
        let slot = Slot::new(super::page::blank());
        self.slot = Some(slot.id());
        self.shown = Shown::Nothing;
        self.table = None;
        self.split = None;
        let (tools, widgets) = self.make_tools();
        self.tools = Some(tools);
        (boxed(slot), widgets)
    }

    pub fn hide(&mut self) {
        self.slot = None;
        self.split = None;
        self.table = None;
        self.tools = None;
        self.shown = Shown::Nothing;
    }

    pub fn shown(&self) -> bool {
        self.slot.is_some()
    }

    fn make_tools(&self) -> (Tools, Vec<Box<dyn Widget>>) {
        let button = |svg: &str, label: Text, intent: Intent| {
            let intents = Rc::clone(&self.intents);
            let click = move || push(&intents, intent.clone());
            let label = label.tr();
            match symbol(svg, self.palette.ink) {
                Some(image) => Button::symbol(image, &label, click).tooltip(&label),
                None => Button::new(&label, click),
            }
        };
        let new = button(NEW_SYMBOL, text!("New Virtual Machine…"), Intent::New);
        let start = button(START_SYMBOL, text!("Start"), Intent::Act(Act::Start));
        let stop = button(STOP_SYMBOL, text!("Shut Down"), Intent::Act(Act::ShutDown));
        let delete = button(DELETE_SYMBOL, text!("Delete"), Intent::Act(Act::Delete));
        let inspector = button(INSPECTOR_SYMBOL, text!("Show Inspector"), Intent::Inspector);
        let intents = Rc::clone(&self.intents);
        let search = SearchField::new(&tr!("Filter machines"), &self.filter, move |text| {
            push(&intents, Intent::Filter(text.to_owned()));
        });
        let tools = Tools {
            new: new.id(),
            start: start.id(),
            stop: stop.id(),
            delete: delete.id(),
            search: search.id(),
        };
        (
            tools,
            vec![
                boxed(new),
                boxed(start),
                boxed(stop),
                boxed(delete),
                boxed(inspector),
                boxed(search),
            ],
        )
    }

    fn selected_ids(&self) -> Vec<String> {
        let fleet = self.fleet.borrow();
        self.selected
            .iter()
            .filter(|key| fleet.entries.contains_key(*key))
            .cloned()
            .collect()
    }

    fn targets(&self, keys: &[String], act: Act) -> Vec<String> {
        let fleet = self.fleet.borrow();
        keys.iter()
            .filter(|key| fleet.entry(key).is_some_and(|entry| entry.can(act)))
            .cloned()
            .collect()
    }

    pub fn can(&self, act: Act) -> bool {
        let targets = self.targets(&self.selected_ids(), act);
        match act {
            Act::Delete => targets.len() == 1,
            _ => !targets.is_empty(),
        }
    }

    pub fn single_selected(&self) -> Option<String> {
        let [id] = self.selected_ids().try_into().ok()?;
        Some(id)
    }

    pub fn single_can(&self, act: Act) -> bool {
        let [id] = self.selected_ids().try_into().unwrap_or_default();
        !id.is_empty()
            && self
                .fleet
                .borrow()
                .entry(&id)
                .is_some_and(|entry| entry.can(act))
    }

    pub fn present_for_selected(&mut self, make: fn(String) -> Request) {
        if let [id] = self.selected_ids().as_slice() {
            self.requests.push(make(id.clone()));
        }
    }

    pub fn can_connect(&self) -> bool {
        let fleet = self.fleet.borrow();
        let ids = self.selected_ids();
        ids.len() == 1
            && ids
                .first()
                .and_then(|id| fleet.entry(id))
                .is_some_and(|entry| entry.running() && entry.ssh_target().is_some())
    }

    pub fn can_create(&self) -> bool {
        self.fleet.borrow().ready()
    }

    pub fn act(&mut self, ui: &mut Ui, act: Act) {
        let keys = self.selected_ids();
        self.act_on(ui, &keys, act);
    }

    fn act_on(&mut self, ui: &mut Ui, keys: &[String], act: Act) {
        let targets = self.targets(keys, act);
        if act == Act::Delete {
            if let [id] = targets.as_slice() {
                self.confirm_delete(ui, id);
            }
            return;
        }
        for id in targets {
            self.fleet.borrow_mut().mark(&id, act);
            self.requests.push(Request::Act(id, act));
        }
    }

    pub fn connect(&mut self) {
        if let [id] = self.selected_ids().as_slice() {
            self.requests.push(Request::Connect(id.clone()));
        }
    }

    fn confirm_delete(&mut self, ui: &mut Ui, id: &str) {
        let name = self
            .fleet
            .borrow()
            .entry(id)
            .map(|entry| entry.name().to_owned())
            .unwrap_or_default();
        let intents = Rc::clone(&self.intents);
        let id = id.to_owned();
        ui.present_sheet(
            Alert::new(
                &tr!("Delete “{name}”?", name = name),
                &tr!(
                    "Its disk, firmware settings and snapshots are deleted. This can’t be undone."
                ),
                move |action| {
                    push(
                        &intents,
                        Intent::Delete(id.clone(), action == CONFIRM_DELETE),
                    )
                },
            )
            .cancel(&tr!("Cancel"), CANCEL)
            .destructive(&tr!("Delete"), CONFIRM_DELETE),
        );
    }

    pub fn new_machine(&mut self, ui: &mut Ui) {
        if !self.can_create() {
            return;
        }
        let draft = Draft::new(&self.fleet.borrow());
        ui.present_sheet(draft.alert(&self.intents, None));
        self.draft = Some(draft);
    }

    pub fn toggle_inspector(&mut self, ui: &mut Ui) {
        self.inspector = !self.inspector;
        self.refresh(ui);
    }

    pub fn inspector_entry(&self) -> Entry {
        let label = match self.inspector {
            true => text!("Hide Inspector"),
            false => text!("Show Inspector"),
        };
        Entry::item(&label.tr(), INSPECTOR_COMMAND).shortcut("Ctrl+I")
    }

    pub fn focus_filter(&self, ui: &mut Ui) {
        if let Some(tools) = self.tools {
            ui.set_focus(tools.search);
        }
    }

    fn sheet_choice(&mut self, ui: &mut Ui, choice: &str) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        let fleet = Rc::clone(&self.fleet);
        let mut error = None;
        match choice {
            CONTINUE if draft.step == Step::Identity => match draft.plan(&fleet.borrow()) {
                Ok(plan) => {
                    self.requests.push(Request::Create(Box::new(plan)));
                    return;
                }
                Err(message) => error = Some(message),
            },
            CONTINUE => draft.advance(&fleet.borrow()),
            BACK => draft.back(),
            _ => return,
        }
        ui.present_sheet(draft.alert(&self.intents, error.as_deref()));
        self.draft = Some(draft);
    }

    fn menu(fleet: &Fleet, keys: &[String]) -> Option<Menu> {
        if let [key] = keys
            && key.starts_with(JOB_PREFIX)
        {
            return Some(Menu::new(vec![menu_item(
                text!("Cancel Download"),
                MENU_CANCEL,
                true,
            )]));
        }
        let entries: Vec<_> = keys.iter().filter_map(|key| fleet.entry(key)).collect();
        let first = entries.first()?;
        let any = |act: Act| entries.iter().any(|entry| entry.can(act));
        let single = entries.len() == 1;
        let ssh = single && first.running() && first.ssh_target().is_some();
        let mut items = vec![
            menu_item(text!("Start"), MENU_START, any(Act::Start)),
            menu_item(text!("Shut Down"), MENU_SHUT_DOWN, any(Act::ShutDown)),
            menu_item(text!("Force Stop"), MENU_FORCE_STOP, any(Act::ForceStop)),
            menu_item(text!("Restart"), MENU_RESTART, any(Act::Restart)),
            match any(Act::Resume) {
                true => menu_item(text!("Resume"), MENU_RESUME, true),
                false => menu_item(text!("Pause"), MENU_PAUSE, any(Act::Pause)),
            },
            menu_item(text!("Suspend"), MENU_SUSPEND, any(Act::Suspend)),
            MenuItem::separator(),
        ];
        if bande_xdg::is_installed(crate::TERMINAL_APP_ID) {
            items.push(menu_item(text!("Connect via SSH"), MENU_CONNECT, ssh));
        }
        items.extend([
            menu_item(text!("Show Machine Window"), MENU_OPEN, single),
            menu_item(
                text!("Serial Console"),
                MENU_SERIAL,
                single && first.running(),
            ),
            menu_item(
                text!("Take Snapshot…"),
                MENU_SNAPSHOT,
                single && first.can(Act::TakeSnapshot),
            ),
            menu_item(text!("Clone…"), MENU_CLONE, single && first.can(Act::Clone)),
            menu_item(
                text!("Copy SSH Command"),
                MENU_COPY_SSH,
                single && first.ssh_target().is_some(),
            ),
            MenuItem::separator(),
            menu_item(
                text!("Delete"),
                MENU_DELETE,
                single && first.can(Act::Delete),
            ),
        ]);
        Some(Menu::new(items))
    }

    fn menu_action(&mut self, ui: &mut Ui, action: &str, keys: &[String]) {
        let act = match action {
            MENU_START => Some(Act::Start),
            MENU_SHUT_DOWN => Some(Act::ShutDown),
            MENU_FORCE_STOP => Some(Act::ForceStop),
            MENU_RESTART => Some(Act::Restart),
            MENU_DELETE => Some(Act::Delete),
            MENU_PAUSE => Some(Act::Pause),
            MENU_RESUME => Some(Act::Resume),
            MENU_SUSPEND => Some(Act::Suspend),
            _ => None,
        };
        if let Some(act) = act {
            return self.act_on(ui, keys, act);
        }
        let Some(key) = keys.first().cloned() else {
            return;
        };
        match action {
            MENU_CONNECT => self.requests.push(Request::Connect(key)),
            MENU_OPEN => self.requests.push(Request::Open(key)),
            MENU_SNAPSHOT => self.requests.push(Request::PresentSnapshot(key)),
            MENU_CLONE => self.requests.push(Request::PresentClone(key)),
            MENU_SERIAL => self.requests.push(Request::Serial(key)),
            MENU_COPY_SSH => {
                let command = self.fleet.borrow().entry(&key).and_then(|entry| {
                    let (host, port) = entry.ssh_target()?;
                    Some(present::short_ssh(&entry.listed.login.user, &host, port))
                });
                if let Some(command) = command {
                    ui.context().copy(&command);
                }
            }
            MENU_CANCEL => {
                if let Some(job) = key
                    .strip_prefix(JOB_PREFIX)
                    .and_then(|job| job.parse().ok())
                {
                    self.requests.push(Request::Cancel(job));
                }
            }
            _ => {}
        }
    }

    pub fn pump(&mut self, ui: &mut Ui) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if intents.is_empty() {
            return false;
        }
        for intent in intents {
            match intent {
                Intent::Select(keys) => {
                    self.selected = keys;
                }
                Intent::Activate(keys) => {
                    let fleet = self.fleet.borrow();
                    for key in keys
                        .into_iter()
                        .filter(|key| fleet.entries.contains_key(key))
                    {
                        self.requests.push(Request::Open(key));
                    }
                }
                Intent::Filter(text) => self.filter = text,
                Intent::Act(act) => self.act(ui, act),
                Intent::New => self.new_machine(ui),
                Intent::Inspector => {
                    self.inspector = !self.inspector;
                }
                Intent::Menu(action, keys) => self.menu_action(ui, &action, &keys),
                Intent::Emulate => self.requests.push(Request::Emulate),
                Intent::Copy(text) => ui.context().copy(text),
                Intent::Field(field) => {
                    if let Some(draft) = &mut self.draft {
                        draft.set(field);
                    }
                }
                Intent::Sheet(choice) => self.sheet_choice(ui, &choice),
                Intent::Delete(id, true) => {
                    self.fleet.borrow_mut().mark(&id, Act::Delete);
                    self.requests.push(Request::Act(id, Act::Delete));
                }
                Intent::Delete(_, false)
                | Intent::Tab(_)
                | Intent::Change(_)
                | Intent::Connect
                | Intent::RetryConsole
                | Intent::Choice(_)
                | Intent::Snapshot(_)
                | Intent::SnapshotTool(_)
                | Intent::Text(_)
                | Intent::Port(_)
                | Intent::Linked(_)
                | Intent::Eject
                | Intent::Graphics3d(_)
                | Intent::StartAtLogin(_)
                | Intent::AddShare
                | Intent::RemoveShare(_) => {}
                Intent::ChooseIso(disk) => self.requests.push(Request::ChooseIso(disk)),
            }
        }
        self.refresh(ui);
        true
    }

    fn make_table(&self) -> Table {
        let select = Rc::clone(&self.intents);
        let activate = Rc::clone(&self.intents);
        let menu = Rc::clone(&self.intents);
        let fleet = Rc::clone(&self.fleet);
        Table::new(present::columns())
            .on_selection(move |keys| push(&select, Intent::Select(keys.to_vec())))
            .on_activate(move |keys| push(&activate, Intent::Activate(keys.to_vec())))
            .context_menu(
                move |keys| Self::menu(&fleet.borrow(), keys),
                move |action, keys| push(&menu, Intent::Menu(action.to_owned(), keys.to_vec())),
            )
    }

    fn inspector_form(&self) -> Option<Box<dyn Widget>> {
        let fleet = self.fleet.borrow();
        let accel = match fleet.status {
            Status::Ready(accel) => Some(accel),
            _ => None,
        };
        let [id] = self.selected.as_slice() else {
            return Some(super::page::blank());
        };
        let entry = fleet.entry(id)?;
        Some(boxed(ScrollView::new(Form::new(
            &tr!("Inspector"),
            present::info_lines(entry, accel),
        ))))
    }

    fn pick_up_iso(&mut self, ui: &mut Ui) {
        let chosen = self.fleet.borrow_mut().chosen_iso.take();
        if let (Some(path), Some(draft)) = (chosen, &mut self.draft) {
            draft.set_iso(path);
            ui.present_sheet(draft.alert(&self.intents, None));
        }
    }

    pub fn refresh(&mut self, ui: &mut Ui) {
        self.pick_up_iso(ui);
        let Some(slot) = self.slot else {
            return;
        };
        let status = self.fleet.borrow().status.clone();
        let wanted = match status {
            Status::Probing => Shown::Nothing,
            Status::Blocked(blocker) => Shown::Page(blocker),
            Status::Ready(_) => {
                let fleet = self.fleet.borrow();
                match fleet.entries.is_empty() && fleet.jobs.is_empty() {
                    true => Shown::Empty,
                    false => Shown::Table,
                }
            }
        };
        if wanted != self.shown {
            let content = match &wanted {
                Shown::Nothing => super::page::blank(),
                Shown::Page(blocker) => super::page::not_ready(blocker, &self.intents),
                Shown::Empty => super::page::empty(&self.intents),
                Shown::Table => {
                    let table = self.make_table();
                    self.table = Some(table.id());
                    let split = Split::new(boxed(table), INSPECTOR_WIDTH);
                    self.split = Some(split.id());
                    boxed(split)
                }
            };
            if wanted != Shown::Table {
                self.table = None;
                self.split = None;
                self.selected.clear();
            }
            if let Some(slot) = ui.widget_as::<Slot>(slot) {
                slot.set(content);
            }
            self.shown = wanted;
        }
        self.fill(ui);
        self.update_tools(ui);
    }

    fn fill(&mut self, ui: &mut Ui) {
        let Some(table) = self.table else {
            return;
        };
        let rows = present::rows(&self.palette, &self.fleet.borrow(), &self.filter);
        let focus = self.fleet.borrow_mut().focus.take();
        if let Some(table) = ui.widget_as::<Table>(table) {
            table.set_rows(rows);
            match focus {
                Some(key) if table.selection().len() <= 1 => {
                    table.select(std::slice::from_ref(&key));
                    if table.selection() == [key.clone()] {
                        self.selected = vec![key];
                    } else {
                        self.fleet.borrow_mut().focus = Some(key);
                    }
                }
                Some(key) => self.fleet.borrow_mut().focus = Some(key),
                None => {}
            }
        }
        let Some(split) = self.split else {
            return;
        };
        let side = match self.inspector {
            true => self.inspector_form(),
            false => None,
        };
        if let Some(split) = ui.widget_as::<Split>(split) {
            split.set_side(side);
        }
    }

    fn update_tools(&mut self, ui: &mut Ui) {
        let Some(tools) = self.tools else {
            return;
        };
        let states = [
            (tools.new, self.can_create()),
            (tools.start, self.can(Act::Start)),
            (tools.stop, self.can(Act::ShutDown)),
            (tools.delete, self.can(Act::Delete)),
        ];
        for (id, enabled) in states {
            if let Some(button) = ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }
}
