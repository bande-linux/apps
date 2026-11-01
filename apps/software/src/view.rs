use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use bande_catalog::{Block, Bundle, Category, Component, search};
use bande_inbox::Inbox;
use bande_packages::{Client, Progress};
use bande_toolkit::ui::{
    Axis, Button, Context, Label, PopUpButton, ProgressBar, ScrollView, SearchField, Stack, Ui,
    Widget, WidgetId,
};
use bande_toolkit::{Palette, TextSystem};

use crate::L10N;
use crate::worker::{self, Job, Message};

pub const WINDOW_SIZE: (u32, u32) = (720, 640);
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;
const MAX_ROWS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Discover,
    Category(Category),
    Installed,
    App(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Show(Page),
    Search(String),
    Back,
    Install(usize),
    Open(usize),
}

type Widgets = Vec<Box<dyn Widget>>;

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
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

fn sections() -> Vec<Page> {
    let mut sections = vec![Page::Discover];
    sections.extend(Category::ALL.into_iter().map(Page::Category));
    sections.push(Page::Installed);
    sections
}

fn section_name(page: Page) -> String {
    match page {
        Page::Category(category) => category_name(category),
        Page::Installed => L10N.tr("installed"),
        Page::Discover | Page::App(_) => L10N.tr("discover"),
    }
}

fn source(bundle: &Bundle) -> String {
    match bundle {
        Bundle::Deb(_) => L10N.tr("source-debian"),
        Bundle::Flatpak { remote, .. } => {
            L10N.tr_args("source-flatpak", [("remote", remote.clone().into())])
        }
    }
}

fn deb(app: &Component) -> Option<&str> {
    app.bundles.iter().find_map(|bundle| match bundle {
        Bundle::Deb(package) => Some(package.as_str()),
        Bundle::Flatpak { .. } => None,
    })
}

pub struct Software {
    apps: Vec<Component>,
    installed: HashSet<String>,
    installing: HashMap<String, Progress>,
    failed: HashMap<String, String>,
    jobs: Sender<Job>,
    inbox: Inbox<Message>,
    page: Page,
    section: Page,
    query: String,
    intents: Rc<RefCell<Vec<Intent>>>,
    ui: Ui,
    content: WidgetId,
    picker: WidgetId,
}

impl Software {
    pub fn new(
        apps: Vec<Component>,
        installed: HashSet<String>,
        client: Client,
        arch: &str,
        palette: Palette,
        text: TextSystem,
    ) -> io::Result<Self> {
        let (jobs, inbox) = worker::start(client, arch.to_owned())?;
        let intents: Rc<RefCell<Vec<Intent>>> = Rc::default();
        let typed = Rc::clone(&intents);
        let search = SearchField::new(&L10N.tr("search"), "", move |query: &str| {
            typed.borrow_mut().push(Intent::Search(query.to_owned()));
        });
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let picker = Stack::new(Axis::Horizontal, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let picker_id = picker.id();
        let page = Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            vec![boxed(search), boxed(picker), boxed(content)],
        );
        let mut software = Self {
            apps: apps.into_iter().filter(|app| !app.needs_x11).collect(),
            installed,
            installing: HashMap::new(),
            failed: HashMap::new(),
            jobs,
            inbox,
            page: Page::Discover,
            section: Page::Discover,
            query: String::new(),
            intents,
            ui: Ui::new(Box::new(ScrollView::new(page)), Context::new(text, palette)),
            content: id,
            picker: picker_id,
        };
        software.rebuild();
        Ok(software)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn show(&mut self, page: Page) {
        self.intents.borrow_mut().push(Intent::Show(page));
        self.pump();
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
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
            match intent {
                Intent::Show(Page::App(index)) => self.page = Page::App(index),
                Intent::Show(page) => {
                    self.section = page;
                    self.page = page;
                }
                Intent::Search(query) => {
                    self.query = query;
                    self.page = self.section;
                }
                Intent::Back => self.page = self.section,
                Intent::Install(index) => self.install(index),
                Intent::Open(index) => self.open(index),
            }
        }
        self.rebuild();
        true
    }

    fn receive(&mut self, message: Message) {
        match message {
            Message::Progress(key, progress) => {
                self.installing.insert(key, progress);
            }
            Message::Installed(key) => {
                self.installing.remove(&key);
                self.installed.insert(key);
            }
            Message::Failed(key, details) => {
                self.installing.remove(&key);
                self.failed.insert(key, details);
            }
        }
    }

