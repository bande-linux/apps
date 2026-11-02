mod address;
mod page;
mod view;

use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::LazyLock;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, WindowId};
use bande_app_menu::Entry;
use bande_l10n::Localizer;
use web_content::Fonts;
use web_net::{Fetcher, Url};

use crate::view::BrowserView;

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
const NEW_WINDOW: &str = "new-window";
const OPEN_LOCATION: &str = "open-location";
const RELOAD: &str = "reload";
const STOP: &str = "stop";
const BACK: &str = "back";
const FORWARD: &str = "forward";

struct Browser {
    fetcher: Fetcher,
    fonts: Rc<RefCell<Fonts>>,
}

fn addresses(arguments: &[String]) -> Vec<Url> {
    arguments
        .iter()
        .filter_map(|argument| address::parse(argument))
        .collect()
}

impl Browser {
    fn open_window(&mut self, cx: &mut Cx<Self>, url: Option<Url>) {
        let opened = cx.open(
            Kind::Main,
            &L10N.tr("app-name"),
            WINDOW_SIZE,
            |appearance| BrowserView::new(appearance, Rc::clone(&self.fonts)),
        );
        let window = match opened {
            Ok(window) => window,
            Err(error) => return tracing::warn!("cannot open a window: {error}"),
        };
        if let Some((_, view)) = cx.window(window) {
            match url {
                Some(url) => view.navigate(url),
                None => view.focus_address(),
            }
        }
        self.changed(cx, window);
    }

    fn open_all(&mut self, cx: &mut Cx<Self>, urls: Vec<Url>) {
        if urls.is_empty() {
            return self.open_window(cx, None);
        }
        for url in urls {
            self.open_window(cx, Some(url));
        }
    }
}

impl Program for Browser {
    type View = BrowserView;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut browser = Self {
            fetcher: Fetcher::new()?,
            fonts: Rc::default(),
        };
        browser.open_all(cx, addresses(&arguments));
        Ok(browser)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = cx.focused().and_then(|window| cx.view(window));
        let file = vec![
            Entry::item(&L10N.tr("new-window"), NEW_WINDOW).shortcut("Ctrl+N"),
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
        ];
        Menus {
            menus: vec![
                Entry::submenu(&L10N.tr("menu-file"), file),
                Entry::submenu(&L10N.tr("menu-view"), page),
                Entry::submenu(&L10N.tr("menu-history"), history),
            ],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if command == NEW_WINDOW {
            return self.open_window(cx, None);
        }
        let Some(window) = window.or_else(|| cx.focused()) else {
            return;
        };
        if let Some((shown, view)) = cx.window(window) {
            match command {
                OPEN_LOCATION => view.focus_address(),
                RELOAD => view.reload(),
                STOP => view.stop(),
                BACK => view.go_back(),
                FORWARD => view.go_forward(),
                _ => return,
            }
            shown.request_redraw();
        }
        self.changed(cx, window);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        self.open_all(cx, addresses(&arguments));
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.open_window(cx, None);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, view: BrowserView) {
        if let Some(fetch) = view.fetch() {
            self.fetcher.cancel(fetch);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let Some((shown, view)) = cx.window(window) else {
            return;
        };
        let Some(request) = view.take_request() else {
            return;
        };
        if let Some(previous) = request.cancel {
            self.fetcher.cancel(previous);
        }
        if let Some(url) = request.load {
            view.loading(self.fetcher.fetch(url));
        }
        shown.request_redraw();
        cx.refresh_menus();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.fetcher.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        for (fetch, result) in self.fetcher.completed() {
            let fetching = cx
                .windows()
                .find(|window| cx.view(*window).and_then(BrowserView::fetch) == Some(fetch));
            let Some(window) = fetching else {
                continue;
            };
            if let Some((shown, view)) = cx.window(window) {
                view.loaded(result);
                shown.request_redraw();
            }
            self.changed(cx, window);
        }
        cx.refresh_menus();
    }
}

fn main() -> ExitCode {
    bande_app::run::<Browser>(&INFO)
}
