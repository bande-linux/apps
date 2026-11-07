mod address;
mod page;
mod pages;
mod places;
mod renderer;
mod session;
mod settings;
mod tiles;
mod view;

use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::LazyLock;

use bande_app::{
    Cx, Detached, Info, Keys, Kind, Menus, Program, Saved, Settings as SettingsWindowInfo,
    TabCarry, TabDrop, View, WindowId,
};
use bande_app_menu::Entry;
use bande_app_settings::{SettingsWatch, SettingsWindow, Stored, settings_folder};
use bande_l10n::Localizer;
use bande_toolkit::Appearance;
use web_host::Url;

use crate::places::{Place, Places};
use crate::renderer::{Renderer, Session};
use crate::settings::{BrowserSettings, Launch, SETTINGS_SIZE, settings_view};
use crate::view::{Ask, BrowserView, CarriedTab, SavedWindow};

const ENGLISH: &str = include_str!("../locales/en-US/bande-browser.ftl");
static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub(crate) static INFO: Info = Info {
    id: "org.bande.Browser",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: Some(SettingsWindowInfo {
        title: || L10N.tr("settings-title"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsWindow::<BrowserSettings>::new(
        appearance,
        settings_folder(),
        settings_view,
    ))
}

const WINDOW_SIZE: (u32, u32) = (1100, 760);
const SESSION_FILE: &str = "browser-session";
const NEW_WINDOW: &str = "new-window";
const NEW_PRIVATE_WINDOW: &str = "new-private-window";
const NEW_TAB: &str = "new-tab";
const OPEN_LOCATION: &str = "open-location";
const RELOAD: &str = "reload";
const STOP: &str = "stop";
const BACK: &str = "back";
const FORWARD: &str = "forward";
const REOPEN_TAB: &str = "reopen-tab";
const NEXT_TAB: &str = "next-tab";
const PREVIOUS_TAB: &str = "previous-tab";
const SHOW_TAB: &str = "show-tab-";
const PLACES_FILE: &str = "browser-places.sqlite";
const SCHEME_TYPE: &str = "x-scheme-handler/https";
const MENU_LABEL: usize = 60;
const SETTINGS_SOURCE: usize = 1;
const SHOW_HISTORY: &str = "show-history";
const RECENT_PAGE: &str = "recent-page-";
const RECENT_SHOWN: usize = 10;
const ADD_BOOKMARK: &str = "add-bookmark";
const SHOW_BOOKMARKS: &str = "show-bookmarks";
const BOOKMARK: &str = "bookmark-";
const ZOOM_IN: &str = "zoom-in";
const ZOOM_OUT: &str = "zoom-out";
const ACTUAL_SIZE: &str = "actual-size";
const SYSTEM_BROWSER: &str = "open-in-system-browser";
const CLEAR_HISTORY: &str = "clear-history";
const WEBSITE_DATA: &str = "website-data";

struct Browser {
    renderer: Renderer,
    ids: Rc<Cell<u64>>,
    next_private: u32,
    carry: TabCarry,
    quitting: bool,
    carried_session: Session,
    session_file: Option<PathBuf>,
    places: Rc<Places>,
    settings: Rc<Cell<BrowserSettings>>,
    settings_watch: Option<SettingsWatch>,
}

fn addresses(arguments: &[String], settings: BrowserSettings) -> Vec<Url> {
    arguments
        .iter()
        .filter_map(|argument| address::parse(argument, settings.search))
        .collect()
}

fn blank() -> SavedWindow {
    SavedWindow {
        tabs: Vec::new(),
        active: 0,
    }
}

impl Browser {
    fn open_view(
        &mut self,
        cx: &mut Cx<Self>,
        session: Session,
        size: (u32, u32),
    ) -> Result<WindowId, bande_app::Error> {
        let (renderer, ids, places, settings) = (
            self.renderer.link(),
            Rc::clone(&self.ids),
            Rc::clone(&self.places),
            Rc::clone(&self.settings),
        );
        Ok(
            cx.open(Kind::Main, &L10N.tr("app-name"), size, move |appearance| {
                BrowserView::new(appearance, session, renderer, ids, places, settings)
            })?,
        )
    }

    fn open_window(&mut self, cx: &mut Cx<Self>, session: Session, saved: SavedWindow) {
        let window = match self.open_view(cx, session, WINDOW_SIZE) {
            Ok(window) => window,
            Err(error) => return tracing::warn!("cannot open a window: {error}"),
        };
        if let Some((_, view)) = cx.window(window) {
            if saved.tabs.is_empty() {
                view.new_tab();
            }
            for (index, url) in saved.tabs.into_iter().enumerate() {
                view.open_tab(Some(url), index == saved.active);
            }
        }
        self.serve(cx, window);
    }

    fn open_urls(&mut self, cx: &mut Cx<Self>, urls: Vec<Url>) {
        let focused = cx
            .focused()
            .filter(|window| cx.view(*window).is_some_and(|view| !view.is_private()));
        let Some(window) = focused else {
            let saved = SavedWindow {
                tabs: urls,
                active: 0,
            };
            return self.open_window(cx, Session::Default, saved);
        };
        if let Some((shown, view)) = cx.window(window) {
            for url in urls {
                view.open_tab(Some(url), true);
            }
            shown.request_redraw();
        }
        self.serve(cx, window);
    }

    fn restore(&mut self, cx: &mut Cx<Self>) -> bool {
        let windows = self
            .session_file
            .as_deref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .map(|text| session::parse(&text))
            .unwrap_or_default();
        let restored = !windows.is_empty();
        for saved in windows {
            self.open_window(cx, Session::Default, saved);
        }
        restored
    }

    fn ask_search_engine(&mut self, cx: &mut Cx<Self>) {
        let settings = self.settings.get();
        if settings.search.is_none()
            && !settings.search_asked
            && let Some(window) = cx.focused().or_else(|| cx.windows().next())
            && let Some((shown, view)) = cx.window(window)
        {
            view.ask_search_engine(None);
            shown.request_redraw();
        }
    }

    fn save_session(&mut self, cx: &mut Cx<Self>) {
        let Some(file) = &self.session_file else {
            return;
        };
        let windows: Vec<SavedWindow> = cx
            .windows()
            .filter_map(|window| cx.view(window).and_then(BrowserView::saved))
            .collect();
        if let Err(error) = bande_xdg::replace_file(file, session::write(&windows)) {
            tracing::warn!("cannot save the open windows: {error}");
        }
    }

    fn float_window(
        &mut self,
        cx: &mut Cx<Self>,
        size: (u32, u32),
    ) -> Result<WindowId, bande_app::Error> {
        self.open_view(cx, self.carried_session, size)
    }

    fn detach(&mut self, cx: &mut Cx<Self>, from: WindowId, detached: Detached<CarriedTab>) {
        self.carried_session = detached.tab.session();
        let mut carry = std::mem::take(&mut self.carry);
        carry.detach(cx, from, detached, &mut |cx, size| {
            self.float_window(cx, size)
        });
        self.carry = carry;
    }

    fn tab_drop(&mut self, cx: &mut Cx<Self>, window: WindowId, drop: TabDrop) {
        let mut carry = std::mem::take(&mut self.carry);
        carry.drop_event(cx, window, drop, &mut |cx, size| {
            self.float_window(cx, size)
        });
        self.carry = carry;
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let (asks, changed) = cx
            .window(window)
            .map(|(_, view)| (view.take_asks(), view.take_changed()))
            .unwrap_or_default();
        for ask in asks {
            match ask {
                Ask::Detach(detached) => self.detach(cx, window, *detached),
                Ask::TabDrop(drop) => self.tab_drop(cx, window, drop),
            }
        }
        if !self.carry.is_carrying() {
            let emptied: Vec<WindowId> = cx
                .windows()
                .filter(|window| cx.view(*window).is_some_and(BrowserView::is_empty))
                .collect();
            for window in emptied {
                cx.close(window);
            }
        }
        if changed {
            self.save_session(cx);
        }
        cx.refresh_menus();
    }
}

impl Program for Browser {
    type View = BrowserView;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let folder = settings_folder();
        let mut browser = Self {
            renderer: renderer::start()?,
            ids: Rc::default(),
            next_private: 0,
            carry: TabCarry::default(),
            quitting: false,
            carried_session: Session::Default,
            session_file: cx.state_file(SESSION_FILE),
            places: Rc::new(open_places(cx.state_file(PLACES_FILE))?),
            settings: Rc::new(Cell::new(
                folder
                    .as_deref()
                    .map(BrowserSettings::load)
                    .unwrap_or_default(),
            )),
            settings_watch: folder
                .as_deref()
                .and_then(|folder| SettingsWatch::start(folder, BrowserSettings::FILE)),
        };
        let settings = browser.settings.get();
        let urls = addresses(&arguments, settings);
        if !urls.is_empty() {
            browser.open_urls(cx, urls);
        }
        Ok(browser)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = cx.focused().and_then(|window| cx.view(window));
        let file = vec![
            Entry::item(&L10N.tr("new-window"), NEW_WINDOW).shortcut("Ctrl+N"),
            Entry::item(&L10N.tr("new-private-window"), NEW_PRIVATE_WINDOW)
                .shortcut("Shift+Ctrl+N"),
            Entry::item(&L10N.tr("new-tab"), NEW_TAB).shortcut("Ctrl+T"),
            Entry::item(&L10N.tr("open-location"), OPEN_LOCATION).shortcut("Ctrl+L"),
        ];
        let history = vec![
            Entry::item(&L10N.tr("back"), BACK)
                .shortcut("Alt+Left")
                .enabled(view.is_some_and(BrowserView::can_go_back)),
            Entry::item(&L10N.tr("forward"), FORWARD)
                .shortcut("Alt+Right")
                .enabled(view.is_some_and(BrowserView::can_go_forward)),
            Entry::item(&L10N.tr("reopen-tab"), REOPEN_TAB)
                .shortcut("Shift+Ctrl+T")
                .enabled(view.is_some_and(BrowserView::can_reopen_closed_tab)),
        ];
        let page = vec![
            Entry::item(&L10N.tr("reload"), RELOAD).shortcut("Ctrl+R"),
            Entry::item(&L10N.tr("stop"), STOP)
                .shortcut("Escape")
                .enabled(view.is_some_and(BrowserView::is_loading)),
            Entry::separator(),
            Entry::item(&L10N.tr("zoom-in"), ZOOM_IN).shortcut("Ctrl+="),
            Entry::item(&L10N.tr("zoom-out"), ZOOM_OUT).shortcut("Ctrl+-"),
            Entry::item(&L10N.tr("actual-size"), ACTUAL_SIZE).shortcut("Ctrl+0"),
        ];
        let mut history = history;
        history.push(Entry::separator());
        history.push(Entry::item(&L10N.tr("show-history"), SHOW_HISTORY).shortcut("Ctrl+H"));
        history.push(Entry::item(&L10N.tr("clear-history"), CLEAR_HISTORY).enabled(view.is_some()));
        history.push(Entry::item(&L10N.tr("website-data"), WEBSITE_DATA).enabled(view.is_some()));
        let recent = self.places.recent(RECENT_SHOWN);
        if !recent.is_empty() {
            history.push(Entry::separator());
        }
        history.extend(recent.iter().enumerate().map(|(index, place)| {
            Entry::item(&menu_label(place), &format!("{RECENT_PAGE}{index}"))
        }));
        let bookmarked = view.is_some_and(BrowserView::is_bookmarked);
        let mut bookmarks = vec![
            Entry::item(
                &L10N.tr(if bookmarked {
                    "remove-bookmark"
                } else {
                    "add-bookmark"
                }),
                ADD_BOOKMARK,
            )
            .shortcut("Ctrl+D")
            .enabled(view.and_then(BrowserView::current_page).is_some()),
            Entry::item(&L10N.tr("show-bookmarks"), SHOW_BOOKMARKS).shortcut("Shift+Ctrl+O"),
        ];
        let saved = self.places.bookmarks();
        if !saved.is_empty() {
            bookmarks.push(Entry::separator());
        }
        bookmarks.extend(
            saved.iter().enumerate().map(|(index, place)| {
                Entry::item(&menu_label(place), &format!("{BOOKMARK}{index}"))
            }),
        );
        let mut file = file;
        file.push(Entry::separator());
        file.push(
            Entry::item(&L10N.tr("open-in-system-browser"), SYSTEM_BROWSER)
                .enabled(view.and_then(BrowserView::current_page).is_some()),
        );
        let tabs = view.map_or(0, BrowserView::tab_count);
        let mut window = vec![
            Entry::item(&L10N.tr("next-tab"), NEXT_TAB)
                .shortcut("Ctrl+Tab")
                .enabled(tabs > 1),
            Entry::item(&L10N.tr("previous-tab"), PREVIOUS_TAB)
                .shortcut("Shift+Ctrl+Tab")
                .enabled(tabs > 1),
        ];
        window.extend((1..=9).map(|number| {
            let label = match number {
                9 => L10N.tr("show-last-tab"),
                number => format!("{} {number}", L10N.tr("show-tab")),
            };
            Entry::item(&label, &format!("{SHOW_TAB}{number}"))
                .shortcut(&format!("Ctrl+{number}"))
                .enabled(tabs > 1)
        }));
        Menus {
            menus: vec![
                Entry::submenu(&L10N.tr("menu-file"), file),
                Entry::submenu(&L10N.tr("menu-view"), page),
                Entry::submenu(&L10N.tr("menu-history"), history),
                Entry::submenu(&L10N.tr("menu-bookmarks"), bookmarks),
            ],
            window,
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        match command {
            NEW_WINDOW => return self.open_window(cx, Session::Default, blank()),
            NEW_PRIVATE_WINDOW => {
                self.next_private += 1;
                let session = Session::Private(self.next_private);
                return self.open_window(cx, session, blank());
            }
            _ => {}
        }
        let Some(window) = window.or_else(|| cx.focused()) else {
            if command == NEW_TAB {
                self.open_window(cx, Session::Default, blank());
            }
            return;
        };
        if command == SYSTEM_BROWSER {
            if let Some(url) = cx.view(window).and_then(BrowserView::current_page) {
                open_in_system_browser(&url);
            }
            return;
        }
        let place = |prefix: &str, places: Vec<Place>| {
            command
                .strip_prefix(prefix)
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| places.into_iter().nth(index))
        };
        let picked = place(RECENT_PAGE, self.places.recent(RECENT_SHOWN))
            .or_else(|| place(BOOKMARK, self.places.bookmarks()));
        if let Some((shown, view)) = cx.window(window) {
            match command {
                _ if picked.is_some() => {
                    if let Some(place) = picked {
                        view.navigate(place.url);
                    }
                }
                NEW_TAB => view.new_tab(),
                SHOW_HISTORY => view.show_history(),
                SHOW_BOOKMARKS => view.show_bookmarks(),
                CLEAR_HISTORY => view.ask_clear_history(),
                WEBSITE_DATA => view.ask_website_data(),
                ADD_BOOKMARK => {
                    view.toggle_bookmark();
                }
                ZOOM_IN => view.zoom(Some(1)),
                ZOOM_OUT => view.zoom(Some(-1)),
                ACTUAL_SIZE => view.zoom(None),
                OPEN_LOCATION => view.focus_address(),
                RELOAD => view.reload(),
                STOP => view.stop(),
                BACK => view.go_back(),
                FORWARD => view.go_forward(),
                REOPEN_TAB => view.reopen_closed_tab(),
                NEXT_TAB => view.step_tab(true),
                PREVIOUS_TAB => view.step_tab(false),
                _ => match command
                    .strip_prefix(SHOW_TAB)
                    .and_then(|number| number.parse().ok())
                {
                    Some(number) => view.select_number(number),
                    None => return,
                },
            }
            shown.request_redraw();
        }
        self.serve(cx, window);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        let urls = addresses(&arguments, self.settings.get());
        match urls.is_empty() {
            true => self.open_window(cx, Session::Default, blank()),
            false => self.open_urls(cx, urls),
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.settings.get().launch == Launch::NewWindow || !self.restore(cx) {
            self.open_window(cx, Session::Default, blank());
        }
        self.ask_search_engine(cx);
    }

    fn save_window(&self, cx: &Cx<Self>, window: WindowId) -> Option<Saved> {
        Saved::of(&session::write(&[cx.view(window)?.saved()?]))
    }

    fn restore_window(&mut self, cx: &mut Cx<Self>, saved: Saved) {
        let windows = saved
            .read::<String>()
            .map(|text| session::parse(&text))
            .unwrap_or_default();
        for window in windows {
            self.open_window(cx, Session::Default, window);
        }
    }

    fn windows_restored(&mut self, cx: &mut Cx<Self>) {
        self.ask_search_engine(cx);
    }

    fn close_tab(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        let closed = cx
            .window(window)
            .is_some_and(|(_, view)| view.close_active_tab());
        if closed {
            self.serve(cx, window);
        }
        closed
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        self.save_session(cx);
        self.quitting = true;
        true
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, _window: WindowId, view: BrowserView) {
        view.close_all();
        if !self.quitting && !self.carry.is_carrying() {
            self.save_session(cx);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.renderer.fd()];
        sources.extend(self.settings_watch.iter().map(SettingsWatch::fd));
        sources
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        if index == SETTINGS_SOURCE {
            if let Some(watch) = &mut self.settings_watch {
                watch.drain();
            }
            if let Some(folder) = settings_folder() {
                self.settings.set(BrowserSettings::load(&folder));
            }
            return;
        }
        let mut touched: Vec<WindowId> = Vec::new();
        for update in self.renderer.updates() {
            let target = cx.windows().find(|window| {
                cx.view(*window)
                    .is_some_and(|view| view.has_tab(update.view))
            });
            if let Some((shown, view)) = target.and_then(|window| cx.window(window)) {
                view.update(update);
                shown.request_redraw();
            }
            if let Some(window) = target
                && !touched.contains(&window)
            {
                touched.push(window);
            }
        }
        for window in touched {
            self.serve(cx, window);
        }
    }
}

fn open_places(file: Option<PathBuf>) -> rusqlite::Result<Places> {
    match file {
        Some(file) => Places::open(&file).or_else(|error| {
            tracing::warn!("cannot open history and bookmarks: {error}");
            Places::in_memory()
        }),
        None => Places::in_memory(),
    }
}

fn menu_label(place: &Place) -> String {
    let label = match place.title.is_empty() {
        true => place.url.as_str(),
        false => place.title.as_str(),
    };
    label.chars().take(MENU_LABEL).collect()
}

fn open_in_system_browser(url: &Url) {
    let apps = bande_xdg::Apps::load(bande_xdg::data_dirs(), &bande_xdg::config_dirs());
    let desktop = format!("{}.desktop", INFO.id);
    let browser = apps
        .apps_for(SCHEME_TYPE)
        .into_iter()
        .find(|app| app.id != desktop);
    let Some(browser) = browser else {
        return tracing::warn!("no other browser is installed");
    };
    let launched = browser
        .command_for_uri(url.as_str())
        .map_err(|error| error.to_string())
        .and_then(|command| {
            bande_xdg::spawn_detached(browser.app_id(), &command, None, None)
                .map_err(|error| error.to_string())
        });
    if let Err(error) = launched {
        tracing::warn!("cannot open {url} in {}: {error}", browser.id);
    }
}

fn main() -> ExitCode {
    bande_app::run::<Browser>(&INFO)
}
