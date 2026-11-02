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
    Act, Action, Effect, Hub, Informer, L10N, MACHINE_WINDOW_SIZE, MachineRequest, MachineWindow,
    Pane as Shown, Request, SIDEBAR_WIDTH, VeloxView,
};
use bande_xdg::{Apps, config_dirs, data_dirs, spawn_detached};

const APP_ID: &str = "org.bande.Velox";
const TERMINAL_APP_ID: &str = "org.bande.Terminal";
const TERMINAL: &str = "bande-terminal";
const WINDOW_SIZE: (u32, u32) = (920, 560);
const START: &str = "start";
const STOP: &str = "stop";
const FIND: &str = "find";
const SHOW_MAIN: &str = "show-main";
const NEW_MACHINE: &str = "new-machine";
const SHOW_MACHINES: &str = "show-machines";
const MACHINE_START: &str = "machine-start";
const MACHINE_SHUT_DOWN: &str = "machine-shut-down";
const MACHINE_FORCE_STOP: &str = "machine-force-stop";
const MACHINE_RESTART: &str = "machine-restart";
const MACHINE_CONNECT: &str = "machine-connect";
const MACHINE_DELETE: &str = "machine-delete";
const MACHINE_INSPECTOR: &str = "machine-inspector";
const MACHINE_WINDOW_TAG: &str = "machine";

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
            NEW_MACHINE => {
                self.view.navigate(Shown::Machines);
                let (machines, ui) = self.view.machines();
                machines.new_machine(ui);
            }
            SHOW_MACHINES => self.view.navigate(Shown::Machines),
            MACHINE_CONNECT => self.view.machines().0.connect(),
            MACHINE_INSPECTOR => {
                let (machines, ui) = self.view.machines();
                machines.toggle_inspector(ui);
            }
            _ => match machine_act(command) {
                Some(act) => {
                    let (machines, ui) = self.view.machines();
                    machines.act(ui, act);
                }
                None => return false,
            },
        }
        self.view.machines_changed();
        self.serve(window);
        true
    }
}

fn machine_act(command: &str) -> Option<Act> {
    Some(match command {
        MACHINE_START => Act::Start,
        MACHINE_SHUT_DOWN => Act::ShutDown,
        MACHINE_FORCE_STOP => Act::ForceStop,
        MACHINE_RESTART => Act::Restart,
        MACHINE_DELETE => Act::Delete,
        _ => return None,
    })
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
        window.set_title(&self.view.title());
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
            self.view.machines_changed();
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
    Machine(Box<MachineWindow>),
}

impl Screen {
    fn view(&mut self) -> &mut dyn View {
        match self {
            Self::Main(pane) => pane.as_mut(),
            Self::Machine(machine) => machine.as_mut(),
        }
    }

    fn view_ref(&self) -> &dyn View {
        match self {
            Self::Main(pane) => pane.as_ref(),
            Self::Machine(machine) => machine.as_ref(),
        }
    }

    fn requests(&mut self) -> Vec<MachineRequest> {
        match self {
            Self::Main(pane) => pane.view.machines().0.take_requests(),
            Self::Machine(machine) => machine.take_requests(),
        }
    }

    fn machines_changed(&mut self) {
        match self {
            Self::Main(pane) => pane.view.machines_changed(),
            Self::Machine(machine) => machine.refresh(),
        }
    }

    fn present_notice(&mut self, title: &str, message: &str) {
        match self {
            Self::Main(pane) => pane.view.present_notice(title, message),
            Self::Machine(machine) => machine.present_notice(title, message),
        }
    }

