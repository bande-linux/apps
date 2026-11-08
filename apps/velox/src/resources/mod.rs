mod layout;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use bande_app_menu::Entry;
use bande_l10n::{Text, tr, trn};
use bande_toolkit::Palette;
use bande_toolkit::ui::{Alert, Button, Label, SearchField, Ui, Widget, WidgetId};

pub use self::layout::{Banded, Field, Fields, Strip};
use crate::engine::{
    Container, DiskUsage, EngineInfo, Image, Network, Pulling, Task, Update, Volume,
};
use crate::machines::Fleet;
use crate::ui::{Section, symbol};
use crate::{images, networks, overview, volumes};

pub type Acts = Rc<RefCell<Vec<Act>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Navigate(Section),
    Select(Vec<String>),
    Expand(Vec<String>, bool),
    Filter(String),
    Text(String),
    Command(&'static str),
    Menu(String, Vec<String>),
    Task(Task),
}

pub fn push(acts: &Acts, act: Act) {
    acts.borrow_mut().push(act);
}

pub fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

pub fn tool(svg: &str, label: Text, palette: &Palette, acts: &Acts, act: Act) -> Button {
    let acts = Rc::clone(acts);
    let click = move || push(&acts, act.clone());
    let label = label.tr();
    match symbol(svg, palette.ink) {
        Some(image) => Button::symbol(image, &label, click).tooltip(&label),
        None => Button::new(&label, click),
    }
}

pub fn search(placeholder: Text, text: &str, acts: &Acts) -> SearchField {
    let acts = Rc::clone(acts);
    SearchField::new(&placeholder.tr(), text, move |text| {
        push(&acts, Act::Filter(text.to_owned()));
    })
}

pub fn confirm(ui: &mut Ui, acts: &Acts, title: &str, message: &str, button: &str, task: Task) {
    let acts = Rc::clone(acts);
    ui.present_sheet(
        Alert::new(title, message, move |choice| {
            if choice == "go" {
                push(&acts, Act::Task(task.clone()));
            }
        })
        .destructive(button, "go")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}

pub fn tell(ui: &mut Ui, title: &str, message: &str) {
    ui.present_sheet(Alert::new(title, message, |_| {}).primary(&tr!("OK"), "ok"));
}

pub fn matches(filter: &str, texts: &[&str]) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || texts
            .iter()
            .any(|text| text.to_lowercase().contains(&filter))
}

pub fn age(seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    let elapsed = (now - seconds).max(0);
    match elapsed {
        elapsed if elapsed >= 365 * 86_400 => {
            trn!(
                "1 year ago",
                "{count} years ago",
                count = elapsed / (365 * 86_400)
            )
        }
        elapsed if elapsed >= 30 * 86_400 => {
            trn!(
                "1 month ago",
                "{count} months ago",
                count = elapsed / (30 * 86_400)
            )
        }
        elapsed if elapsed >= 7 * 86_400 => {
            trn!(
                "1 week ago",
                "{count} weeks ago",
                count = elapsed / (7 * 86_400)
            )
        }
        elapsed if elapsed >= 86_400 => {
            trn!("1 day ago", "{count} days ago", count = elapsed / 86_400)
        }
        elapsed if elapsed >= 3_600 => {
            trn!("1 hour ago", "{count} hours ago", count = elapsed / 3_600)
        }
        elapsed if elapsed >= 60 => {
            trn!("1 minute ago", "{count} minutes ago", count = elapsed / 60)
        }
        _ => tr!("Just now"),
    }
}

pub fn file_menu(ready: bool) -> Vec<Entry> {
    vec![
        Entry::item(&tr!("Pull Image…"), images::PULL).enabled(ready),
        Entry::item(&tr!("Run Image…"), images::RUN).enabled(ready),
        Entry::separator(),
        Entry::item(&tr!("Reclaim Space…"), overview::RECLAIM).enabled(ready),
    ]
}

pub fn size(bytes: u64) -> String {
    bande_fs::format_size(bytes)
}

#[derive(Default)]
pub struct Store {
    pub images: Vec<Image>,
    pub volumes: Vec<Volume>,
    pub networks: Vec<Network>,
    pub usage: Option<DiskUsage>,
    pub pulling: Option<(String, Pulling)>,
}

pub struct Scope<'a> {
    pub store: &'a Store,
    pub containers: &'a BTreeMap<String, Container>,
    pub palette: &'a Palette,
    pub info: &'a EngineInfo,
}

pub trait Pane {
    fn build(&mut self, acts: &Acts, palette: &Palette) -> (Vec<Box<dyn Widget>>, Box<dyn Widget>);
    fn fill(&mut self, ui: &mut Ui, scope: &Scope);
    fn act(&mut self, _act: Act) {}
    fn search(&self) -> Option<WidgetId> {
        None
    }
    fn menu(&self, _scope: &Scope) -> Option<Entry> {
        None
    }
    fn command(&mut self, ui: &mut Ui, acts: &Acts, scope: &Scope, command: &str) -> bool;
    fn menu_action(
        &mut self,
        ui: &mut Ui,
        acts: &Acts,
        scope: &Scope,
        action: &str,
        keys: &[String],
    );
    fn finished(&mut self, _ui: &mut Ui, _task: &Task, _result: &Result<u64, String>) -> bool {
        false
    }
}

