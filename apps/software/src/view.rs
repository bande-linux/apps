use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use bande_fs::format_size;
use bande_inbox::Inbox;
use bande_packages::{Canceller, Client, Package, Progress, Status};
use bande_toolkit::ui::{
    Axis, Button, Context, Disclosure, Label, ProgressBar, ScrollState, ScrollView, SearchField,
    SegmentedControl, Spinner, Stack, Ui, Widget, WidgetId,
};
use bande_toolkit::{Image, Palette, Rect, TextSystem};

use crate::L10N;
use crate::catalog::{Block, Bundle, Category, Component, Icon, search};
use crate::media::{self, Loaded, Media, ShotError};
use crate::widgets::{AppRow, Grid, Picture};
use crate::worker::{self, Facts, Failure, Job, Message, Problem, Query};

pub const WINDOW_SIZE: (u32, u32) = (820, 680);
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;
const PAGE_ROWS: usize = 40;
const NEW_ROWS: usize = 12;
const ROW_ICON: f32 = 40.0;
const PAGE_ICON: f32 = 80.0;
const SHOT_WIDTH: f32 = 640.0;
const SHOT_HEIGHT: f32 = 400.0;
const LISTED_UPDATES: usize = 30;
const SETTINGS: &str = "bande-settings";
const SETTINGS_ID: &str = "org.bande.Settings";
const SOFTWARE_UPDATE_PANE: &str = "software-update";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Discover,
    Installed,
    Updates,
}

