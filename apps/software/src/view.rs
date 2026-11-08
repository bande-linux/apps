use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use bande_app_settings::{SettingsWatch, Stored, settings_folder};
use bande_datetime::DateFormats;
use bande_fs::format_size;
use bande_inbox::Inbox;
use bande_packages::flatpak::{
    self, Canceller as FlatpakCanceller, FLATHUB, Installed, Permission, permissions,
};
use bande_packages::{Canceller, Client, Package, Progress, Status};
use bande_toolkit::ui::{
    Axis, Button, Context, Disclosure, Label, ProgressBar, ScrollState, ScrollView, SearchField,
    SegmentedControl, Spinner, Stack, Ui, Widget, WidgetId,
};
use bande_toolkit::{Image, Palette, Rect, TextSystem};

use crate::L10N;
use crate::catalog::{self, Block, Bundle, Category, Component, Icon, flatpak_needs_x11, search};
use crate::media::{self, Loaded, Media, Request, ShotError};
use crate::online;
use crate::settings::SoftwareSettings;
use crate::widgets::{AppRow, Grid, Picture};
use crate::worker::{self, Facts, Failure, Job, Message, Problem, Query};

pub const WINDOW_SIZE: (u32, u32) = (820, 680);
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;
const PAGE_ROWS: usize = 40;
const NEW_ROWS: usize = 12;
const ROW_ICON: f32 = 40.0;
const PAGE_ICON: f32 = 96.0;
const SHOT_WIDTH: f32 = 640.0;
const SHOT_HEIGHT: f32 = 400.0;
const LISTED_UPDATES: usize = 30;
pub const COMMAND_FIND: &str = "find";
pub const COMMAND_BACK: &str = "back";
pub const COMMAND_DISCOVER: &str = "discover";
pub const COMMAND_INSTALLED: &str = "installed";
pub const COMMAND_UPDATES: &str = "updates";
const SETTINGS: &str = "bande-settings";
pub const SETTINGS_ID: &str = "org.bande.Settings";
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
    Source(String, Source),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Task {
    Install,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Debian,
    Flatpak(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Removal {
    key: String,
    id: String,
    also: Vec<String>,
    source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Busy {
    task: Task,
    progress: Progress,
    source: Source,
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

fn remote_name(remote: &str) -> String {
    match remote {
        FLATHUB => L10N.tr("flathub"),
        other => other.to_owned(),
    }
}

fn permission_text(permission: Permission) -> String {
    L10N.tr(match permission {
        Permission::Network => "permission-network",
        Permission::AllFiles => "permission-all-files",
        Permission::Home => "permission-home",
        Permission::ReadOnlyFiles => "permission-read-files",
        Permission::Folders => "permission-folders",
        Permission::Devices => "permission-devices",
        Permission::Audio => "permission-audio",
        Permission::Bluetooth => "permission-bluetooth",
        Permission::Printing => "permission-printing",
        Permission::Ssh => "permission-ssh",
        Permission::X11 => "permission-x11",
        Permission::SessionBus => "permission-session-bus",
        Permission::SystemBus => "permission-system-bus",
        Permission::Settings => "permission-settings",
    })
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

fn place_control(
    intents: &Rc<RefCell<Vec<Intent>>>,
    place: Place,
    updates: usize,
) -> SegmentedControl {
    let chosen = Rc::clone(intents);
    let names: Vec<String> = Place::ALL
        .into_iter()
        .map(|shown| match (shown, updates) {
            (Place::Updates, 1..) => L10N.tr_args("updates-counted", [("count", updates.into())]),
            _ => place_name(shown),
        })
        .collect();
    let selected = Place::ALL.iter().position(|shown| *shown == place);
    SegmentedControl::new(&L10N.tr("section"), &names, selected, move |index| {
        if let Some(place) = Place::ALL.get(index) {
            chosen.borrow_mut().push(Intent::Go(*place));
        }
    })
}

fn open_software_update() {
    let command = [SETTINGS.to_owned(), SOFTWARE_UPDATE_PANE.to_owned()];
    if let Err(error) = bande_xdg::spawn_detached(SETTINGS_ID, &command, None, None) {
        tracing::warn!("cannot open Software Update: {error}");
    }
}

fn in_background(work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name("software-open".to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start a thread: {error}");
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

fn flatpak_reference<'a>(app: &'a Component, wanted: &str) -> Option<&'a str> {
    app.bundles.iter().find_map(|bundle| match bundle {
        Bundle::Flatpak { remote, reference } if remote == wanted => Some(reference.as_str()),
        _ => None,
    })
}

fn sources(app: &Component) -> Vec<Source> {
    let mut found: Vec<Source> = deb(app).map(|_| Source::Debian).into_iter().collect();
    for bundle in &app.bundles {
        if let Bundle::Flatpak { remote, .. } = bundle {
            let source = Source::Flatpak(remote.clone());
            if !found.contains(&source) {
                found.push(source);
            }
        }
    }
    found
}

fn source_name(source: &Source) -> String {
    match source {
        Source::Debian => L10N.tr("debian"),
        Source::Flatpak(remote) => {
            L10N.tr_args("source-flatpak", [("remote", remote_name(remote).into())])
        }
    }
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
        (Problem::NotAuthorized, _) => "problem-not-authorized",
        (Problem::NoPackageKit, _) => "problem-no-packagekit",
        (Problem::NeedsX11, _) => "needs-x11",
        (Problem::Other | Problem::Cancelled, Task::Install) => "install-failed",
        (Problem::Other | Problem::Cancelled, Task::Remove) => "remove-failed",
    };
    L10N.tr_args(id, [("name", name.to_owned().into())])
}

fn block_text(block: &Block) -> String {
    match block {
        Block::Paragraph(text) => text.clone(),
        Block::List(items) => items
            .iter()
            .map(|item| format!("• {item}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn latest_release(app: &Component) -> i64 {
    app.releases
        .iter()
        .filter_map(|release| release.timestamp)
        .max()
        .unwrap_or(0)
}

fn remote_icon(app: &Component) -> Option<&str> {
    app.icons
        .iter()
        .filter_map(|icon| match icon {
            Icon::Remote { url, size } if *size as f32 > PAGE_ICON => Some((size, url.as_str())),
            _ => None,
        })
        .max_by_key(|(size, _)| **size)
        .map(|(_, url)| url)
}

fn has_local_icon(app: &Component) -> bool {
    app.icons
        .iter()
        .any(|icon| matches!(icon, Icon::Cached { .. } | Icon::Local(_)))
}

pub struct Software {
    apps: Vec<Component>,
    loading: bool,
    online: bool,
    dates: DateFormats,
    installed: HashSet<String>,
    flatpaks: HashMap<String, Installed>,
    flathub: bool,
    flathub_wanted: Option<bool>,
    flathub_busy: bool,
    flathub_failure: Option<Failure>,
    chosen: HashMap<String, Source>,
    icons: HashMap<String, Option<Image>>,
    requested: HashSet<String>,
    shots: HashMap<String, Result<Image, ShotError>>,
    shot: usize,
    facts: HashMap<String, Facts>,
    flatpak_facts: HashMap<(String, String), flatpak::Facts>,
    flatpak_offered: bool,
    busy: HashMap<String, Busy>,
    failed: HashMap<String, (Task, Failure)>,
    removal: Option<Removal>,
    updates: Vec<Package>,
    flatpak_updates: Vec<flatpak::Update>,
    shown: usize,
    jobs: Sender<Job>,
    queries: Sender<Query>,
    canceller: Canceller,
    flatpak_canceller: FlatpakCanceller,
    media: Media,
    inbox: Inbox<Message>,
    settings_folder: Option<PathBuf>,
    settings_watch: Option<SettingsWatch>,
    place: Place,
    pages: Vec<(Page, ScrollState)>,
    query: String,
    intents: Rc<RefCell<Vec<Intent>>>,
    scroll: Rc<RefCell<ScrollState>>,
    ui: Ui,
    content: WidgetId,
    search: WidgetId,
    places: WidgetId,
    header: WidgetId,
    counted: usize,
    finished: Vec<Finished>,
    settings_app: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    Installed(String),
    Failed(String),
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
        let flatpak_canceller = FlatpakCanceller::default();
        let workers = worker::start(transactions, queries, flatpak_canceller.clone(), arch)?;
        let folder = settings_folder();
        let settings_watch = folder
            .as_deref()
            .and_then(|folder| SettingsWatch::start(folder, SoftwareSettings::FILE));
        let flathub_wanted = folder
            .as_deref()
            .filter(|folder| folder.join(SoftwareSettings::FILE).exists())
            .map(|folder| SoftwareSettings::load(folder).flathub);
        let media = media::start(&workers.inbox.outbox())?;
        online::watch(workers.inbox.outbox());
        let intents: Rc<RefCell<Vec<Intent>>> = Rc::default();
        let typed = Rc::clone(&intents);
        let search = SearchField::new(&L10N.tr("search"), "", move |query: &str| {
            typed.borrow_mut().push(Intent::Search(query.to_owned()));
        });
        let places = place_control(&intents, Place::Discover, 0);
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let search_id = search.id();
        let places_id = places.id();
        let header = Stack::new(Axis::Horizontal, SPACING, 0.0, vec![boxed(places)]);
        let header_id = header.id();
        let page = Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            vec![boxed(header), boxed(search), boxed(content)],
        );
        let scroll: Rc<RefCell<ScrollState>> = Rc::default();
        let view = ScrollView::shared(boxed(page), Rc::clone(&scroll));
        let mut software = Self {
            apps: Vec::new(),
            loading: true,
            online: true,
            dates: DateFormats::from_env(),
            installed: HashSet::new(),
            flatpaks: HashMap::new(),
            flathub: false,
            flathub_wanted,
            flathub_busy: false,
            flathub_failure: None,
            chosen: HashMap::new(),
            icons: HashMap::new(),
            requested: HashSet::new(),
            shots: HashMap::new(),
            shot: 0,
            facts: HashMap::new(),
            flatpak_facts: HashMap::new(),
            flatpak_offered: false,
            busy: HashMap::new(),
            failed: HashMap::new(),
            removal: None,
            updates: Vec::new(),
            flatpak_updates: Vec::new(),
            shown: PAGE_ROWS,
            jobs: workers.jobs,
            queries: workers.queries,
            canceller,
            flatpak_canceller,
            media,
            inbox: workers.inbox,
            settings_folder: folder,
            settings_watch,
            place: Place::Discover,
            pages: vec![(Page::Home, ScrollState::default())],
            query: String::new(),
            intents,
            scroll,
            ui: Ui::new(Box::new(view), Context::new(text, palette)),
            content: id,
            search: search_id,
            places: places_id,
            header: header_id,
            counted: 0,
            finished: Vec::new(),
            settings_app: false,
        };
        software.ask(Query::Updates);
        software.rebuild();
        Ok(software)
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        let mut fds = vec![self.inbox.fd()];
        fds.extend(self.settings_watch.iter().map(SettingsWatch::fd));
        fds
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
        let settings = self
            .settings_watch
            .as_mut()
            .is_some_and(SettingsWatch::drain);
        if settings {
            self.follow_settings();
        }
        if messages.is_empty() && intents.is_empty() && !settings {
            return false;
        }
        for intent in intents {
            self.act(intent);
        }
        for message in messages {
            self.receive(message);
        }
        self.rebuild();
        true
    }

    pub fn set_settings_installed(&mut self, installed: bool) {
        if self.settings_app != installed {
            self.settings_app = installed;
            self.rebuild();
        }
    }

    pub fn reload(&mut self) {
        if self.media.icons.send(Request::Catalogue).is_err() {
            tracing::warn!("the catalogue loader stopped");
        }
        self.ask(Query::Updates);
    }

    fn follow_settings(&mut self) {
        let Some(folder) = &self.settings_folder else {
            return;
        };
        let wanted = SoftwareSettings::load(folder).flathub;
        if self.flathub_wanted == Some(wanted) {
            return;
        }
        self.flathub_wanted = Some(wanted);
        if wanted == self.flathub && self.flathub_failure.is_none() {
            return;
        }
        self.flathub_failure = None;
        self.flathub_busy = true;
        self.send(Job::Flathub(wanted));
    }

    fn remember_flathub(&mut self) {
        if self.flathub_wanted.is_some() {
            return;
        }
        self.flathub_wanted = Some(self.flathub);
        let settings = SoftwareSettings {
            flathub: self.flathub,
        };
        if let Some(folder) = &self.settings_folder
            && let Err(error) = settings.save(folder)
        {
            tracing::warn!("cannot save the Software settings: {error}");
        }
    }

    fn send(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::warn!("the software worker stopped");
        }
    }

    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.finished)
    }

    fn name_of(&self, key: &str) -> String {
        self.apps
            .iter()
            .find(|app| app.key() == key)
            .map(|app| app.name.clone())
            .unwrap_or_else(|| key.to_owned())
    }

    pub fn command(&mut self, command: &str) -> bool {
        let place = match command {
            COMMAND_FIND => {
                self.ui.set_focus(self.search);
                return true;
            }
            COMMAND_BACK => {
                self.intents.borrow_mut().push(Intent::Back);
                return self.pump();
            }
            COMMAND_DISCOVER => Place::Discover,
            COMMAND_INSTALLED => Place::Installed,
            COMMAND_UPDATES => Place::Updates,
            _ => return false,
        };
        if let Some(index) = Place::ALL.iter().position(|shown| *shown == place) {
            self.ui
                .update_widget(self.places, |control: &mut SegmentedControl, cx| {
                    control.set_selected(cx, index);
                });
        }
        self.intents.borrow_mut().push(Intent::Go(place));
        self.pump()
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
            Intent::Cancel(key) => match self.busy.get(&key).map(|busy| &busy.source) {
                Some(Source::Flatpak(_)) => self.flatpak_canceller.cancel(),
                Some(Source::Debian) => {
                    if let Err(error) = self.canceller.cancel() {
                        tracing::warn!("cannot cancel: {error}");
                    }
                }
                None => {}
            },
            Intent::Open(index) => self.open(index),
            Intent::Remove(index) => self.plan_removal(index),
            Intent::ConfirmRemoval => self.remove(),
            Intent::CancelRemoval => self.removal = None,
            Intent::Shot(step) => {
                self.shot = self.shot.saturating_add_signed(step);
                self.request_shot();
            }
            Intent::Website(url) => in_background(move || open_website(&url)),
            Intent::SoftwareUpdate => open_software_update(),
            Intent::Source(key, source) => {
                self.chosen.insert(key, source);
                self.refresh_details();
            }
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
            self.ask_details(index);
            self.shot = 0;
            self.shots.clear();
            self.request_shot();
            if let Some(url) = self.apps.get(index).and_then(remote_icon) {
                self.media.shots.want(url);
            }
        }
    }

    fn ask_details(&self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        let key = app.key().to_owned();
        let wanted = match self.is_installed(&key) {
            true => self.source_of(app).into_iter().collect(),
            false => sources(app),
        };
        for source in wanted {
            match source {
                Source::Debian if !self.facts.contains_key(&key) => {
                    if let Some(package) = deb(app) {
                        self.ask(Query::Details {
                            key: key.clone(),
                            package: package.to_owned(),
                        });
                    }
                }
                Source::Flatpak(remote)
                    if !self
                        .flatpak_facts
                        .contains_key(&(key.clone(), remote.clone())) =>
                {
                    let installed = self
                        .flatpaks
                        .get(&key)
                        .filter(|found| found.origin == remote);
                    if let Some(reference) = installed
                        .map(|found| found.reference.as_str())
                        .or_else(|| flatpak_reference(app, &remote))
                    {
                        self.ask(Query::FlatpakDetails {
                            key: key.clone(),
                            reference: reference.to_owned(),
                            remote,
                            installed: installed.is_some(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn source_of(&self, app: &Component) -> Option<Source> {
        let key = app.key();
        if let Some(found) = self.flatpaks.get(key) {
            return Some(Source::Flatpak(found.origin.clone()));
        }
        if self.installed.contains(key) {
            return deb(app).map(|_| Source::Debian);
        }
        let offered = sources(app);
        self.chosen
            .get(key)
            .filter(|source| offered.contains(source))
            .cloned()
            .or_else(|| offered.into_iter().next())
    }

    fn is_installed(&self, key: &str) -> bool {
        self.installed.contains(key) || self.flatpaks.contains_key(key)
    }

    fn refresh_details(&self) {
        if let Page::App(index) = self.page() {
            self.ask_details(index);
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
            Message::Loaded(Loaded::Catalogue {
                apps,
                installed,
                flatpaks,
                flathub,
            }) => {
                let apps: Vec<Component> = apps.into_iter().filter(|app| !app.needs_x11).collect();
                let shown: Vec<Option<usize>> = self
                    .pages
                    .iter()
                    .map(|(page, _)| match page {
                        Page::App(index) => {
                            let key = self.apps.get(*index).map(Component::key);
                            apps.iter().position(|app| Some(app.key()) == key)
                        }
                        _ => Some(0),
                    })
                    .collect();
                let kept = shown.iter().take_while(|found| found.is_some()).count();
                if kept.max(1) < self.pages.len() {
                    self.pages.truncate(kept.max(1));
                    let restored = self
                        .pages
                        .last()
                        .map(|(_, scroll)| scroll.clone())
                        .unwrap_or_default();
                    *self.scroll.borrow_mut() = restored;
                }
                for ((page, _), found) in self.pages.iter_mut().zip(shown) {
                    if let (Page::App(index), Some(found)) = (page, found) {
                        *index = found;
                    }
                }
                self.apps = apps;
                self.installed = installed;
                self.flatpaks = flatpaks;
                self.flathub = flathub;
                self.facts.clear();
                self.flatpak_facts.clear();
                self.flatpak_offered = !self.flatpaks.is_empty()
                    || self
                        .apps
                        .iter()
                        .flat_map(|app| &app.bundles)
                        .any(|bundle| matches!(bundle, Bundle::Flatpak { .. }));
                self.loading = false;
                self.remember_flathub();
                self.refresh_details();
            }
            Message::Online(online) => {
                self.online = online;
                if online {
                    self.shots
                        .retain(|_, shot| !matches!(shot, Err(ShotError::Offline)));
                    self.request_shot();
                }
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
                if let Some(busy) = self.busy.get_mut(&key) {
                    busy.progress = progress;
                }
            }
            Message::Installed(key) => {
                self.finished.push(Finished::Installed(self.name_of(&key)));
                let source = self.busy.remove(&key).map(|busy| busy.source);
                self.facts.remove(&key);
                match source {
                    Some(Source::Flatpak(remote)) => self.installed_flatpak(key, remote),
                    _ => {
                        self.installed.insert(key);
                    }
                }
                self.ask(Query::Updates);
                self.refresh_details();
            }
            Message::Removed(key) => {
                self.busy.remove(&key);
                self.facts.remove(&key);
                self.flatpak_facts.retain(|(found, _), _| *found != key);
                if self.flatpaks.remove(&key).is_none() {
                    self.installed.remove(&key);
                }
                self.ask(Query::Updates);
                self.refresh_details();
            }
            Message::FlatpakDetails(key, remote, facts) => {
                self.flatpak_facts.insert((key, remote), facts);
            }
            Message::Flathub(result) => {
                self.flathub_busy = false;
                match result {
                    Ok(on) => {
                        self.flathub = on;
                        self.reload();
                    }
                    Err(failure) if failure.problem == Problem::Cancelled => {}
                    Err(failure) => self.flathub_failure = Some(failure),
                }
            }
            Message::Stopped(key) => {
                self.busy.remove(&key);
            }
            Message::RemovalPlanned { key, id, also } => {
                self.busy.remove(&key);
                self.removal = Some(Removal {
                    key,
                    id,
                    also,
                    source: Source::Debian,
                });
            }
            Message::Details(key, facts) => {
                self.facts.insert(key, facts);
            }
            Message::Updates(updates, flatpaks) => {
                self.updates = updates;
                self.flatpak_updates = flatpaks;
            }
            Message::Failed(key, failure) => {
                if let Some(Busy { task, .. }) = self.busy.remove(&key) {
                    if task == Task::Install {
                        self.finished.push(Finished::Failed(self.name_of(&key)));
                    }
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
        if !self.shots.contains_key(&url) {
            self.media.shots.want(&url);
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
        if self.media.icons.send(Request::Icons(wanted)).is_err() {
            tracing::warn!("the icon loader stopped");
        }
    }

    fn installed_flatpak(&mut self, key: String, remote: String) {
        let Some(reference) = self
            .apps
            .iter()
            .find(|app| app.key() == key)
            .and_then(|app| flatpak_reference(app, &remote))
            .map(str::to_owned)
        else {
            return;
        };
        let version = self
            .flatpak_facts
            .remove(&(key.clone(), remote.clone()))
            .map(|facts| facts.version)
            .unwrap_or_default();
        self.flatpaks.insert(
            key.clone(),
            Installed {
                id: key,
                version,
                origin: remote,
                reference,
            },
        );
    }

    fn start(&mut self, key: String, task: Task, source: Source) {
        self.failed.remove(&key);
        let progress = match (&source, task) {
            (Source::Debian, _) => Progress::default(),
            (Source::Flatpak(_), Task::Install) => Progress {
                status: Status::Downloading,
                allow_cancel: true,
                ..Progress::default()
            },
            (Source::Flatpak(_), Task::Remove) => Progress {
                status: Status::Removing,
                ..Progress::default()
            },
        };
        self.busy.insert(
            key,
            Busy {
                task,
                progress,
                source,
            },
        );
    }

    fn install(&mut self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        let key = app.key().to_owned();
        let Some(source) = self.source_of(app) else {
            return;
        };
        let job = match &source {
            Source::Debian => deb(app).map(|package| Job::Install {
                key: key.clone(),
                package: package.to_owned(),
            }),
            Source::Flatpak(remote) => {
                flatpak_reference(app, remote).map(|reference| Job::FlatpakInstall {
                    key: key.clone(),
                    remote: remote.clone(),
                    reference: reference.to_owned(),
                })
            }
        };
        let Some(job) = job else {
            return;
        };
        self.start(key, Task::Install, source);
        self.send(job);
    }

    fn plan_removal(&mut self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        let key = app.key().to_owned();
        if let Some(found) = self.flatpaks.get(&key) {
            self.removal = Some(Removal {
                id: found.reference.clone(),
                key,
                also: Vec::new(),
                source: Source::Flatpak(found.origin.clone()),
            });
            return;
        }
        let Some(package) = deb(app).map(str::to_owned) else {
            return;
        };
        self.start(key.clone(), Task::Remove, Source::Debian);
        self.ask(Query::PlanRemoval { key, package });
    }

    fn remove(&mut self) {
        if let Some(Removal {
            key, id, source, ..
        }) = self.removal.take()
        {
            let job = match &source {
                Source::Debian => Job::Remove {
                    key: key.clone(),
                    id,
                },
                Source::Flatpak(_) => Job::FlatpakRemove {
                    key: key.clone(),
                    reference: id,
                },
            };
            self.start(key, Task::Remove, source);
            self.send(job);
        }
    }

    fn open(&self, index: usize) {
        let Some(launchable) = self.apps.get(index).and_then(|app| app.launchable.clone()) else {
            return;
        };
        in_background(move || {
            let mut dirs = bande_xdg::data_dirs();
            if let Some(exports) = catalog::flatpak_user_dir()
                .map(|dir| dir.join("exports").join("share"))
                .filter(|exports| !dirs.contains(exports))
            {
                dirs.push(exports);
            }
            let entries = bande_xdg::applications(&dirs, None, &bande_xdg::current_desktops());
            let entry = entries.iter().find(|entry| {
                entry.path.file_name().and_then(|name| name.to_str()) == Some(launchable.as_str())
            });
            if let Some(entry) = entry
                && let Err(error) = bande_xdg::launch(entry, None)
            {
                tracing::warn!("cannot open {}: {error}", entry.name);
            }
        });
    }

    fn button(&self, title: &str, intent: Intent) -> Button {
        let intents = Rc::clone(&self.intents);
        Button::new(title, move || intents.borrow_mut().push(intent.clone()))
    }

    fn icon(&self, app: &Component) -> Option<Image> {
        self.icons.get(app.key()).cloned().flatten()
    }

    fn has_update(&self, app: &Component) -> bool {
        match self.source_of(app) {
            Some(Source::Flatpak(_)) => self
                .flatpak_updates
                .iter()
                .any(|update| update.id == app.key()),
            _ => deb(app)
                .is_some_and(|package| self.updates.iter().any(|update| update.name == package)),
        }
    }

    fn update_count(&self) -> usize {
        self.updates.len() + self.flatpak_updates.len()
    }

    fn row_action(&self, index: usize, app: &Component) -> Option<Box<dyn Widget>> {
        let key = app.key();
        if let Some(busy) = self.busy.get(key) {
            return Some(boxed(
                Label::new(&status_text(busy.task, &busy.progress)).secondary(),
            ));
        }
        if self.is_installed(key) {
            return Some(match app.launchable.is_some() {
                true => boxed(self.button(&L10N.tr("open"), Intent::Open(index))),
                false => boxed(Label::new(&L10N.tr("is-installed")).secondary()),
            });
        }
        self.source_of(app)
            .map(|_| boxed(self.button(&L10N.tr("install"), Intent::Install(index))))
    }

    fn source_caption(&self, app: &Component) -> Option<String> {
        let source = self.source_of(app)?;
        let listed = matches!(self.place, Place::Installed | Place::Updates);
        match source {
            Source::Debian if !(listed && self.flatpak_offered) => None,
            source => Some(source_name(&source)),
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
            self.source_caption(app),
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
                .map(|app| (deb(app).is_none(), app.name.to_lowercase()))
                .unwrap_or_default()
        });
        indices
    }

    fn debian_first(&self, indices: &mut [usize]) {
        indices.sort_by_key(|index| self.apps.get(*index).is_none_or(|app| deb(app).is_none()));
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
        let mut found = search(&self.apps, &self.query);
        self.debian_first(&mut found);
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
        let mut fresh = self
            .matching(|app| deb(app).is_some() && has_local_icon(app) && latest_release(app) > 0);
        fresh.sort_by_key(|index| {
            std::cmp::Reverse(self.apps.get(*index).map(latest_release).unwrap_or(0))
        });
        let mut names = HashSet::new();
        fresh.retain(|index| {
            self.apps
                .get(*index)
                .is_some_and(|app| names.insert(app.name.to_lowercase()))
        });
        let mut widgets = self.flathub_status();
        widgets.extend([
            boxed(Label::heading(&L10N.tr("categories"))),
            boxed(Grid::new(3, SPACING / 2.0, tiles)),
        ]);
        if !fresh.is_empty() {
            widgets.push(boxed(Label::heading(&L10N.tr("new-and-updated"))));
            widgets.extend(self.rows(&fresh, NEW_ROWS));
        }
        widgets
    }

    fn flathub_status(&self) -> Widgets {
        if self.flathub_busy {
            let id = match self.flathub_wanted {
                Some(false) => "flathub-turning-off",
                _ => "flathub-turning-on",
            };
            return vec![row(vec![
                boxed(Spinner::new(&L10N.tr(id))),
                boxed(Label::new(&L10N.tr(id)).secondary()),
            ])];
        }
        let Some(failure) = &self.flathub_failure else {
            return Vec::new();
        };
        let mut widgets = vec![boxed(Label::new(&problem_text(
            failure.problem,
            Task::Install,
            &L10N.tr("flathub"),
        )))];
        if !failure.details.is_empty() {
            widgets.push(boxed(Disclosure::new(
                &L10N.tr("details"),
                false,
                Label::new(&failure.details),
                |_| {},
            )));
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
        let found = self.sorted(self.matching(|app| self.is_installed(app.key())));
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
        let count = self.update_count();
        if count == 0 {
            widgets.push(boxed(Label::new(&L10N.tr("up-to-date"))));
            if self.settings_app {
                widgets.push(boxed(Label::new(&L10N.tr("updates-hint")).secondary()));
                widgets.push(row(vec![boxed(
                    self.button(&L10N.tr("check-for-updates"), Intent::SoftwareUpdate),
                )]));
            }
            return widgets;
        }
        let apps = self.sorted(self.matching(|app| self.has_update(app)));
        widgets.push(boxed(Label::new(
            &L10N.tr_args("updates-available", [("count", count.into())]),
        )));
        if self.settings_app {
            widgets.push(boxed(Label::new(&L10N.tr("updates-hint")).secondary()));
            widgets.push(row(vec![boxed(
                self.button(&L10N.tr("update-all"), Intent::SoftwareUpdate)
                    .primary(),
            )]));
        }
        if !apps.is_empty() {
            widgets.push(boxed(Label::heading(&L10N.tr("app-updates"))));
            widgets.extend(self.rows(&apps, apps.len()));
        }
        let debian = self.flatpak_offered.then(|| source_name(&Source::Debian));
        let mut packages: Vec<String> = self
            .updates
            .iter()
            .map(|package| (&package.name, &package.version, debian.clone()))
            .chain(self.flatpak_updates.iter().map(|update| {
                let remote = self
                    .flatpaks
                    .get(&update.id)
                    .map(|found| found.origin.clone())
                    .unwrap_or_default();
                (
                    &update.name,
                    &update.version,
                    Some(source_name(&Source::Flatpak(remote))),
                )
            }))
            .take(LISTED_UPDATES)
            .map(|(name, version, source)| match source {
                Some(source) => L10N.tr_args(
                    "update-from",
                    [
                        ("name", name.clone().into()),
                        ("version", version.clone().into()),
                        ("source", source.into()),
                    ],
                ),
                None => format!("{name} {version}"),
            })
            .collect();
        if count > LISTED_UPDATES {
            packages
                .push(L10N.tr_args("more-updates", [("count", (count - LISTED_UPDATES).into())]));
        }
        widgets.push(boxed(Disclosure::new(
            &L10N.tr_args("all-updates", [("count", count.into())]),
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
                Label::new(&L10N.tr_args(
                    match task {
                        Task::Install => "authorize-hint",
                        Task::Remove => "authorize-hint-remove",
                    },
                    [("name", app.name.clone().into())],
                ))
                .secondary(),
            ));
        }
        widgets
    }

    fn source_version(&self, app: &Component, source: &Source) -> Option<String> {
        match source {
            Source::Debian => self.facts.get(app.key())?.available.clone(),
            Source::Flatpak(remote) => Some(
                self.flatpak_facts
                    .get(&(app.key().to_owned(), remote.clone()))?
                    .version
                    .clone(),
            ),
        }
        .filter(|version| !version.is_empty())
    }

    fn source_picker(&self, app: &Component) -> Widgets {
        let offered = sources(app);
        if self.is_installed(app.key()) || offered.len() < 2 {
            return Vec::new();
        }
        let names: Vec<String> = offered.iter().map(source_name).collect();
        let selected = self
            .source_of(app)
            .and_then(|chosen| offered.iter().position(|source| *source == chosen));
        let intents = Rc::clone(&self.intents);
        let key = app.key().to_owned();
        let choices = offered.clone();
        let mut widgets = vec![row(vec![
            boxed(Label::new(&L10N.tr("info-source")).secondary()),
            boxed(SegmentedControl::new(
                &L10N.tr("info-source"),
                &names,
                selected,
                move |index| {
                    if let Some(source) = choices.get(index) {
                        intents
                            .borrow_mut()
                            .push(Intent::Source(key.clone(), source.clone()));
                    }
                },
            )),
        ])];
        widgets.extend(offered.iter().map(|source| {
            let name = match self.source_version(app, source) {
                Some(version) => L10N.tr_args(
                    "source-version",
                    [
                        ("source", source_name(source).into()),
                        ("version", version.into()),
                    ],
                ),
                None => source_name(source),
            };
            let note = match source {
                Source::Debian => "source-debian-note",
                Source::Flatpak(_) => "source-flatpak-note",
            };
            boxed(
                Label::new(&L10N.tr_args(
                    "source-about",
                    [("source", name.into()), ("note", L10N.tr(note).into())],
                ))
                .secondary(),
            )
        }));
        widgets
    }

    fn chosen_flatpak_facts(&self, app: &Component) -> Option<&flatpak::Facts> {
        let Some(Source::Flatpak(remote)) = self.source_of(app) else {
            return None;
        };
        self.flatpak_facts.get(&(app.key().to_owned(), remote))
    }

    fn needs_x11(&self, app: &Component) -> bool {
        self.chosen_flatpak_facts(app)
            .is_some_and(|facts| flatpak_needs_x11(&facts.metadata))
    }

    fn actions(&self, index: usize, app: &Component) -> Widgets {
        let key = app.key();
        let mut widgets: Widgets = Vec::new();
        widgets.extend(self.source_picker(app));
        if let Some(busy) = self.busy.get(key) {
            widgets.extend(self.progress(key, app, busy.task, &busy.progress));
        } else if let Some(removal) = self.removal.as_ref().filter(|removal| removal.key == key) {
            widgets.extend(self.confirmation(app, removal));
        } else if self.is_installed(key) {
            let mut buttons: Widgets = Vec::new();
            if app.launchable.is_some() {
                buttons.push(boxed(
                    self.button(&L10N.tr("open"), Intent::Open(index)).primary(),
                ));
            }
            if self.source_of(app).is_some() {
                buttons.push(boxed(
                    self.button(&L10N.tr("remove"), Intent::Remove(index)),
                ));
            }
            if !buttons.is_empty() {
                widgets.push(row(buttons));
            }
            if self.has_update(app) {
                let mut update = vec![boxed(Label::new(&L10N.tr("update-available")))];
                if self.settings_app {
                    update.push(boxed(
                        self.button(&L10N.tr("update-all"), Intent::SoftwareUpdate),
                    ));
                }
                widgets.push(row(update));
            }
        } else if self.needs_x11(app) {
            widgets.push(boxed(Label::new(&L10N.tr("needs-x11"))));
        } else if let Some(source) = self.source_of(app) {
            widgets.push(row(vec![boxed(
                self.button(&L10N.tr("install"), Intent::Install(index))
                    .primary(),
            )]));
            let hint = match source {
                Source::Debian => "password-hint",
                Source::Flatpak(_) => "flatpak-hint",
            };
            widgets.push(boxed(Label::new(&L10N.tr(hint)).secondary()));
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
        if matches!(removal.source, Source::Flatpak(_)) {
            widgets.push(boxed(Label::new(&L10N.tr("remove-runtimes")).secondary()));
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

    fn whats_new(&self, app: &Component) -> Widgets {
        let Some(release) = app
            .releases
            .first()
            .filter(|release| !release.notes.is_empty())
        else {
            return Vec::new();
        };
        let date = release
            .timestamp
            .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
            .map(|at| {
                self.dates
                    .date(&at.to_zoned(jiff::tz::TimeZone::system()), true)
            });
        let version = match date {
            Some(date) => L10N.tr_args(
                "release-dated",
                [
                    ("version", release.version.clone().into()),
                    ("date", date.into()),
                ],
            ),
            None => L10N.tr_args("release", [("version", release.version.clone().into())]),
        };
        let mut widgets = vec![
            boxed(Label::heading(&L10N.tr("whats-new"))),
            boxed(Label::new(&version).secondary()),
        ];
        widgets.extend(
            release
                .notes
                .iter()
                .map(|block| boxed(Label::new(&block_text(block)))),
        );
        widgets
    }

    fn flatpak_facts(&self, app: &Component) -> Option<(Facts, Option<(String, u64)>)> {
        let Some(Source::Flatpak(remote)) = self.source_of(app) else {
            return None;
        };
        let facts = self
            .flatpak_facts
            .get(&(app.key().to_owned(), remote.clone()))?;
        let installed = self.flatpaks.get(app.key());
        let version = (!facts.version.is_empty()).then(|| facts.version.clone());
        let runtime = facts
            .runtime
            .clone()
            .filter(|_| installed.is_none() && facts.runtime_download > 0)
            .map(|runtime| (runtime, facts.runtime_download));
        Some((
            Facts {
                available: version.clone().filter(|_| installed.is_none()),
                installed: installed
                    .map(|found| found.version.clone())
                    .filter(|version| !version.is_empty())
                    .or(version.filter(|_| installed.is_some())),
                size: installed.map(|_| facts.size).unwrap_or(0),
                download: installed.map(|_| 0).unwrap_or(facts.download),
                extra: 0,
                origin: source_name(&Source::Flatpak(remote)),
                homepage: String::new(),
            },
            runtime,
        ))
    }

    fn permissions(&self, app: &Component) -> Widgets {
        let Some(facts) = self.chosen_flatpak_facts(app) else {
            return Vec::new();
        };
        let mut widgets = vec![boxed(Label::heading(&L10N.tr("permissions")))];
        let found = permissions(&facts.metadata);
        if found.is_empty() {
            widgets.push(boxed(Label::new(&L10N.tr("permission-none"))));
        }
        widgets.extend(
            found
                .into_iter()
                .map(|permission| boxed(Label::new(&format!("• {}", permission_text(permission))))),
        );
        widgets
    }

    fn information(&self, app: &Component) -> Widgets {
        let flatpak = self.flatpak_facts(app);
        let runtime = flatpak.as_ref().and_then(|(_, runtime)| runtime.clone());
        let facts = flatpak
            .as_ref()
            .map(|(facts, _)| facts)
            .or_else(|| self.facts.get(app.key()));
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
        if let Some(facts) = facts.filter(|facts| facts.download > 0) {
            let size = format_size(facts.download);
            let value = match (facts.extra, runtime) {
                (_, Some((runtime, runtime_size))) => L10N.tr_args(
                    "download-with-runtime",
                    [
                        ("size", size.into()),
                        ("runtime", runtime.into()),
                        ("runtime_size", format_size(runtime_size).into()),
                    ],
                ),
                (0, None) => size,
                (extra, None) => L10N.tr_args(
                    "download-with",
                    [("size", size.into()), ("count", extra.into())],
                ),
            };
            cells.push(fact(&L10N.tr("info-download"), text(&value)));
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
            .or_else(|| self.source_of(app).map(|source| source_name(&source)));
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
        if let Some(source @ Source::Flatpak(_)) = self.source_of(app) {
            heading.push(boxed(Label::new(&source_name(&source)).secondary()));
        }
        let mut widgets = vec![
            row(vec![boxed(self.button(&L10N.tr("back"), Intent::Back))]),
            row(vec![
                boxed(Picture::new(
                    remote_icon(app)
                        .and_then(|url| self.shots.get(url)?.clone().ok())
                        .or_else(|| self.icon(app)),
                    PAGE_ICON,
                    PAGE_ICON,
                    &app.name,
                )),
                boxed(Stack::new(Axis::Vertical, 4.0, 0.0, heading)),
            ]),
        ];
        widgets.extend(self.actions(index, app));
        widgets.extend(self.screenshots(app));
        widgets.extend(
            app.description
                .iter()
                .map(|block| boxed(Label::new(&block_text(block)))),
        );
        widgets.extend(self.whats_new(app));
        widgets.extend(self.permissions(app));
        widgets.extend(self.information(app));
        widgets
    }

    fn recount(&mut self) {
        if self.counted == self.update_count() {
            return;
        }
        self.counted = self.update_count();
        let places = place_control(&self.intents, self.place, self.counted);
        let id = places.id();
        if let Some(holder) = self.ui.widget(self.header)
            && let Some(slot) = holder.children_mut().first_mut()
        {
            *slot = boxed(places);
            self.places = id;
        }
    }

    fn rebuild(&mut self) {
        self.recount();
        let mut widgets = if self.loading {
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
        if !self.online {
            widgets.insert(0, boxed(Label::new(&L10N.tr("offline")).secondary()));
        }
        let page = boxed(Stack::new(Axis::Vertical, SPACING, 0.0, widgets));
        if let Some(holder) = self.ui.widget(self.content)
            && let Some(slot) = holder.children_mut().first_mut()
        {
            *slot = page;
        }
    }
}