#[derive(Default)]
struct Demand {
    busy: bool,
    again: bool,
}

pub struct Resources {
    store: Store,
    acts: Acts,
    shown: Option<Section>,
    images: images::Images,
    volumes: volumes::Volumes,
    networks: networks::Networks,
    overview: overview::Overview,
    tasks: Vec<Task>,
    usage: Demand,
    watched: Vec<String>,
    watch: Option<Vec<String>>,
    navigate: Option<Section>,
    reclaim: bool,
    hidden: bool,
}

impl Default for Resources {
    fn default() -> Self {
        Self::new()
    }
}

impl Resources {
    pub fn new() -> Self {
        Self {
            store: Store::default(),
            acts: Acts::default(),
            shown: None,
            images: images::Images::default(),
            volumes: volumes::Volumes::default(),
            networks: networks::Networks::default(),
            overview: overview::Overview::default(),
            tasks: Vec::new(),
            usage: Demand::default(),
            watched: Vec::new(),
            watch: None,
            navigate: None,
            reclaim: false,
            hidden: false,
        }
    }

    fn pane(&mut self, section: Section) -> Option<&mut dyn Pane> {
        match section {
            Section::Overview => Some(&mut self.overview),
            Section::Images => Some(&mut self.images),
            Section::Volumes => Some(&mut self.volumes),
            Section::Networks => Some(&mut self.networks),
            _ => None,
        }
    }

    pub fn count(&self, section: Section) -> Option<usize> {
        match section {
            Section::Images => Some(self.store.images.len()),
            Section::Volumes => Some(self.store.volumes.len()),
            Section::Networks => Some(self.store.networks.len()),
            _ => None,
        }
    }

    fn wants_usage(&self) -> bool {
        self.reclaim || matches!(self.shown, Some(Section::Overview | Section::Volumes))
    }

    fn need_usage(&mut self) {
        if !self.wants_usage() {
            return;
        }
        match self.usage.busy {
            true => self.usage.again = true,
            false => {
                self.usage.busy = true;
                self.tasks.push(Task::Usage);
            }
        }
    }

    pub fn add_task(&mut self, task: Task) {
        self.tasks.push(task);
    }

    pub fn take_tasks(&mut self) -> Vec<Task> {
        std::mem::take(&mut self.tasks)
    }

    pub fn take_watch(&mut self) -> Option<Vec<String>> {
        self.watch.take()
    }

    pub fn take_navigation(&mut self) -> Option<Section> {
        self.navigate.take()
    }

    pub fn clear(&mut self) {
        self.store = Store::default();
        self.usage = Demand::default();
        self.reclaim = false;
    }

    pub fn apply(&mut self, ui: &mut Ui, update: Update) -> Option<Update> {
        match update {
            Update::Images(images) => {
                self.store.images = images;
                self.need_usage();
            }
            Update::Volumes(volumes) => {
                self.store.volumes = volumes;
                self.need_usage();
            }
            Update::Networks(networks) => self.store.networks = networks,
            Update::Usage(usage) => {
                self.store.usage = Some(usage);
                self.usage.busy = false;
                if std::mem::take(&mut self.usage.again) {
                    self.need_usage();
                }
                if std::mem::take(&mut self.reclaim) {
                    overview::reclaim_sheet(ui, &self.acts, &self.store);
                }
            }
            Update::Pulling(reference, pulling) => {
                self.store.pulling = Some((reference, pulling));
            }
            Update::Chosen(path) => volumes::name_sheet(ui, &self.acts, &path),
            Update::Stats(id, sample) => self.overview.sample(id, sample),
            Update::Finished(task, result) => {
                if matches!(task, Task::Pull(_)) {
                    self.store.pulling = None;
                }
                let handled = [
                    Section::Images,
                    Section::Volumes,
                    Section::Networks,
                    Section::Overview,
                ]
                .into_iter()
                .any(|section| {
                    self.pane(section)
                        .is_some_and(|pane| pane.finished(ui, &task, &result))
                });
                if !handled && let Err(message) = &result {
                    tell(ui, &tr!("Action failed"), message);
                }
                self.need_usage();
            }
            other => return Some(other),
        }
        None
    }