impl Place {
    const ALL: [Self; 3] = [Self::Discover, Self::Installed, Self::Updates];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Category(Category),
    App(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Go(Place),
    Show(Page),
    Search(String),
    Back,
    More,
    Install(usize),
    Cancel(String),
    Open(usize),
    Remove(usize),
    ConfirmRemoval,
    CancelRemoval,
    Shot(isize),
    Website(String),
    SoftwareUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Task {
    Install,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Removal {
    key: String,
    id: String,
    also: Vec<String>,
}

type Widgets = Vec<Box<dyn Widget>>;

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn row(children: Widgets) -> Box<dyn Widget> {
    boxed(Stack::new(Axis::Horizontal, SPACING, 0.0, children))
}

fn category_name(category: Category) -> String {
    L10N.tr(match category {
        Category::Internet => "category-internet",
        Category::Productivity => "category-productivity",
        Category::Graphics => "category-graphics",
        Category::MusicAndVideo => "category-music-and-video",
        Category::Games => "category-games",
        Category::Education => "category-education",
        Category::Development => "category-development",
        Category::Utilities => "category-utilities",
        Category::System => "category-system",
    })
}

fn place_name(place: Place) -> String {
    L10N.tr(match place {
        Place::Discover => "discover",
        Place::Installed => "installed",
        Place::Updates => "updates",
    })
}

fn source(bundle: &Bundle) -> String {
    match bundle {
        Bundle::Deb(_) => L10N.tr("source-debian"),
        Bundle::Flatpak { remote, .. } => {
            L10N.tr_args("source-flatpak", [("remote", remote.clone().into())])
        }
    }
}

fn origin_name(origin: &str) -> String {
    let mut parts = origin.splitn(3, '-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("debian"), Some(suite), Some(component)) => L10N.tr_args(
            "origin-debian",
            [
                ("suite", suite.to_owned().into()),
                ("component", component.to_owned().into()),
            ],
        ),
        _ => origin.to_owned(),
    }
}

fn open_software_update() {
    let command = [SETTINGS.to_owned(), SOFTWARE_UPDATE_PANE.to_owned()];
    if let Err(error) = bande_xdg::spawn_detached(SETTINGS_ID, &command, None, None) {
        tracing::warn!("cannot open Software Update: {error}");
    }
}

fn open_website(url: &str) {
    let apps = bande_xdg::Apps::load(bande_xdg::data_dirs(), &bande_xdg::config_dirs());
    let scheme = url
        .split(':')
        .next()
        .unwrap_or("https")
        .to_ascii_lowercase();
    match apps.default_for(&format!("x-scheme-handler/{scheme}")) {
        Some(entry) => {
            if let Err(error) = bande_xdg::launch_with_arguments(entry, &[url.to_owned()], None) {
                tracing::warn!("cannot open {url}: {error}");
            }
        }
        None => tracing::warn!("no app opens {scheme} links"),
    }
}

fn deb(app: &Component) -> Option<&str> {
    app.bundles.iter().find_map(|bundle| match bundle {
        Bundle::Deb(package) => Some(package.as_str()),
        Bundle::Flatpak { .. } => None,
    })
}

fn status_text(task: Task, progress: &Progress) -> String {
    let id = match (progress.status, task) {
        (Status::Authorizing, _) => "status-authorizing",
        (Status::Waiting, _) => "status-waiting",
        (Status::Downloading, _) => "status-downloading",
        (Status::Removing, _) | (_, Task::Remove) => "status-removing",
        (Status::Installing | Status::Working, Task::Install) => "status-installing",
    };
    match progress.percent {
        Some(percent) if progress.status != Status::Authorizing => L10N.tr_args(
            "status-percent",
            [("status", L10N.tr(id).into()), ("percent", percent.into())],
        ),
        _ => L10N.tr(id),
    }
}

fn problem_text(problem: Problem, task: Task, name: &str) -> String {
    let id = match (problem, task) {
        (Problem::NoNetwork, _) => "problem-no-network",
        (Problem::Locked, _) => "problem-locked",
        (Problem::NoSpace, _) => "problem-no-space",
        (Problem::NotFound, _) => "problem-not-found",
        (Problem::Dependencies, _) => "problem-dependencies",
        (Problem::SystemPackage, _) => "problem-system-package",
        (Problem::Unsigned, _) => "problem-unsigned",
        (Problem::NoPackageKit, _) => "problem-no-packagekit",
        (Problem::Other, Task::Install) => "install-failed",
        (Problem::Other, Task::Remove) => "remove-failed",
    };
    L10N.tr_args(id, [("name", name.to_owned().into())])
}

fn latest_release(app: &Component) -> i64 {
    app.releases
        .iter()
        .filter_map(|release| release.timestamp)
        .max()
        .unwrap_or(0)
}

fn has_local_icon(app: &Component) -> bool {
    app.icons
        .iter()
        .any(|icon| matches!(icon, Icon::Cached { .. } | Icon::Local(_)))
}

pub struct Software {
    apps: Vec<Component>,
    loading: bool,
    installed: HashSet<String>,
    icons: HashMap<String, Option<Image>>,
    requested: HashSet<String>,
    shots: HashMap<String, Result<Image, ShotError>>,
    shot: usize,
    facts: HashMap<String, Facts>,
    busy: HashMap<String, (Task, Progress)>,
    failed: HashMap<String, (Task, Failure)>,
    removal: Option<Removal>,
    updates: Vec<Package>,
    shown: usize,
    jobs: Sender<Job>,
    queries: Sender<Query>,
    canceller: Canceller,
    media: Media,
    inbox: Inbox<Message>,
    place: Place,
    pages: Vec<(Page, ScrollState)>,
    query: String,
    intents: Rc<RefCell<Vec<Intent>>>,
    scroll: Rc<RefCell<ScrollState>>,
    ui: Ui,
    content: WidgetId,
}

impl Software {
    pub fn new(
        transactions: Client,
        queries: Client,
        arch: &str,
        palette: Palette,
        text: TextSystem,
    ) -> io::Result<Self> {
        let canceller = transactions.canceller();
        let workers = worker::start(transactions, queries, arch)?;
        let media = media::start(&workers.inbox.outbox())?;
        let intents: Rc<RefCell<Vec<Intent>>> = Rc::default();
        let typed = Rc::clone(&intents);
        let search = SearchField::new(&L10N.tr("search"), "", move |query: &str| {
            typed.borrow_mut().push(Intent::Search(query.to_owned()));
        });
        let chosen = Rc::clone(&intents);
        let names: Vec<String> = Place::ALL.into_iter().map(place_name).collect();
        let places = SegmentedControl::new(&L10N.tr("section"), &names, Some(0), move |index| {
            if let Some(place) = Place::ALL.get(index) {
                chosen.borrow_mut().push(Intent::Go(*place));
            }
        });
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let page = Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            vec![row(vec![boxed(places)]), boxed(search), boxed(content)],
        );
        let scroll: Rc<RefCell<ScrollState>> = Rc::default();
        let view = ScrollView::shared(boxed(page), Rc::clone(&scroll));
        let mut software = Self {
            apps: Vec::new(),
            loading: true,
            installed: HashSet::new(),
            icons: HashMap::new(),
            requested: HashSet::new(),
            shots: HashMap::new(),
            shot: 0,
            facts: HashMap::new(),
            busy: HashMap::new(),
            failed: HashMap::new(),
            removal: None,
            updates: Vec::new(),
            shown: PAGE_ROWS,
            jobs: workers.jobs,
            queries: workers.queries,
            canceller,
            media,
            inbox: workers.inbox,
            place: Place::Discover,
            pages: vec![(Page::Home, ScrollState::default())],
            query: String::new(),
            intents,
            scroll,
            ui: Ui::new(Box::new(view), Context::new(text, palette)),
            content: id,
        };
        software.ask(Query::Updates);
        software.rebuild();
        Ok(software)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn page(&self) -> Page {
        self.pages
            .last()
            .map(|(page, _)| *page)
            .unwrap_or(Page::Home)
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn accessibility(&self) -> accesskit::TreeUpdate {
        self.ui.accessibility()
    }

    pub fn pump(&mut self) -> bool {
        let messages = self.inbox.drain();
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if messages.is_empty() && intents.is_empty() {
            return false;
        }
        for message in messages {
            self.receive(message);
        }
        for intent in intents {
            self.act(intent);
        }
        self.rebuild();
        true
    }

    fn ask(&self, query: Query) {
        if self.queries.send(query).is_err() {
            tracing::warn!("the software queries stopped");
        }
    }

    fn act(&mut self, intent: Intent) {
        match intent {
            Intent::Go(place) => {
                self.place = place;
                self.shown = PAGE_ROWS;
                self.removal = None;
                self.pages = vec![(Page::Home, ScrollState::default())];
                *self.scroll.borrow_mut() = ScrollState::default();
                if place == Place::Updates {
                    self.ask(Query::Updates);
                }
            }
            Intent::Show(page) => self.push(page),
            Intent::Search(query) => {
                self.query = query;
                self.shown = PAGE_ROWS;
                if matches!(self.page(), Page::App(_)) {
                    self.back();
                }
            }
            Intent::Back => self.back(),
            Intent::More => self.shown += PAGE_ROWS,
            Intent::Install(index) => self.install(index),
            Intent::Cancel(key) => {
                if self.busy.contains_key(&key)
                    && let Err(error) = self.canceller.cancel()
                {
                    tracing::warn!("cannot cancel: {error}");
                }
            }
            Intent::Open(index) => self.open(index),
            Intent::Remove(index) => self.plan_removal(index),
            Intent::ConfirmRemoval => self.remove(),
            Intent::CancelRemoval => self.removal = None,
            Intent::Shot(step) => {
                self.shot = self.shot.saturating_add_signed(step);
                self.request_shot();
            }
            Intent::Website(url) => open_website(&url),
            Intent::SoftwareUpdate => open_software_update(),
        }
    }

    fn push(&mut self, page: Page) {
        let kept = self.scroll.replace(ScrollState::default());
        if let Some((_, scroll)) = self.pages.last_mut() {
            *scroll = kept;
        }
        self.removal = None;
        self.shown = PAGE_ROWS;
        self.pages.push((page, ScrollState::default()));
        if let Page::App(index) = page {
            self.shot = 0;
            self.request_shot();
            if let Some(app) = self.apps.get(index)
                && let Some(package) = deb(app)
                && !self.facts.contains_key(app.key())
            {
                self.ask(Query::Details {
                    key: app.key().to_owned(),
                    package: package.to_owned(),
                });
            }
        }
    }

    fn back(&mut self) {
        if self.pages.len() < 2 {
            return;
        }
        self.pages.pop();
        self.removal = None;
        let restored = self
            .pages
            .last()
            .map(|(_, scroll)| scroll.clone())
            .unwrap_or_default();
        *self.scroll.borrow_mut() = restored;
    }

    fn receive(&mut self, message: Message) {
        match message {
            Message::Loaded(Loaded::Catalogue { apps, installed }) => {
                self.apps = apps.into_iter().filter(|app| !app.needs_x11).collect();
                self.installed = installed;
                self.loading = false;
            }
            Message::Loaded(Loaded::Icons(icons)) => {
                for (key, image) in icons {
                    self.requested.remove(&key);
                    self.icons.insert(key, image);
                }
            }
            Message::Loaded(Loaded::Shot { url, image }) => {
                self.shots.insert(url, image);
            }
            Message::Progress(key, progress) => {
                if let Some((_, shown)) = self.busy.get_mut(&key) {
                    *shown = progress;
                }
            }
            Message::Installed(key) => {
                self.busy.remove(&key);
                self.facts.remove(&key);
                self.installed.insert(key);
                self.ask(Query::Updates);
            }
            Message::Removed(key) => {
                self.busy.remove(&key);
                self.facts.remove(&key);
                self.installed.remove(&key);
                self.ask(Query::Updates);
            }
            Message::Stopped(key) => {
                self.busy.remove(&key);
            }
            Message::RemovalPlanned { key, id, also } => {
                self.busy.remove(&key);
                self.removal = Some(Removal { key, id, also });
            }
            Message::Details(key, facts) => {
                self.facts.insert(key, facts);
            }
            Message::Updates(updates) => self.updates = updates,
            Message::Failed(key, failure) => {
                if let Some((task, _)) = self.busy.remove(&key) {
                    self.failed.insert(key, (task, failure));
                }
            }
        }
    }

    fn request_shot(&mut self) {
        let Page::App(index) = self.page() else {
            return;
        };
        let Some(app) = self.apps.get(index) else {
            return;
        };
        self.shot = self.shot.min(app.screenshots.len().saturating_sub(1));
        let Some(shot) = app.screenshots.get(self.shot) else {
            return;
        };
        let url = shot.thumbnail.clone().unwrap_or_else(|| shot.image.clone());
        if self.shots.contains_key(&url) {
            return;
        }
        if self.media.shots.send(url).is_err() {
            tracing::warn!("the screenshot loader stopped");
        }
    }

    fn request_icons(&mut self, indices: &[usize]) {
        let wanted: Vec<(String, Vec<Icon>)> = indices
            .iter()
            .filter_map(|index| self.apps.get(*index))
            .filter(|app| {
                !self.icons.contains_key(app.key()) && !self.requested.contains(app.key())
            })
            .map(|app| (app.key().to_owned(), app.icons.clone()))
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.requested
            .extend(wanted.iter().map(|(key, _)| key.clone()));
        if self.media.icons.send(wanted).is_err() {
            tracing::warn!("the icon loader stopped");
        }
    }

    fn start(&mut self, key: String, task: Task) {
        self.failed.remove(&key);
        self.busy.insert(key, (task, Progress::default()));
    }

    fn install(&mut self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        let Some(package) = deb(app).map(str::to_owned) else {
            return;
        };
        let key = app.key().to_owned();
        self.start(key.clone(), Task::Install);
        if self.jobs.send(Job::Install { key, package }).is_err() {
            tracing::warn!("the software worker stopped");
        }
    }

    fn plan_removal(&mut self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        let Some(package) = deb(app).map(str::to_owned) else {
            return;
        };
        let key = app.key().to_owned();
        self.start(key.clone(), Task::Remove);
        self.ask(Query::PlanRemoval { key, package });
    }

    fn remove(&mut self) {
        if let Some(Removal { key, id, .. }) = self.removal.take() {
            self.start(key.clone(), Task::Remove);
            if self.jobs.send(Job::Remove { key, id }).is_err() {
                tracing::warn!("the software worker stopped");
            }
        }
    }

    fn open(&self, index: usize) {
        let Some(launchable) = self.apps.get(index).and_then(|app| app.launchable.clone()) else {
            return;
        };
        let entries = bande_xdg::applications(
            &bande_xdg::data_dirs(),
            None,
            &bande_xdg::current_desktops(),
        );
        let entry = entries.iter().find(|entry| {
            entry.path.file_name().and_then(|name| name.to_str()) == Some(launchable.as_str())
        });
        if let Some(entry) = entry
            && let Err(error) = bande_xdg::launch(entry, None)
        {
            tracing::warn!("cannot open {}: {error}", entry.name);
        }
    }

    fn button(&self, title: &str, intent: Intent) -> Button {
        let intents = Rc::clone(&self.intents);
        Button::new(title, move || intents.borrow_mut().push(intent.clone()))
    }

    fn icon(&self, app: &Component) -> Option<Image> {
        self.icons.get(app.key()).cloned().flatten()
    }

    fn has_update(&self, app: &Component) -> bool {
        deb(app).is_some_and(|package| self.updates.iter().any(|update| update.name == package))
    }

    fn row_action(&self, index: usize, app: &Component) -> Option<Box<dyn Widget>> {
        let key = app.key();
        if let Some((task, progress)) = self.busy.get(key) {
            return Some(boxed(Label::new(&status_text(*task, progress)).secondary()));
        }
        if self.installed.contains(key) {
            return Some(match app.launchable.is_some() {
                true => boxed(self.button(&L10N.tr("open"), Intent::Open(index))),
                false => boxed(Label::new(&L10N.tr("is-installed")).secondary()),
            });
        }
        match deb(app) {
            Some(_) => Some(boxed(
                self.button(&L10N.tr("install"), Intent::Install(index)),
            )),
            None => app
                .bundles
                .first()
                .map(|bundle| boxed(Label::new(&source(bundle)).secondary())),
        }
    }

    fn app_row(&self, index: usize) -> Option<Box<dyn Widget>> {
        let app = self.apps.get(index)?;
        let intents = Rc::clone(&self.intents);
        Some(boxed(AppRow::new(
            self.icon(app),
            ROW_ICON,
            &app.name,
            &app.summary,
            self.row_action(index, app),
            move || intents.borrow_mut().push(Intent::Show(Page::App(index))),
        )))
    }

    fn rows(&mut self, indices: &[usize], limit: usize) -> Widgets {
        let shown: Vec<usize> = indices.iter().take(limit).copied().collect();
        self.request_icons(&shown);
        let rows: Widgets = shown
            .iter()
            .filter_map(|index| self.app_row(*index))
            .collect();
        let mut widgets = vec![boxed(Grid::new(2, SPACING / 2.0, rows))];
        if indices.len() > limit && limit == self.shown {
            widgets.push(row(vec![boxed(self.button(
                &L10N.tr_args("show-more", [("count", (indices.len() - limit).into())]),
                Intent::More,
            ))]));
        }
        widgets
    }

    fn sorted(&self, mut indices: Vec<usize>) -> Vec<usize> {
        indices.sort_by_cached_key(|index| {
            self.apps
                .get(*index)
                .map(|app| app.name.to_lowercase())
                .unwrap_or_default()
        });
        indices
    }

    fn matching(&self, keep: impl Fn(&Component) -> bool) -> Vec<usize> {
        self.apps
            .iter()
            .enumerate()
            .filter(|(_, app)| keep(app))
            .map(|(index, _)| index)
            .collect()
    }

    fn results(&mut self) -> Widgets {
        let found = search(&self.apps, &self.query);
        let query = self.query.trim().to_owned();
        if found.is_empty() {
            return vec![boxed(Label::new(
                &L10N.tr_args("no-results", [("query", query.into())]),
            ))];
        }
        let mut widgets = vec![boxed(Label::heading(
            &L10N.tr_args("results-for", [("query", query.into())]),
        ))];
        let shown = self.shown;
        widgets.extend(self.rows(&found, shown));
        widgets
    }

    fn discover(&mut self) -> Widgets {
        let tiles: Widgets = Category::ALL
            .into_iter()
            .map(|category| {
                boxed(self.button(
                    &category_name(category),
                    Intent::Show(Page::Category(category)),
                ))
            })
            .collect();
        let mut fresh = self.matching(|app| has_local_icon(app) && latest_release(app) > 0);
        fresh.sort_by_key(|index| {
            std::cmp::Reverse(self.apps.get(*index).map(latest_release).unwrap_or(0))
        });
        let mut widgets = vec![
            boxed(Label::heading(&L10N.tr("categories"))),
            boxed(Grid::new(3, SPACING / 2.0, tiles)),
        ];
        if !fresh.is_empty() {
            widgets.push(boxed(Label::heading(&L10N.tr("new-and-updated"))));
            widgets.extend(self.rows(&fresh, NEW_ROWS));
        }
        widgets
    }

    fn category(&mut self, category: Category) -> Widgets {
        let found =
            self.sorted(self.matching(|app| Category::of(&app.categories).contains(&category)));
        let mut widgets = vec![
            row(vec![boxed(self.button(&L10N.tr("back"), Intent::Back))]),
            boxed(Label::heading(&category_name(category))),
        ];
        let shown = self.shown;
        widgets.extend(self.rows(&found, shown));
        widgets
    }

    fn installed_list(&mut self) -> Widgets {
        let found = self.sorted(self.matching(|app| self.installed.contains(app.key())));
        let mut widgets = vec![boxed(Label::heading(&L10N.tr("installed")))];
        if found.is_empty() {
            widgets.push(boxed(Label::new(&L10N.tr("no-installed"))));
            return widgets;
        }
        let shown = self.shown;
        widgets.extend(self.rows(&found, shown));
        widgets
    }

    fn updates_list(&mut self) -> Widgets {
        let mut widgets = vec![boxed(Label::heading(&L10N.tr("updates")))];
        if self.updates.is_empty() {
            widgets.push(boxed(Label::new(&L10N.tr("up-to-date"))));
            widgets.push(boxed(Label::new(&L10N.tr("updates-hint")).secondary()));
            widgets.push(row(vec![boxed(
                self.button(&L10N.tr("check-for-updates"), Intent::SoftwareUpdate),
            )]));
            return widgets;
        }
        let apps = self.sorted(self.matching(|app| self.has_update(app)));
        widgets.push(boxed(Label::new(&L10N.tr_args(
            "updates-available",
            [("count", self.updates.len().into())],
        ))));
        widgets.push(boxed(Label::new(&L10N.tr("updates-hint")).secondary()));
        widgets.push(row(vec![boxed(
            self.button(&L10N.tr("update-all"), Intent::SoftwareUpdate)
                .primary(),
        )]));
        if !apps.is_empty() {
            widgets.push(boxed(Label::heading(&L10N.tr("app-updates"))));
            widgets.extend(self.rows(&apps, apps.len()));
        }
        let mut packages: Vec<String> = self
            .updates
            .iter()
            .take(LISTED_UPDATES)
            .map(|package| format!("{} {}", package.name, package.version))
            .collect();
        if self.updates.len() > LISTED_UPDATES {
            packages.push(L10N.tr_args(
                "more-updates",
                [("count", (self.updates.len() - LISTED_UPDATES).into())],
            ));
        }
        widgets.push(boxed(Disclosure::new(
            &L10N.tr_args("all-updates", [("count", self.updates.len().into())]),
            false,
            Label::new(&packages.join("\n")),
            |_| {},
        )));
        widgets
    }

    fn progress(&self, key: &str, app: &Component, task: Task, progress: &Progress) -> Widgets {
        let status = status_text(task, progress);
        let fraction = match progress.status {
            Status::Authorizing | Status::Waiting => None,
            _ => progress.percent.map(|percent| percent as f32 / 100.0),
        };
        let mut cancel = self.button(&L10N.tr("cancel"), Intent::Cancel(key.to_owned()));
        cancel.set_enabled(progress.allow_cancel);
        let mut widgets = vec![
            boxed(ProgressBar::new(&status, fraction)),
            row(vec![boxed(Label::new(&status)), boxed(cancel)]),
        ];
        if progress.status == Status::Authorizing {
            widgets.push(boxed(
                Label::new(&L10N.tr_args("authorize-hint", [("name", app.name.clone().into())]))
                    .secondary(),
            ));
        }
        widgets
    }

    fn actions(&self, index: usize, app: &Component) -> Widgets {
        let key = app.key();
        let mut widgets: Widgets = Vec::new();
        if let Some((task, progress)) = self.busy.get(key) {
            widgets.extend(self.progress(key, app, *task, progress));
        } else if let Some(removal) = self.removal.as_ref().filter(|removal| removal.key == key) {
            widgets.extend(self.confirmation(app, removal));
        } else if self.installed.contains(key) {
            let mut buttons: Widgets = Vec::new();
            if app.launchable.is_some() {
                buttons.push(boxed(
                    self.button(&L10N.tr("open"), Intent::Open(index)).primary(),
                ));
            }
            if deb(app).is_some() {
                buttons.push(boxed(
                    self.button(&L10N.tr("remove"), Intent::Remove(index)),
                ));
            }
            if !buttons.is_empty() {
                widgets.push(row(buttons));
            }
            if self.has_update(app) {
                widgets.push(row(vec![
                    boxed(Label::new(&L10N.tr("update-available"))),
                    boxed(self.button(&L10N.tr("update-all"), Intent::SoftwareUpdate)),
                ]));
            }
        } else if deb(app).is_some() {
            widgets.push(row(vec![boxed(
                self.button(&L10N.tr("install"), Intent::Install(index))
                    .primary(),
            )]));
            widgets.push(boxed(Label::new(&L10N.tr("password-hint")).secondary()));
        } else if let Some(bundle) = app.bundles.first() {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "flatpak-unsupported",
                [("source", source(bundle).into())],
            ))));
        }
        if let Some((task, failure)) = self.failed.get(key) {
            widgets.push(boxed(Label::new(&problem_text(
                failure.problem,
                *task,
                &app.name,
            ))));
            if !failure.details.is_empty() {
                widgets.push(boxed(Disclosure::new(
                    &L10N.tr("details"),
                    false,
                    Label::new(&failure.details),
                    |_| {},
                )));
            }
        }
        widgets
    }

    fn confirmation(&self, app: &Component, removal: &Removal) -> Widgets {
        let mut widgets = vec![boxed(Label::new(
            &L10N.tr_args("remove-confirm", [("name", app.name.clone().into())]),
        ))];
        if !removal.also.is_empty() {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "remove-also",
                [("packages", removal.also.join(", ").into())],
            ))));
        }
        widgets.push(row(vec![
            boxed(self.button(&L10N.tr("cancel"), Intent::CancelRemoval)),
            boxed(
                self.button(&L10N.tr("remove"), Intent::ConfirmRemoval)
                    .destructive(),
            ),
        ]));
        widgets
    }

    fn screenshots(&self, app: &Component) -> Widgets {
        let Some(shot) = app.screenshots.get(self.shot) else {
            return Vec::new();
        };
        let url = shot.thumbnail.as_deref().unwrap_or(&shot.image);
        let mut widgets: Widgets = vec![match self.shots.get(url) {
            None => boxed(Spinner::new(&L10N.tr("loading-screenshot"))),
            Some(Ok(image)) => boxed(Picture::fitted(
                image.clone(),
                SHOT_WIDTH,
                SHOT_HEIGHT,
                &L10N.tr_args("screenshot-of", [("name", app.name.clone().into())]),
            )),
            Some(Err(ShotError::Offline)) => {
                boxed(Label::new(&L10N.tr("screenshot-offline")).secondary())
            }
            Some(Err(ShotError::Unreadable)) => {
                boxed(Label::new(&L10N.tr("screenshot-unavailable")).secondary())
            }
        }];
        let count = app.screenshots.len();
        let mut controls: Widgets = Vec::new();
        if count > 1 {
            let mut previous = self.button(&L10N.tr("previous"), Intent::Shot(-1));
            previous.set_enabled(self.shot > 0);
            let mut next = self.button(&L10N.tr("next"), Intent::Shot(1));
            next.set_enabled(self.shot + 1 < count);
            controls.push(boxed(previous));
            controls.push(boxed(next));
            controls.push(boxed(
                Label::new(&L10N.tr_args(
                    "screenshot-count",
                    [("index", (self.shot + 1).into()), ("count", count.into())],
                ))
                .secondary(),
            ));
        }
        if let Some(caption) = shot
            .caption
            .as_deref()
            .filter(|caption| !caption.is_empty())
        {
            controls.push(boxed(Label::new(caption).secondary()));
        }
        if !controls.is_empty() {
            widgets.push(row(controls));
        }
        widgets
    }

    fn information(&self, app: &Component) -> Widgets {
        let facts = self.facts.get(app.key());
        let fact = |title: &str, value: Box<dyn Widget>| {
            boxed(Stack::new(
                Axis::Vertical,
                2.0,
                0.0,
                vec![boxed(Label::new(title).secondary()), value],
            ))
        };
        let text = |text: &str| boxed(Label::new(text));
        let mut cells: Widgets = Vec::new();
        let available = facts.and_then(|facts| facts.available.clone());
        let installed = facts.and_then(|facts| facts.installed.clone());
        let version = available
            .clone()
            .or_else(|| installed.clone())
            .or_else(|| app.releases.first().map(|release| release.version.clone()));
        if let Some(version) = version {
            cells.push(fact(&L10N.tr("info-version"), text(&version)));
        }
        if let Some(installed) = installed.filter(|installed| available.as_ref() != Some(installed))
        {
            cells.push(fact(&L10N.tr("info-installed-version"), text(&installed)));
        }
        if let Some(size) = facts.map(|facts| facts.size).filter(|size| *size > 0) {
            cells.push(fact(&L10N.tr("info-size"), text(&format_size(size))));
        }
        if let Some(category) = Category::of(&app.categories).first() {
            cells.push(fact(
                &L10N.tr("info-category"),
                text(&category_name(*category)),
            ));
        }
        if let Some(license) = &app.license {
            cells.push(fact(&L10N.tr("info-license"), text(license)));
        }
        let origin = facts
            .map(|facts| facts.origin.as_str())
            .filter(|origin| !origin.is_empty())
            .map(origin_name)
            .or_else(|| app.bundles.first().map(source));
        if let Some(origin) = origin {
            cells.push(fact(&L10N.tr("info-source"), text(&origin)));
        }
        if let Some(developer) = &app.developer {
            cells.push(fact(&L10N.tr("info-developer"), text(developer)));
        }
        if let Some(url) = facts
            .map(|facts| facts.homepage.clone())
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        {
            let link = self
                .button(&L10N.tr("visit-website"), Intent::Website(url))
                .link();
            cells.push(fact(&L10N.tr("info-website"), row(vec![boxed(link)])));
        }
        if cells.is_empty() {
            return Vec::new();
        }
        vec![
            boxed(Label::heading(&L10N.tr("information"))),
            boxed(Grid::new(3, SPACING, cells)),
        ]
    }

    fn details(&mut self, index: usize) -> Widgets {
        self.request_icons(&[index]);
        let Some(app) = self.apps.get(index) else {
            return Vec::new();
        };
        let mut heading: Widgets = vec![boxed(Label::heading(&app.name))];
        if let Some(developer) = &app.developer {
            heading.push(boxed(Label::new(developer).secondary()));
        }
        if !app.summary.is_empty() {
            heading.push(boxed(Label::new(&app.summary)));
        }
        let mut widgets = vec![
            row(vec![boxed(self.button(&L10N.tr("back"), Intent::Back))]),
            row(vec![
                boxed(Picture::new(
                    self.icon(app),
                    PAGE_ICON,
                    PAGE_ICON,
                    &app.name,
                )),
                boxed(Stack::new(Axis::Vertical, 4.0, 0.0, heading)),
            ]),
        ];
        widgets.extend(self.actions(index, app));
        widgets.extend(self.screenshots(app));
        for block in &app.description {
            let text = match block {
                Block::Paragraph(text) => text.clone(),
                Block::List(items) => items
                    .iter()
                    .map(|item| format!("• {item}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            widgets.push(boxed(Label::new(&text)));
        }
        widgets.extend(self.information(app));
        widgets
    }

    fn rebuild(&mut self) {
        let widgets = if self.loading {
            vec![boxed(Spinner::new(&L10N.tr("loading")))]
        } else if self.apps.is_empty() {
            vec![boxed(Label::new(&L10N.tr("no-catalogue")))]
        } else {
            match self.page() {
                Page::App(index) => self.details(index),
                _ if !self.query.trim().is_empty() => self.results(),
                Page::Category(category) => self.category(category),
                Page::Home => match self.place {
                    Place::Discover => self.discover(),
                    Place::Installed => self.installed_list(),
                    Place::Updates => self.updates_list(),
                },
            }
        };
        let page = boxed(Stack::new(Axis::Vertical, SPACING, 0.0, widgets));
        if let Some(holder) = self.ui.widget(self.content)
            && let Some(slot) = holder.children_mut().first_mut()
        {
            *slot = page;
        }
    }
}
