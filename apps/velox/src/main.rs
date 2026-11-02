use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::Entry;
use bande_toolkit::{
    Appearance, FrameRegions, Rect, Scene, WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};
use bande_velox::{Action, Informer, L10N, Request, SIDEBAR_WIDTH, VeloxView};
use bande_xdg::{Apps, config_dirs, data_dirs};

const APP_ID: &str = "org.bande.Velox";
const WINDOW_SIZE: (u32, u32) = (920, 560);
const START: &str = "start";
const STOP: &str = "stop";
const FIND: &str = "find";
const SHOW_MAIN: &str = "show-main";

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

struct Pane {
    view: VeloxView,
    informer: Rc<Informer>,
    apps: Option<Apps>,
}

impl Pane {
    fn serve(&mut self, window: &mut Window) {
        if let Some(request) = self.view.ui().take_clipboard_request()
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot copy: {error}");
        }
        for request in self.view.take_requests() {
            match request {
                Request::Act(engine, action, ids) => self.informer.act(engine, action, ids),
                Request::Relist(engine) => self.informer.relist(engine),
                Request::OpenLink(link) => self.open_link(window, link),
            }
        }
    }

    fn open_link(&mut self, window: &mut Window, link: &str) {
        let token = window.activation_token(APP_ID).ok();
        let entry = self
            .apps
            .get_or_insert_with(|| Apps::load(data_dirs(), &config_dirs()))
            .default_for("x-scheme-handler/https")
            .cloned();
        match entry {
            Some(entry) => {
                if let Err(error) =
                    bande_xdg::launch_with_arguments(&entry, &[link.to_owned()], token.as_deref())
                {
                    tracing::warn!("cannot open {link}: {error}");
                }
            }
            None => tracing::warn!("no app opens web links"),
        }
    }

    fn command(&mut self, window: &mut Window, command: &str) -> bool {
        match command {
            START => self.view.act(Action::Start),
            STOP => self.view.act(Action::Stop),
            FIND => self.view.focus_filter(),
            _ => return false,
        }
        self.serve(window);
        true
    }
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        window.set_frame_regions(FrameRegions {
            sidebar: SIDEBAR_WIDTH,
            toolbar: match full_screen {
                true => WINDOW_HEADER_HEIGHT as f32,
                false => 0.0,
            },
        });
        self.view.set_full_screen(full_screen);
        let scene = View::scene(self.view.ui(), window);
        window.set_header(self.view.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(self.view.ui(), window, event);
        let pumped = self.view.pump();
        self.serve(window);
        handled || pumped
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.view.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.view.ui().set_window_active(active);
        if active {
            self.view.relist();
            for request in self.view.take_requests() {
                if let Request::Relist(engine) = request {
                    self.informer.relist(engine);
                }
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.ui_ref().deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.view.ui_ref().is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.view.ui_ref().accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view.ui_ref().text_input_area()
    }
}

struct Velox {
    main: Option<WindowId>,
    informer: Option<Rc<Informer>>,
}

impl Velox {
    fn show_main(&mut self, cx: &mut Cx<Self>) {
        if let Some(main) = self.main {
            return cx.raise(main, None);
        }
        let informer = match Informer::start() {
            Ok(informer) => Rc::new(informer),
            Err(error) => return tracing::warn!("cannot follow the engine: {error}"),
        };
        let shared = Rc::clone(&informer);
        let opened = cx.open(
            Kind::Main,
            &L10N.tr("pane-containers"),
            WINDOW_SIZE,
            |appearance| Pane {
                view: VeloxView::new(appearance),
                informer: shared,
                apps: None,
            },
        );
        match opened {
            Ok(window) => {
                self.main = Some(window);
                self.informer = Some(informer);
            }
            Err(error) => tracing::warn!("cannot open a window: {error}"),
        }
    }
}

impl Program for Velox {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut velox = Self {
            main: None,
            informer: None,
        };
        velox.show_main(cx);
        Ok(velox)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = self
            .main
            .and_then(|main| cx.view(main))
            .map(|pane| &pane.view);
        let can = |action| view.is_some_and(|view| view.can(action));
        Menus {
            menus: vec![
                Entry::submenu(
                    &L10N.tr("menu-view"),
                    vec![Entry::item(&L10N.tr("find"), FIND).shortcut("Ctrl+F")],
                ),
                Entry::submenu(
                    &L10N.tr("menu-container"),
                    vec![
                        Entry::item(&L10N.tr("action-start"), START).enabled(can(Action::Start)),
                        Entry::item(&L10N.tr("action-stop"), STOP).enabled(can(Action::Stop)),
                    ],
                ),
            ],
            window: vec![Entry::item(&L10N.tr("app-name"), SHOW_MAIN)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        self.show_main(cx);
        if let Some((window, pane)) = self.main.and_then(|main| cx.window(main))
            && pane.command(window, command)
        {
            window.request_redraw();
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.show_main(cx);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.main == Some(window) {
            self.main = None;
            self.informer = None;
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        cx.refresh_menus();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.informer.iter().map(|informer| informer.fd()).collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        let Some(informer) = self.informer.clone() else {
            return;
        };
        let updates = informer.drain();
        if let Some((window, pane)) = self.main.and_then(|main| cx.window(main)) {
            for update in updates {
                pane.view.apply(update);
            }
            pane.serve(window);
            window.request_redraw();
        }
        cx.refresh_menus();
    }
}

fn main() -> ExitCode {
    bande_app::run::<Velox>(&INFO)
}
