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
use bande_velox::{
    Informer, L10N, LogsTarget, LogsView, Request, SIDEBAR_WIDTH, Update, VeloxView, Work,
};
use bande_xdg::{Apps, config_dirs, data_dirs};

const APP_ID: &str = "org.bande.Velox";
const WINDOW_SIZE: (u32, u32) = (920, 560);
const LOGS_SIZE: (u32, u32) = (760, 480);
const FIND: &str = "find";
const SHOW_MAIN: &str = "show-main";
const SHOW_LOGS: &str = "show-logs:";

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
    work: Work,
    logs: Vec<LogsTarget>,
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
                Request::OpenLink(link) => self.open_link(window, &link),
                Request::Containers(request) => {
                    let token = || window.activation_token(APP_ID).ok();
                    self.logs.extend(self.work.serve(request, token));
                }
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
            FIND => self.view.focus_filter(),
            command if self.view.command(command) => {}
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
        self.view.sync_streams();
        self.serve(window);
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::ToplevelChanged(state) = &event {
            self.view.set_hidden(state.suspended);
            self.view.sync_streams();
        }
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

enum Screen {
    Main(Box<Pane>),
    Logs(Box<LogsView>),
}

impl Screen {
    fn view(&mut self) -> &mut dyn View {
        match self {
            Self::Main(pane) => pane.as_mut(),
            Self::Logs(logs) => logs.as_mut(),
        }
    }

    fn view_ref(&self) -> &dyn View {
        match self {
            Self::Main(pane) => pane.as_ref(),
            Self::Logs(logs) => logs.as_ref(),
        }
    }
}

impl View for Screen {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.view().scene(window)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        self.view().event(window, event)
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.view().set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.view().set_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.view_ref().sources()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) -> bool {
        self.view().source_ready(window, index)
    }

    fn deadline(&self) -> Option<Instant> {
        self.view_ref().deadline()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) -> bool {
        self.view().tick(window, now)
    }

    fn animating(&self) -> bool {
        self.view_ref().animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        self.view_ref().accessibility()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view_ref().text_input_area()
    }
}

struct Velox {
    main: Option<WindowId>,
    informer: Option<Rc<Informer>>,
}

impl Velox {
    fn pane<'a>(&self, cx: &'a mut Cx<Self>) -> Option<(&'a mut Window, &'a mut Pane)> {
        match cx.window(self.main?)? {
            (window, Screen::Main(pane)) => Some((window, pane)),
            _ => None,
        }
    }

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
            |appearance| {
                Screen::Main(Box::new(Pane {
                    view: VeloxView::new(appearance),
                    work: Work::new(&shared),
                    informer: shared,
                    logs: Vec::new(),
                    apps: None,
                }))
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

    fn open_logs(&mut self, cx: &mut Cx<Self>) {
        let wanted = self
            .pane(cx)
            .map(|(_, pane)| std::mem::take(&mut pane.logs))
            .unwrap_or_default();
        for target in wanted {
            let shown = cx.windows().find(|window| {
                matches!(cx.view(*window), Some(Screen::Logs(logs)) if logs.target().key() == target.key())
            });
            if let Some(window) = shown {
                cx.raise(window, None);
                continue;
            }
            let title = target.title();
            if let Err(error) = cx.open(Kind::Secondary("logs"), &title, LOGS_SIZE, |appearance| {
                Screen::Logs(Box::new(LogsView::new(appearance, target)))
            }) {
                tracing::warn!("cannot open the logs: {error}");
            }
        }
    }
}

impl Program for Velox {
    type View = Screen;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut velox = Self {
            main: None,
            informer: None,
        };
        velox.show_main(cx);
        Ok(velox)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = match self.main.and_then(|main| cx.view(main)) {
            Some(Screen::Main(pane)) => Some(&pane.view),
            _ => None,
        };
        let mut shown = vec![Entry::item(&L10N.tr("find"), FIND).shortcut("Ctrl+F")];
        shown.extend(view.map(VeloxView::view_entries).unwrap_or_default());
        let mut window = vec![Entry::item(&L10N.tr("app-name"), SHOW_MAIN)];
        window.extend(cx.windows().filter_map(|id| match cx.view(id) {
            Some(Screen::Logs(logs)) => Some(Entry::item(
                &logs.target().title(),
                &format!("{SHOW_LOGS}{}", logs.target().key()),
            )),
            _ => None,
        }));
        Menus {
            menus: vec![
                Entry::submenu(&L10N.tr("menu-view"), shown),
                Entry::submenu(
                    &L10N.tr("menu-container"),
                    view.map(VeloxView::container_menu).unwrap_or_default(),
                ),
            ],
            window,
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        if let Some(key) = command.strip_prefix(SHOW_LOGS) {
            let shown = cx.windows().find(|window| {
                matches!(cx.view(*window), Some(Screen::Logs(logs)) if logs.target().key() == key)
            });
            if let Some(window) = shown {
                cx.raise(window, None);
            }
            return;
        }
        self.show_main(cx);
        if let Some((window, pane)) = self.pane(cx)
            && pane.command(window, command)
        {
            window.request_redraw();
        }
        self.open_logs(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.show_main(cx);
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, _view: Screen) {
        if self.main == Some(window) {
            self.main = None;
        }
        if cx.windows().next().is_none() {
            self.informer = None;
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.open_logs(cx);
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
        let windows: Vec<WindowId> = cx.windows().collect();
        for update in &updates {
            let Update::Changed(container) = update else {
                continue;
            };
            for id in &windows {
                if let Some((window, Screen::Logs(logs))) = cx.window(*id)
                    && logs.container_changed(container)
                {
                    window.request_redraw();
                }
            }
        }
        if let Some((window, pane)) = self.pane(cx) {
            for update in updates {
                pane.view.apply(update);
            }
            pane.serve(window);
            window.request_redraw();
        }
        self.open_logs(cx);
        cx.refresh_menus();
    }
}

fn main() -> ExitCode {
    bande_app::run::<Velox>(&INFO)
}
