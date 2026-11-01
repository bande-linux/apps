use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use bande_catalog::{Block, Bundle, Category, Component, search};
use bande_toolkit::ui::{
    Axis, Button, Context, Label, PopUpButton, ScrollView, SearchField, Stack, Ui, Widget, WidgetId,
};
use bande_toolkit::{Palette, TextSystem};

use crate::L10N;

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

pub struct Software {
    apps: Vec<Component>,
    installed: HashSet<String>,
    page: Page,
    section: Page,
    query: String,
    intents: Rc<RefCell<Vec<Intent>>>,
    ui: Ui,
    content: WidgetId,
}

impl Software {
    pub fn new(
        apps: Vec<Component>,
        installed: HashSet<String>,
        palette: Palette,
        text: TextSystem,
    ) -> Self {
        let intents: Rc<RefCell<Vec<Intent>>> = Rc::default();
        let typed = Rc::clone(&intents);
        let search = SearchField::new(&L10N.tr("search"), "", move |query: &str| {
            typed.borrow_mut().push(Intent::Search(query.to_owned()));
        });
        let chosen = Rc::clone(&intents);
        let pages = sections();
        let names: Vec<String> = pages.iter().map(|page| section_name(*page)).collect();
        let picker = PopUpButton::new(&L10N.tr("section"), &names, 0, move |index| {
            if let Some(page) = pages.get(index) {
                chosen.borrow_mut().push(Intent::Show(*page));
            }
        });
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let header = Stack::new(
            Axis::Horizontal,
            SPACING,
            0.0,
            vec![boxed(picker), boxed(search)],
        );
        let page = Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            vec![boxed(header), boxed(content)],
        );
        let mut software = Self {
            apps: apps.into_iter().filter(|app| !app.needs_x11).collect(),
            installed,
            page: Page::Discover,
            section: Page::Discover,
            query: String::new(),
            intents,
            ui: Ui::new(Box::new(ScrollView::new(page)), Context::new(text, palette)),
            content: id,
        };
        software.rebuild();
        software
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
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if intents.is_empty() {
            return false;
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
            }
        }
        self.rebuild();
        true
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

    fn details(&self, app: &Component) -> Widgets {
        let mut widgets = vec![
            boxed(self.button(&L10N.tr("back"), Intent::Back)),
            boxed(Label::heading(&app.name)),
        ];
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

    fn rebuild(&mut self) {
        let widgets = match self.page {
            Page::App(index) => match self.apps.get(index) {
                Some(app) => self.details(app),
                None => self.list(),
            },
            Page::Discover | Page::Category(_) | Page::Installed => self.list(),
        };
        let page = boxed(Stack::new(Axis::Vertical, SPACING, 0.0, widgets));
        if let Some(content) = self.ui.widget(self.content)
            && let Some(slot) = content.children_mut().first_mut()
        {
            *slot = page;
        }
    }
}
