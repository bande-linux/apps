use bande_app_settings::{Shared, Stored, flag, pairs};
use bande_toolkit::ui::{Context, Form, FormLine, Label, PopUpButton, ScrollView, Toggle, Ui};
use bande_toolkit::{Appearance, TextSystem};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use web_host::Url;

use crate::L10N;

pub(crate) const SETTINGS_SIZE: (u32, u32) = (560, 420);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Launch {
    #[default]
    LastSession,
    NewWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchEngine {
    Bing,
    Brave,
    DuckDuckGo,
    Ecosia,
    Google,
    Qwant,
    Startpage,
}

impl SearchEngine {
    pub(crate) const ALL: [Self; 7] = [
        Self::Bing,
        Self::Brave,
        Self::DuckDuckGo,
        Self::Ecosia,
        Self::Google,
        Self::Qwant,
        Self::Startpage,
    ];

    pub(crate) fn shuffled() -> Vec<Self> {
        let random = RandomState::new();
        let mut engines = Self::ALL.to_vec();
        engines.sort_by_key(|engine| random.hash_one(engine.key()));
        engines
    }

    fn key(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::Google => "google",
            Self::DuckDuckGo => "duckduckgo",
            Self::Startpage => "startpage",
            Self::Brave => "brave",
            Self::Ecosia => "ecosia",
            Self::Qwant => "qwant",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Bing => "Bing",
            Self::Google => "Google",
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Startpage => "Startpage",
            Self::Brave => "Brave Search",
            Self::Ecosia => "Ecosia",
            Self::Qwant => "Qwant",
        }
    }

    pub(crate) fn search(self, query: &str) -> Option<Url> {
        let (base, key) = match self {
            Self::Bing => ("https://www.bing.com/search", "q"),
            Self::Brave => ("https://search.brave.com/search", "q"),
            Self::DuckDuckGo => ("https://html.duckduckgo.com/html/", "q"),
            Self::Ecosia => ("https://www.ecosia.org/search", "q"),
            Self::Google => ("https://www.google.com/search", "q"),
            Self::Qwant => ("https://www.qwant.com/", "q"),
            Self::Startpage => ("https://www.startpage.com/sp/search", "query"),
        };
        Url::parse_with_params(base, [(key, query)]).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct BrowserSettings {
    pub(crate) launch: Launch,
    pub(crate) search: Option<SearchEngine>,
    pub(crate) https_only: bool,
}

impl Stored for BrowserSettings {
    const FILE: &'static str = "browser.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            match key {
                "launch" => {
                    settings.launch = match value {
                        "new-window" => Launch::NewWindow,
                        _ => Launch::LastSession,
                    }
                }
                "search" => {
                    settings.search = SearchEngine::ALL
                        .into_iter()
                        .find(|engine| engine.key() == value);
                }
                "https-only" => settings.https_only = flag(value).unwrap_or(false),
                _ => {}
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        let launch = match self.launch {
            Launch::LastSession => "last-session",
            Launch::NewWindow => "new-window",
        };
        let search = self.search.map_or("", SearchEngine::key);
        format!(
            "launch = {launch}\nsearch = {search}\nhttps-only = {}\n",
            self.https_only
        )
    }
}

pub(crate) fn settings_view(appearance: Appearance, settings: &Shared<BrowserSettings>) -> Ui {
    let shared = settings.clone();
    let launches = [L10N.tr("launch-last-session"), L10N.tr("launch-new-window")];
    let launch = PopUpButton::new(
        &L10N.tr("settings-launch"),
        &launches,
        match settings.borrow().launch {
            Launch::LastSession => 0,
            Launch::NewWindow => 1,
        },
        move |index| {
            shared.borrow_mut().launch = match index {
                1 => Launch::NewWindow,
                _ => Launch::LastSession,
            }
        },
    );
    let shared = settings.clone();
    let unchosen = settings.borrow().search.is_none();
    let mut engines: Vec<String> = Vec::new();
    if unchosen {
        engines.push(L10N.tr("search-not-chosen"));
    }
    engines.extend(
        SearchEngine::ALL
            .iter()
            .map(|engine| engine.name().to_owned()),
    );
    let offset = usize::from(unchosen);
    let chosen = settings.borrow().search.map_or(0, |search| {
        SearchEngine::ALL
            .iter()
            .position(|engine| *engine == search)
            .map_or(0, |index| index + offset)
    });
    let search = PopUpButton::new(
        &L10N.tr("settings-search-engine"),
        &engines,
        chosen,
        move |index| {
            let engine = index
                .checked_sub(offset)
                .and_then(|index| SearchEngine::ALL.get(index));
            if let Some(engine) = engine {
                shared.borrow_mut().search = Some(*engine);
            }
        },
    );
    let shared = settings.clone();
    let https_only = Toggle::new(
        &L10N.tr("settings-https-only"),
        settings.borrow().https_only,
        move |on| shared.borrow_mut().https_only = on,
    );
    let form = Form::new(
        &L10N.tr("settings-title"),
        vec![
            FormLine::Heading(L10N.tr("settings-general")),
            FormLine::Row(L10N.tr("settings-launch"), Box::new(launch)),
            FormLine::Heading(L10N.tr("settings-search")),
            FormLine::Row(L10N.tr("settings-search-engine"), Box::new(search)),
            FormLine::Wide(Box::new(
                Label::new(&L10N.tr("settings-search-note")).secondary(),
            )),
            FormLine::Heading(L10N.tr("settings-privacy")),
            FormLine::Row(L10N.tr("settings-https-only"), Box::new(https_only)),
            FormLine::Wide(Box::new(
                Label::new(&L10N.tr("settings-privacy-note")).secondary(),
            )),
        ],
    );
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form)),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