    fn install(&mut self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        let Some(package) = deb(app) else {
            return;
        };
        let key = app.key().to_owned();
        self.failed.remove(&key);
        self.installing.insert(key.clone(), Progress::default());
        let job = Job::Install {
            key,
            package: package.to_owned(),
        };
        if self.jobs.send(job).is_err() {
            tracing::warn!("the software worker stopped");
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

    fn actions(&self, index: usize, app: &Component) -> Widgets {
        let key = app.key();
        let row =
            |button: Button| boxed(Stack::new(Axis::Horizontal, 0.0, 0.0, vec![boxed(button)]));
        let mut widgets: Widgets = Vec::new();
        if let Some(progress) = self.installing.get(key) {
            let status = L10N.tr("installing");
            widgets.push(boxed(ProgressBar::new(
                &status,
                progress.percent.map(|percent| percent as f32 / 100.0),
            )));
        } else if self.installed.contains(key) {
            if app.launchable.is_some() {
                widgets.push(row(self
                    .button(&L10N.tr("open"), Intent::Open(index))
                    .primary()));
            }
        } else if deb(app).is_some() {
            widgets.push(row(self
                .button(&L10N.tr("install"), Intent::Install(index))
                .primary()));
        }
        if let Some(details) = self.failed.get(key) {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "install-failed",
                [
                    ("name", app.name.clone().into()),
                    ("details", details.clone().into()),
                ],
            ))));
        }
        widgets
    }

    fn button(&self, title: &str, intent: Intent) -> Button {
        let intents = Rc::clone(&self.intents);
        Button::new(title, move || intents.borrow_mut().push(intent.clone()))
    }

    fn in_section(&self, app: &Component) -> bool {
        match self.section {
            Page::Category(category) => Category::of(&app.categories).contains(&category),
            Page::Installed => self.installed.contains(app.key()),
            Page::Discover | Page::App(_) => true,
        }
    }

    fn listed(&self) -> Vec<usize> {
        let mut found: Vec<usize> = search(&self.apps, &self.query)
            .into_iter()
            .filter(|index| {
                self.apps
                    .get(*index)
                    .is_some_and(|app| self.in_section(app))
            })
            .collect();
        if self.query.trim().is_empty() {
            found.sort_by_cached_key(|index| {
                self.apps
                    .get(*index)
                    .map(|app| app.name.to_lowercase())
                    .unwrap_or_default()
            });
        }
        found
    }

    fn list(&self) -> Widgets {
        if self.apps.is_empty() {
            return vec![boxed(Label::new(&L10N.tr("no-catalogue")))];
        }
        let listed = self.listed();
        if listed.is_empty() {
            return vec![boxed(Label::new(&match self.query.trim().is_empty() {
                true => L10N.tr("no-apps"),
                false => L10N.tr_args(
                    "no-results",
                    [("query", self.query.trim().to_owned().into())],
                ),
            }))];
        }
        let mut widgets: Widgets = vec![boxed(Label::heading(&section_name(self.section)))];
        widgets.extend(listed.iter().take(MAX_ROWS).filter_map(|index| {
            let app = self.apps.get(*index)?;
            Some(boxed(Stack::new(
                Axis::Vertical,
                2.0,
                0.0,
                vec![
                    boxed(
                        self.button(&app.name, Intent::Show(Page::App(*index)))
                            .quiet(),
                    ),
                    boxed(Label::new(&app.summary)),
                ],
            )))
        }));
        if listed.len() > MAX_ROWS {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "more-results",
                [("count", (listed.len() - MAX_ROWS).into())],
            ))));
        }
        widgets
    }

    fn details(&self, index: usize, app: &Component) -> Widgets {
        let mut widgets = vec![
            boxed(Stack::new(
                Axis::Horizontal,
                0.0,
                0.0,
                vec![boxed(self.button(&L10N.tr("back"), Intent::Back))],
            )),
            boxed(Label::heading(&app.name)),
        ];
        widgets.extend(self.actions(index, app));
        if let Some(developer) = &app.developer {
            widgets.push(boxed(Label::new(
                &L10N.tr_args("by-developer", [("developer", developer.clone().into())]),
            )));
        }
        if !app.summary.is_empty() {
            widgets.push(boxed(Label::new(&app.summary)));
        }
        if app.needs_x11 {
            widgets.push(boxed(Label::new(&L10N.tr("needs-x11"))));
        }
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
        if let Some(release) = app.releases.first() {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "info-version",
                [("version", release.version.clone().into())],
            ))));
        }
        if let Some(license) = &app.license {
            widgets.push(boxed(Label::new(
                &L10N.tr_args("info-license", [("license", license.clone().into())]),
            )));
        }
        let sources: Vec<String> = app.bundles.iter().map(source).collect();
        if !sources.is_empty() {
            widgets.push(boxed(Label::new(&sources.join("\n"))));
        }
        widgets
    }

    fn section_picker(&self) -> PopUpButton {
        let pages = sections();
        let names: Vec<String> = pages.iter().map(|page| section_name(*page)).collect();
        let selected = pages
            .iter()
            .position(|page| *page == self.section)
            .unwrap_or_default();
        let intents = Rc::clone(&self.intents);
        PopUpButton::new(&L10N.tr("section"), &names, selected, move |index| {
            if let Some(page) = pages.get(index) {
                intents.borrow_mut().push(Intent::Show(*page));
            }
        })
    }

    fn rebuild(&mut self) {
        let widgets = match self.page {
            Page::App(index) => match self.apps.get(index) {
                Some(app) => self.details(index, app),
                None => self.list(),
            },
            Page::Discover | Page::Category(_) | Page::Installed => self.list(),
        };
        let page = boxed(Stack::new(Axis::Vertical, SPACING, 0.0, widgets));
        let picker = boxed(self.section_picker());
        for (id, widget) in [(self.content, page), (self.picker, picker)] {
            if let Some(holder) = self.ui.widget(id)
                && let Some(slot) = holder.children_mut().first_mut()
            {
                *slot = widget;
            }
        }
    }
}