    fn wants_stats(&self) -> bool {
        match self {
            Self::Main(pane) => pane.view.pane() == Shown::Machines,
            Self::Machine(machine) => machine.wants_stats(),
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
    hub: Option<Hub>,
}

impl Velox {
    fn hub(&mut self) -> Option<&mut Hub> {
        if self.hub.is_none() {
            match Hub::start() {
                Ok(hub) => self.hub = Some(hub),
                Err(error) => tracing::warn!("cannot follow the virtual machines: {error}"),
            }
        }
        self.hub.as_mut()
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
        let hub = self.hub().map(|hub| hub as &Hub);
        let opened = cx.open(
            Kind::Main,
            &L10N.tr("pane-containers"),
            WINDOW_SIZE,
            |appearance| {
                let mut view = VeloxView::new(appearance);
                if let Some(hub) = hub {
                    view.set_hub(hub);
                }
                Screen::Main(Box::new(Pane {
                    view,
                    informer: shared,
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

    fn machine_window(&self, cx: &Cx<Self>, id: &str) -> Option<WindowId> {
        cx.windows().find(|window| {
            matches!(cx.view(*window), Some(Screen::Machine(machine)) if machine.machine() == id)
        })
    }

    fn open_machine(&mut self, cx: &mut Cx<Self>, id: &str) {
        if let Some(window) = self.machine_window(cx, id) {
            return cx.raise(window, None);
        }
        let Some(hub) = self.hub.as_ref() else {
            return;
        };
        let opened = cx.open(
            Kind::Secondary(MACHINE_WINDOW_TAG),
            &L10N.tr("machine-pane"),
            MACHINE_WINDOW_SIZE,
            |appearance| Screen::Machine(Box::new(MachineWindow::new(id, hub, appearance))),
        );
        if let Err(error) = opened {
            tracing::warn!("cannot open the machine's window: {error}");
        }
        self.update_sampling(cx);
    }

    fn connect(cx: &mut Cx<Self>, window: WindowId, command: Vec<String>) {
        let token = cx
            .window(window)
            .and_then(|(window, _)| window.activation_token(APP_ID).ok());
        let mut argv = vec![TERMINAL.to_owned(), "-e".to_owned()];
        argv.extend(command);
        if let Err(error) = spawn_detached(TERMINAL_APP_ID, &argv, None, token.as_deref()) {
            tracing::warn!("cannot open Terminal: {error}");
        }
    }

    fn serve_machines(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let requests = cx
            .window(window)
            .map(|(_, screen)| screen.requests())
            .unwrap_or_default();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            let Some(hub) = self.hub.as_mut() else {
                return;
            };
            match hub.serve(request) {
                Some(Effect::Open(id)) => self.open_machine(cx, &id),
                Some(Effect::Connect(command)) => Self::connect(cx, window, command),
                None => {}
            }
        }
        self.machines_changed(cx);
    }

    fn machines_changed(&mut self, cx: &mut Cx<Self>) {
        let notices = self
            .hub
            .as_ref()
            .map(|hub| hub.take_notices())
            .unwrap_or_default();
        for notice in notices {
            let target = notice
                .id
                .as_deref()
                .and_then(|id| self.machine_window(cx, id))
                .or(self.main);
            if let Some((_, screen)) = target.and_then(|target| cx.window(target)) {
                screen.present_notice(&notice.title, &notice.message);
            }
        }
        let windows: Vec<WindowId> = cx.windows().collect();
        for id in windows {
            let gone = matches!(cx.view(id), Some(Screen::Machine(machine)) if machine.gone());
            if gone {
                cx.close(id);
                continue;
            }
            if let Some((window, screen)) = cx.window(id) {
                screen.machines_changed();
                window.request_redraw();
            }
        }
        self.update_sampling(cx);
        cx.refresh_menus();
    }

    fn update_sampling(&mut self, cx: &Cx<Self>) {
        let wanted = cx
            .windows()
            .filter_map(|window| cx.view(window))
            .any(Screen::wants_stats);
        if let Some(hub) = self.hub.as_mut() {
            hub.schedule(wanted);
        }
    }

    fn machine_menu(&self, cx: &Cx<Self>, focused: Option<WindowId>) -> Entry {
        let screen = focused
            .and_then(|window| cx.view(window))
            .or_else(|| self.main.and_then(|main| cx.view(main)));
        let can = |act: Act| match screen {
            Some(Screen::Main(pane)) => pane.view.machines_ref().can(act),
            Some(Screen::Machine(machine)) => machine.can(act),
            None => false,
        };
        let connect = match screen {
            Some(Screen::Main(pane)) => pane.view.machines_ref().can_connect(),
            Some(Screen::Machine(machine)) => machine.can_connect(),
            None => false,
        };
        let item = |key: &str, command: &str, act: Act| {
            Entry::item(&L10N.tr(key), command).enabled(can(act))
        };
        Entry::submenu(
            &L10N.tr("machine-menu"),
            vec![
                item("action-start", MACHINE_START, Act::Start),
                item("machine-shut-down", MACHINE_SHUT_DOWN, Act::ShutDown),
                item("machine-force-stop", MACHINE_FORCE_STOP, Act::ForceStop),
                item("machine-restart", MACHINE_RESTART, Act::Restart),
                Entry::item(&L10N.tr("machine-connect"), MACHINE_CONNECT).enabled(connect),
                item("machine-delete-ellipsis", MACHINE_DELETE, Act::Delete)
                    .shortcut("Ctrl+Backspace"),
            ],
        )
    }
}

impl Program for Velox {
    type View = Screen;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut velox = Self {
            main: None,
            informer: None,
            hub: None,
        };
        velox.show_main(cx);
        Ok(velox)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = self
            .main
            .and_then(|main| cx.view(main))
            .and_then(|screen| match screen {
                Screen::Main(pane) => Some(&pane.view),
                Screen::Machine(_) => None,
            });
        let can = |action| view.is_some_and(|view| view.can(action));
        let can_create = view.is_some_and(|view| view.machines_ref().can_create());
        Menus {
            menus: vec![
                Entry::submenu(
                    &L10N.tr("menu-file"),
                    vec![
                        Entry::item(&L10N.tr("machine-new-ellipsis"), NEW_MACHINE)
                            .shortcut("Ctrl+N")
                            .enabled(can_create),
                    ],
                ),
                Entry::submenu(
                    &L10N.tr("menu-view"),
                    vec![
                        Entry::item(&L10N.tr("machine-pane"), SHOW_MACHINES).shortcut("Ctrl+6"),
                        Entry::item(&L10N.tr("machine-inspector"), MACHINE_INSPECTOR)
                            .shortcut("Ctrl+I"),
                        Entry::item(&L10N.tr("find"), FIND).shortcut("Ctrl+F"),
                    ],
                ),
                Entry::submenu(
                    &L10N.tr("menu-container"),
                    vec![
                        Entry::item(&L10N.tr("action-start"), START).enabled(can(Action::Start)),
                        Entry::item(&L10N.tr("action-stop"), STOP).enabled(can(Action::Stop)),
                    ],
                ),
                self.machine_menu(cx, cx.focused()),
            ],
            window: vec![Entry::item(&L10N.tr("app-name"), SHOW_MAIN)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if let Some(window) = window
            && Some(window) != self.main
            && let Some((shown, Screen::Machine(machine))) = cx.window(window)
        {
            match (command, machine_act(command)) {
                (MACHINE_CONNECT, _) => machine.connect(),
                (_, Some(act)) => machine.act(act),
                _ => {}
            }
            shown.request_redraw();
            self.serve_machines(cx, window);
            if machine_act(command).is_some() || command == MACHINE_CONNECT {
                return;
            }
        }
        self.show_main(cx);
        let Some(main) = self.main else {
            return;
        };
        if let Some((window, Screen::Main(pane))) = cx.window(main)
            && pane.command(window, command)
        {
            window.request_redraw();
        }
        self.serve_machines(cx, main);
        self.update_sampling(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.show_main(cx);
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, _view: Screen) {
        if self.main == Some(window) {
            self.main = None;
            self.informer = None;
        }
        if cx.windows().next().is_none() {
            self.hub = None;
        } else {
            self.update_sampling(cx);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve_machines(cx, window);
        self.update_sampling(cx);
        cx.refresh_menus();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.informer
            .iter()
            .map(|informer| informer.fd())
            .chain(self.hub.iter().map(Hub::fd))
            .collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        let informer = match (index, &self.informer) {
            (0, Some(informer)) => Some(Rc::clone(informer)),
            _ => None,
        };
        let Some(informer) = informer else {
            if self.hub.as_ref().is_some_and(Hub::drain) {
                self.machines_changed(cx);
            }
            return;
        };
        let updates = informer.drain();
        if let Some((window, Screen::Main(pane))) = self.main.and_then(|main| cx.window(main)) {
            for update in updates {
                pane.view.apply(update);
            }
            pane.serve(window);
            window.request_redraw();
        }
        cx.refresh_menus();
    }

    fn deadline(&self) -> Option<Instant> {
        self.hub.as_ref().and_then(Hub::deadline)
    }

    fn tick(&mut self, cx: &mut Cx<Self>, now: Instant) {
        if self.hub.as_mut().is_some_and(|hub| hub.tick(now)) {
            self.machines_changed(cx);
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Velox>(&INFO)
}