    pub fn pump(
        &mut self,
        ui: &mut Ui,
        containers: &BTreeMap<String, Container>,
        info: Option<&EngineInfo>,
    ) -> bool {
        let acts: Vec<Act> = self.acts.borrow_mut().drain(..).collect();
        if acts.is_empty() {
            return false;
        }
        for act in acts {
            match act {
                Act::Navigate(section) => self.navigate = Some(section),
                Act::Task(task) => {
                    if let Task::Pull(reference) = &task {
                        self.store.pulling = Some((
                            reference.clone(),
                            Pulling {
                                status: tr!("Starting…"),
                                fraction: None,
                            },
                        ));
                    }
                    self.tasks.push(task);
                }
                Act::Command(command) => {
                    if let Some(info) = info {
                        self.command(ui, containers, info, command);
                    }
                }
                Act::Menu(action, keys) => {
                    let (Some(info), Some(section)) = (info, self.shown) else {
                        continue;
                    };
                    let acts = Rc::clone(&self.acts);
                    let store = std::mem::take(&mut self.store);
                    let palette = ui.context().palette;
                    let scope = Scope {
                        store: &store,
                        containers,
                        palette: &palette,
                        info,
                    };
                    if let Some(pane) = self.pane(section) {
                        pane.menu_action(ui, &acts, &scope, &action, &keys);
                    }
                    self.store = store;
                }
                other => {
                    if let Some(pane) = self.shown.and_then(|section| self.pane(section)) {
                        pane.act(other);
                    }
                }
            }
        }
        true
    }

    pub fn command(
        &mut self,
        ui: &mut Ui,
        containers: &BTreeMap<String, Container>,
        info: &EngineInfo,
        command: &str,
    ) -> bool {
        if command == overview::RECLAIM {
            self.reclaim = true;
            self.need_usage();
            return true;
        }
        let Some(section) = self.shown else {
            return false;
        };
        let acts = Rc::clone(&self.acts);
        let store = std::mem::take(&mut self.store);
        let palette = ui.context().palette;
        let scope = Scope {
            store: &store,
            containers,
            palette: &palette,
            info,
        };
        let done = self
            .pane(section)
            .is_some_and(|pane| pane.command(ui, &acts, &scope, command));
        self.store = store;
        done
    }

    pub fn menu(
        &self,
        section: Section,
        containers: &BTreeMap<String, Container>,
        info: Option<&EngineInfo>,
        palette: &Palette,
    ) -> Option<Entry> {
        let info = info?;
        let scope = Scope {
            store: &self.store,
            containers,
            palette,
            info,
        };
        match section {
            Section::Images => self.images.menu(&scope),
            Section::Volumes => self.volumes.menu(&scope),
            Section::Networks => self.networks.menu(&scope),
            _ => None,
        }
    }

    pub fn focus_filter(&mut self, ui: &mut Ui) -> bool {
        let Some(search) = self
            .shown
            .and_then(|section| self.pane(section))
            .and_then(|pane| pane.search())
        else {
            return false;
        };
        ui.set_focus(search);
        true
    }

    pub fn set_fleet(&mut self, fleet: Rc<RefCell<Fleet>>) {
        self.overview.set_fleet(fleet);
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        match self.shown {
            Some(Section::Overview) => self.overview.deadline(),
            _ => None,
        }
    }

    pub fn machines_sampled(&mut self) {
        if self.shown == Some(Section::Overview) {
            self.overview.machines_sampled();
        }
    }

    pub fn hide(&mut self, ui: &mut Ui, root: WidgetId) {
        if self.shown.take().is_none() {
            return;
        }
        if let Some(root) = ui.widget_as::<crate::ui::Root>(root) {
            root.set_tools(Vec::new());
            root.set_content(boxed(Label::new("")));
        }
        self.refresh_watch(&BTreeMap::new());
    }

    pub fn stats_round(&mut self, ui: &mut Ui) -> bool {
        match self.shown {
            Some(Section::Overview) => {
                self.overview.fill_live(ui);
                true
            }
            _ => false,
        }
    }

    pub fn set_hidden(&mut self, hidden: bool, containers: &BTreeMap<String, Container>) {
        self.hidden = hidden;
        self.refresh_watch(containers);
    }

    fn refresh_watch(&mut self, containers: &BTreeMap<String, Container>) {
        let shown = match self.hidden {
            true => None,
            false => self.shown,
        };
        let wanted: Vec<String> = match shown {
            Some(Section::Overview) => containers
                .values()
                .filter(|container| container.running())
                .map(|container| container.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        if wanted != self.watched {
            self.overview.keep(&wanted);
            self.watched.clone_from(&wanted);
            self.watch = Some(wanted);
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        root: WidgetId,
        section: Section,
        containers: &BTreeMap<String, Container>,
        info: &EngineInfo,
    ) {
        let palette = ui.context().palette;
        if self.shown != Some(section) {
            let acts = Rc::clone(&self.acts);
            let Some(pane) = self.pane(section) else {
                return;
            };
            let (tools, content) = pane.build(&acts, &palette);
            if let Some(root) = ui.widget_as::<crate::ui::Root>(root) {
                root.set_tools(tools);
                root.set_content(content);
            }
            self.shown = Some(section);
            self.need_usage();
        }
        self.refresh_watch(containers);
        let store = std::mem::take(&mut self.store);
        let scope = Scope {
            store: &store,
            containers,
            palette: &palette,
            info,
        };
        if let Some(pane) = self.pane(section) {
            pane.fill(ui, &scope);
        }
        self.store = store;
    }

    pub fn rebuild(&mut self) {
        self.shown = None;
    }
}
