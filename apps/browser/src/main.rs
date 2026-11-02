mod address;
mod page;
mod renderer;
mod session;
mod tiles;
mod view;

use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::LazyLock;

use bande_app::{Cx, Detached, Info, Keys, Kind, Menus, Program, TabCarry, TabDrop, WindowId};
use bande_app_menu::Entry;
use bande_l10n::Localizer;
use web_net::Url;

use crate::renderer::{Renderer, Session};
use crate::view::{Ask, BrowserView, CarriedTab, SavedWindow};

const ENGLISH: &str = include_str!("../locales/en-US/bande-browser.ftl");
static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

static INFO: Info = Info {
    id: "org.bande.Browser",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

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

struct Browser {
    renderer: Renderer,
    ids: Rc<Cell<u64>>,
    next_private: u32,
    carry: TabCarry,
    quitting: bool,
    carried_session: Session,
    session_file: Option<PathBuf>,
}

fn addresses(arguments: &[String]) -> Vec<Url> {
    arguments
        .iter()
        .filter_map(|argument| address::parse(argument))
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
        let (renderer, ids) = (self.renderer.link(), Rc::clone(&self.ids));
        Ok(
            cx.open(Kind::Main, &L10N.tr("app-name"), size, move |appearance| {
                BrowserView::new(appearance, session, renderer, ids)
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
                view.open_tab(None, true);
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
        let mut browser = Self {
            renderer: Renderer::new()?,
            ids: Rc::default(),
            next_private: 0,
            carry: TabCarry::default(),
            quitting: false,
            carried_session: Session::Default,
            session_file: cx.state_file(SESSION_FILE),
        };
        let urls = addresses(&arguments);
        if !urls.is_empty() {
            browser.open_urls(cx, urls);
        } else if !browser.restore(cx) {
            browser.open_window(cx, Session::Default, blank());
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
        let page = vec![
            Entry::item(&L10N.tr("reload"), RELOAD).shortcut("Ctrl+R"),
            Entry::item(&L10N.tr("stop"), STOP)
                .shortcut("Ctrl+.")
                .enabled(view.is_some_and(BrowserView::is_loading)),
        ];
        let history = vec![
            Entry::item(&L10N.tr("back"), BACK)
                .shortcut("Ctrl+[")
                .enabled(view.is_some_and(BrowserView::can_go_back)),
            Entry::item(&L10N.tr("forward"), FORWARD)
                .shortcut("Ctrl+]")
                .enabled(view.is_some_and(BrowserView::can_go_forward)),
            Entry::item(&L10N.tr("reopen-tab"), REOPEN_TAB)
                .shortcut("Shift+Ctrl+T")
                .enabled(view.is_some_and(BrowserView::can_reopen_closed_tab)),
        ];
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
        if let Some((shown, view)) = cx.window(window) {
            match command {
                NEW_TAB => view.new_tab(),
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
        let urls = addresses(&arguments);
        match urls.is_empty() {
            true => self.open_window(cx, Session::Default, blank()),
            false => self.open_urls(cx, urls),
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.open_window(cx, Session::Default, blank());
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
        vec![self.renderer.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
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

fn main() -> ExitCode {
    bande_app::run::<Browser>(&INFO)
}
