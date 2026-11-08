use std::collections::HashSet;
use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use bande_l10n::{Text, text, tr};
use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_menu::Entry;
use bande_app_settings::{SettingsWindow, settings_folder};
use bande_toolkit::{
    Appearance, FrameRegions, Rect, Scene, WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};
use bande_velox::{
    Act, ContainerRequest, DisplayWindow, Effect, Hub, Informer, LogsTarget, LogsView,
    MACHINE_WINDOW_SIZE, MachineRequest, MachineWindow, Request, SETTINGS_SIZE, SIDEBAR_WIDTH,
    Section, ShellLink, ShellTarget, ShellWindow, TERMINAL_APP_ID, Update, VeloxSettings,
    VeloxView, Work, notify_crash, settings_form,
};
use bande_xdg::{Apps, config_dirs, data_dirs, spawn_detached};

const APP_ID: &str = "org.bande.Velox";
const RUNNING_ID: &str = match option_env!("VELOX_TEST_APP_ID") {
    Some(id) => id,
    None => APP_ID,
};
const WINDOW_SIZE: (u32, u32) = (920, 560);
const LOGS_SIZE: (u32, u32) = (760, 480);
const FIND: &str = "find";
const SHOW_MAIN: &str = "show-main";
const SHOW_LOGS: &str = "show-logs:";
const SHOW_MACHINE: &str = "show-machine:";
const VIEW_KEYS: [&str; 6] = ["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6"];
const TERMINAL: &str = "bande-terminal";
const NEW_MACHINE: &str = "new-machine";
const MACHINE_START: &str = "machine-start";
const MACHINE_SHUT_DOWN: &str = "machine-shut-down";
const MACHINE_FORCE_STOP: &str = "machine-force-stop";
const MACHINE_RESTART: &str = "machine-restart";
const MACHINE_CONNECT: &str = "machine-connect";
const MACHINE_DELETE: &str = "machine-delete";
const MACHINE_SEND_KEYS: &str = "machine-send-ctrl-alt-delete";
const MACHINE_PAUSE: &str = "machine-pause";
const MACHINE_SNAPSHOT: &str = "machine-take-snapshot";
const MACHINE_CLONE: &str = "machine-clone";
const MACHINE_OPEN: &str = "machine-open-console";
const MACHINE_SERIAL: &str = "machine-serial";
const MACHINE_SUSPEND: &str = "machine-suspend";
const MACHINE_USB: &str = "machine-usb:";
const SHOW_SHELL: &str = "show-shell:";
const SHELL_SIZE: (u32, u32) = (760, 480);
const MACHINE_WINDOW_TAG: &str = "machine";
const DISPLAY_WINDOW_TAG: &str = "display";

static INFO: Info = Info {
    id: RUNNING_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Velox"),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || tr!("Velox Settings"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsWindow::<VeloxSettings>::new(
        appearance,
        settings_folder(),
        settings_form,
    ))
}

struct Pane {
    view: VeloxView,
    informer: Rc<Informer>,
    work: Work,
    logs: Vec<LogsTarget>,
    shells: Vec<ShellTarget>,
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
                Request::Task(engine, api, task) => self.informer.run(engine, api, task),
                Request::Watch(engine, ids) => self.informer.watch("overview", &engine, ids),
                Request::Containers(ContainerRequest::Shell(target)) => self.shells.push(target),
                Request::Containers(request) => {
                    let token = || window.activation_token(RUNNING_ID).ok();
                    self.logs.extend(self.work.serve(request, token));
                }
            }
        }
    }

    fn open_link(&mut self, window: &mut Window, link: &str) {
        let token = window.activation_token(RUNNING_ID).ok();
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
            NEW_MACHINE => {
                self.view.show(Section::Machines);
                let (machines, ui) = self.view.machines();
                machines.new_machine(ui);
            }
            MACHINE_CONNECT => self.view.machines().0.connect(),
            MACHINE_PAUSE => {
                let (machines, ui) = self.view.machines();
                let act = match machines.can(Act::Resume) {
                    true => Act::Resume,
                    false => Act::Pause,
                };
                machines.act(ui, act);
            }
            MACHINE_SNAPSHOT => self
                .view
                .machines()
                .0
                .present_for_selected(MachineRequest::PresentSnapshot),
            MACHINE_CLONE => self
                .view
                .machines()
                .0
                .present_for_selected(MachineRequest::PresentClone),
            MACHINE_SERIAL => self
                .view
                .machines()
                .0
                .present_for_selected(MachineRequest::Serial),
            MACHINE_OPEN => self
                .view
                .machines()
                .0
                .present_for_selected(MachineRequest::Open),
            bande_velox::MACHINE_INSPECTOR => {
                let (machines, ui) = self.view.machines();
                machines.toggle_inspector(ui);
            }
            _ if machine_act(command).is_some() => {
                if let Some(act) = machine_act(command) {
                    let (machines, ui) = self.view.machines();
                    machines.act(ui, act);
                }
            }
            _ if self.view.command(command) => {}
            _ => return false,
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
        MACHINE_SUSPEND => Act::Suspend,
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
        let scene = View::scene(self.view.ui(), window);
        window.set_title(&self.view.title());
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
            self.view.machines_changed();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        match (self.view.ui_ref().deadline(), self.view.deadline()) {
            (Some(ui), Some(view)) => Some(ui.min(view)),
            (ui, view) => ui.or(view),
        }
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        self.view.tick(now);
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
    Machine(Box<MachineWindow>),
    Display(Box<DisplayWindow>),
    Shell(Box<ShellWindow>),
}

impl Screen {
    fn view(&mut self) -> &mut dyn View {
        match self {
            Self::Main(pane) => pane.as_mut(),
            Self::Logs(logs) => logs.as_mut(),
            Self::Machine(machine) => machine.as_mut(),
            Self::Display(display) => display.as_mut(),
            Self::Shell(shell) => shell.as_mut(),
        }
    }

    fn view_ref(&self) -> &dyn View {
        match self {
            Self::Main(pane) => pane.as_ref(),
            Self::Logs(logs) => logs.as_ref(),
            Self::Machine(machine) => machine.as_ref(),
            Self::Display(display) => display.as_ref(),
            Self::Shell(shell) => shell.as_ref(),
        }
    }

    fn machine_requests(&mut self) -> Vec<MachineRequest> {
        match self {
            Self::Main(pane) => pane.view.machines().0.take_requests(),
            Self::Machine(machine) => machine.take_requests(),
            Self::Logs(_) | Self::Display(_) | Self::Shell(_) => Vec::new(),
        }
    }

    fn machines_changed(&mut self) -> bool {
        match self {
            Self::Main(pane) => pane.view.machines_changed(),
            Self::Machine(machine) => machine.refresh(),
            Self::Logs(_) | Self::Display(_) | Self::Shell(_) => return false,
        }
        true
    }

    fn present_notice(&mut self, title: &str, message: &str) {
        match self {
            Self::Main(pane) => pane.view.present_notice(title, message),
            Self::Machine(machine) => machine.present_notice(title, message),
            Self::Logs(_) | Self::Display(_) | Self::Shell(_) => {}
        }
    }

    fn wants_stats(&self) -> bool {
        match self {
            Self::Main(pane) => {
                matches!(pane.view.section(), Section::Machines | Section::Overview)
            }
            Self::Machine(machine) => machine.wants_stats(),
            Self::Logs(_) | Self::Display(_) | Self::Shell(_) => false,
        }
    }
}

impl View for Screen {
    fn keys(&self) -> Option<Keys> {
        self.view_ref().keys()
    }

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
    hub: Option<Hub>,
    dismissed: HashSet<(String, u32)>,
}

impl Velox {
    fn hub(&mut self) -> Option<&Hub> {
        if self.hub.is_none() {
            match Hub::start() {
                Ok(hub) => self.hub = Some(hub),
                Err(error) => tracing::warn!("cannot follow the virtual machines: {error}"),
            }
        }
        self.hub.as_ref()
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
        let terminal = cx.installed(TERMINAL_APP_ID);
        let opened = cx.open(
            Kind::Secondary(MACHINE_WINDOW_TAG),
            &tr!("Machines"),
            MACHINE_WINDOW_SIZE,
            |appearance| {
                Screen::Machine(Box::new(MachineWindow::new(id, hub, appearance, terminal)))
            },
        );
        if let Err(error) = opened {
            tracing::warn!("cannot open the machine's window: {error}");
        }
        self.sync_displays(cx);
    }

    fn with_machine(&self, cx: &mut Cx<Self>, id: &str, act: impl FnOnce(&mut MachineWindow)) {
        if let Some(window) = self.machine_window(cx, id)
            && let Some((shown, Screen::Machine(machine))) = cx.window(window)
        {
            act(machine);
            shown.request_redraw();
        }
    }

    fn open_shell(cx: &mut Cx<Self>, target: ShellTarget) {
        let shown = cx.windows().find(|window| {
            matches!(cx.view(*window), Some(Screen::Shell(shell)) if shell.target() == &target)
        });
        if let Some(window) = shown {
            return cx.raise(window, None);
        }
        let title = target.title();
        let link = match ShellLink::connect(target) {
            Ok(link) => link,
            Err(error) => return tracing::warn!("cannot open the shell: {error}"),
        };
        let opened = cx.open(Kind::Secondary("shell"), &title, SHELL_SIZE, |appearance| {
            Screen::Shell(Box::new(ShellWindow::new(link, appearance)))
        });
        if let Err(error) = opened {
            tracing::warn!("cannot open the shell: {error}");
        }
    }

    fn connect(cx: &mut Cx<Self>, window: WindowId, command: Vec<String>) {
        let token = cx
            .window(window)
            .and_then(|(window, _)| window.activation_token(RUNNING_ID).ok());
        let mut argv = vec![TERMINAL.to_owned(), "-e".to_owned()];
        argv.extend(command);
        if let Err(error) = spawn_detached(TERMINAL_APP_ID, &argv, None, token.as_deref()) {
            tracing::warn!("cannot open Terminal: {error}");
        }
    }

    fn serve_machines(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let requests = cx
            .window(window)
            .map(|(_, screen)| screen.machine_requests())
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
                Some(Effect::Serial(target)) => Self::open_shell(cx, target),
                Some(Effect::PresentClone(id)) => {
                    self.open_machine(cx, &id);
                    self.with_machine(cx, &id, |machine| machine.present_clone(None));
                }
                Some(Effect::PresentSnapshot(id)) => {
                    self.open_machine(cx, &id);
                    self.with_machine(cx, &id, MachineWindow::present_take_snapshot);
                }
                Some(Effect::Connect(command)) => Self::connect(cx, window, command),
                None => {}
            }
        }
        self.machines_changed(cx);
    }

    fn machines_changed(&mut self, cx: &mut Cx<Self>) {
        let to_open = self
            .hub
            .as_ref()
            .map(Hub::take_windows_to_open)
            .unwrap_or_default();
        for id in to_open {
            self.open_machine(cx, &id);
        }
        let notices = self.hub.as_ref().map(Hub::take_notices).unwrap_or_default();
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
            if matches!(cx.view(id), Some(Screen::Machine(machine)) if machine.gone()) {
                cx.close(id);
                continue;
            }
            if let Some((window, screen)) = cx.window(id)
                && screen.machines_changed()
            {
                window.request_redraw();
            }
        }
        self.sync_displays(cx);
        self.update_sampling(cx);
        cx.refresh_menus();
    }

    fn wanted_displays(cx: &Cx<Self>) -> Vec<(String, u32, String)> {
        cx.windows()
            .filter_map(|window| match cx.view(window) {
                Some(Screen::Machine(machine)) => Some(machine),
                _ => None,
            })
            .flat_map(|machine| {
                let (name, count) = machine.displays();
                let id = machine.machine().to_owned();
                (1..count).map(move |display| (id.clone(), display, name.clone()))
            })
            .collect()
    }

    fn sync_displays(&mut self, cx: &mut Cx<Self>) {
        let wanted = Self::wanted_displays(cx);
        let is_wanted =
            |id: &str, display: u32| wanted.iter().any(|(w, d, _)| w == id && *d == display);
        self.dismissed
            .retain(|(id, display)| is_wanted(id, *display));
        let shown: Vec<(WindowId, String, u32)> = cx
            .windows()
            .filter_map(|window| match cx.view(window) {
                Some(Screen::Display(display)) => {
                    Some((window, display.machine().to_owned(), display.display()))
                }
                _ => None,
            })
            .collect();
        for (window, id, display) in &shown {
            if !is_wanted(id, *display) {
                cx.close(*window);
            }
        }
        for (id, index, name) in &wanted {
            let open = shown.iter().any(|(_, w, d)| w == id && d == index);
            if open || self.dismissed.contains(&(id.clone(), *index)) {
                continue;
            }
            let opened = cx.open(
                Kind::Secondary(DISPLAY_WINDOW_TAG),
                name,
                MACHINE_WINDOW_SIZE,
                |appearance| {
                    Screen::Display(Box::new(DisplayWindow::new(id, *index, name, appearance)))
                },
            );
            if let Err(error) = opened {
                tracing::warn!("cannot open display {}: {error}", index + 1);
            }
        }
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

    fn machine_menu(&self, cx: &Cx<Self>) -> Entry {
        let screen = cx
            .focused()
            .and_then(|window| cx.view(window))
            .filter(|screen| matches!(screen, Screen::Machine(_)))
            .or_else(|| self.main.and_then(|main| cx.view(main)));
        let can = |act: Act| match screen {
            Some(Screen::Main(pane)) => pane.view.machines_ref().can(act),
            Some(Screen::Machine(machine)) => machine.can(act),
            _ => false,
        };
        let connect = match screen {
            Some(Screen::Main(pane)) => pane.view.machines_ref().can_connect(),
            Some(Screen::Machine(machine)) => machine.can_connect(),
            _ => false,
        };
        let has_console = matches!(screen, Some(Screen::Machine(machine)) if machine.has_console());
        let single = |act: Act| match screen {
            Some(Screen::Main(pane)) => pane.view.machines_ref().single_can(act),
            Some(Screen::Machine(machine)) => machine.can(act),
            _ => false,
        };
        let item =
            |key: Text, command: &str, act: Act| Entry::item(&key.tr(), command).enabled(can(act));
        let subject = match screen {
            Some(Screen::Main(pane)) => pane.view.machines_ref().single_selected(),
            Some(Screen::Machine(machine)) => Some(machine.machine().to_owned()),
            _ => None,
        };
        let usb = subject.as_deref().and_then(|id| {
            let choices = self.hub.as_ref()?.usb_choices(id)?;
            Some(
                choices
                    .into_iter()
                    .map(|(key, name, on)| {
                        Entry::item(&name, &format!("{MACHINE_USB}{id}/{key}")).radio(on)
                    })
                    .collect::<Vec<_>>(),
            )
        });
        let usb = match usb {
            Some(items) if !items.is_empty() => Entry::submenu(&tr!("USB Devices"), items),
            Some(_) => Entry::submenu(
                &tr!("USB Devices"),
                vec![Entry::item(&tr!("No USB devices"), "").enabled(false)],
            ),
            None => Entry::item(&tr!("USB Devices"), "").enabled(false),
        };
        let (pause_key, pause_act) = match can(Act::Resume) {
            true => (text!("Resume"), Act::Resume),
            false => (text!("Pause"), Act::Pause),
        };
        let mut entries = vec![
            item(text!("Start"), MACHINE_START, Act::Start).shortcut("Ctrl+R"),
            item(pause_key, MACHINE_PAUSE, pause_act),
            item(text!("Suspend"), MACHINE_SUSPEND, Act::Suspend),
            item(text!("Shut Down"), MACHINE_SHUT_DOWN, Act::ShutDown),
            item(text!("Force Stop"), MACHINE_FORCE_STOP, Act::ForceStop),
            item(text!("Restart"), MACHINE_RESTART, Act::Restart),
            Entry::separator(),
        ];
        if cx.installed(TERMINAL_APP_ID) {
            entries.push(Entry::item(&tr!("Connect via SSH"), MACHINE_CONNECT).enabled(connect));
        }
        entries.extend([
            Entry::item(&tr!("Show Machine Window"), MACHINE_OPEN)
                .shortcut("Ctrl+O")
                .enabled(connect || single(Act::Delete)),
            Entry::item(&tr!("Serial Console"), MACHINE_SERIAL)
                .enabled(can(Act::ShutDown) || can(Act::Resume)),
            Entry::item(&tr!("Send Ctrl+Alt+Delete"), MACHINE_SEND_KEYS).enabled(has_console),
            usb,
            Entry::separator(),
            Entry::item(&tr!("Take Snapshot…"), MACHINE_SNAPSHOT)
                .shortcut("Shift+Ctrl+S")
                .enabled(single(Act::TakeSnapshot)),
            Entry::item(&tr!("Clone…"), MACHINE_CLONE).enabled(single(Act::Clone)),
            Entry::separator(),
            item(text!("Delete…"), MACHINE_DELETE, Act::Delete),
        ]);
        Entry::submenu(&tr!("Machine"), entries)
    }

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
        let hub = self.hub();
        let opened = cx.open(Kind::Main, &tr!("Containers"), WINDOW_SIZE, |appearance| {
            let mut view = VeloxView::new(appearance);
            if let Some(hub) = hub {
                view.set_hub(hub);
            }
            Screen::Main(Box::new(Pane {
                view,
                work: Work::new(Rc::clone(&shared)),
                informer: shared,
                logs: Vec::new(),
                shells: Vec::new(),
                apps: None,
            }))
        });
        match opened {
            Ok(window) => {
                self.main = Some(window);
                self.informer = Some(informer);
            }
            Err(error) => tracing::warn!("cannot open a window: {error}"),
        }
    }

    fn open_logs(&mut self, cx: &mut Cx<Self>) {
        let shells = self
            .pane(cx)
            .map(|(_, pane)| std::mem::take(&mut pane.shells))
            .unwrap_or_default();
        for target in shells {
            Self::open_shell(cx, target);
        }
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

    fn start(_cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        Ok(Self {
            main: None,
            informer: None,
            hub: None,
            dismissed: HashSet::new(),
        })
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = match self.main.and_then(|main| cx.view(main)) {
            Some(Screen::Main(pane)) => Some(&pane.view),
            _ => None,
        };
        let ready = view.is_some_and(VeloxView::ready);
        let mut sections: Vec<Entry> = Section::ALL
            .into_iter()
            .zip(VIEW_KEYS)
            .map(|(section, key)| {
                Entry::item(&section.title(), section.command())
                    .shortcut(key)
                    .radio(view.is_some_and(|view| view.section() == section))
            })
            .collect();
        sections.push(Entry::separator());
        sections.push(Entry::item(&tr!("Find"), FIND).shortcut("Ctrl+F"));
        sections.extend(view.map(VeloxView::view_entries).unwrap_or_default());
        let pane = view.and_then(VeloxView::resource_menu).unwrap_or_else(|| {
            Entry::submenu(
                &tr!("Container"),
                view.map(VeloxView::container_menu).unwrap_or_default(),
            )
        });
        let mut window = vec![Entry::item(&tr!("Velox"), SHOW_MAIN)];
        window.extend(cx.windows().filter_map(|id| match cx.view(id) {
            Some(Screen::Logs(logs)) => Some(Entry::item(
                &logs.target().title(),
                &format!("{SHOW_LOGS}{}", logs.target().key()),
            )),
            Some(Screen::Shell(shell)) => Some(Entry::item(
                &shell.target().title(),
                &format!("{SHOW_SHELL}{}", shell.target().key()),
            )),
            _ => None,
        }));
        let can_create = view.is_some_and(|view| view.machines_ref().can_create());
        let mut file = vec![
            Entry::item(&tr!("New Virtual Machine…"), NEW_MACHINE)
                .shortcut("Ctrl+N")
                .enabled(can_create),
            Entry::separator(),
        ];
        file.extend(bande_velox::file_menu(ready));
        window.extend(cx.windows().filter_map(|id| match cx.view(id) {
            Some(Screen::Machine(machine)) => Some(Entry::item(
                &machine.title(),
                &format!("{SHOW_MACHINE}{}", machine.machine()),
            )),
            _ => None,
        }));
        Menus {
            menus: vec![
                Entry::submenu(&tr!("File"), file),
                Entry::submenu(&tr!("View"), sections),
                pane,
                self.machine_menu(cx),
            ],
            window,
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if let Some((id, key)) = command
            .strip_prefix(MACHINE_USB)
            .and_then(|rest| rest.split_once('/'))
        {
            if let Some(hub) = self.hub.as_mut() {
                hub.serve(MachineRequest::Usb(id.to_owned(), key.to_owned()));
            }
            return;
        }
        if let Some(id) = command.strip_prefix(SHOW_MACHINE) {
            if let Some(window) = self.machine_window(cx, id) {
                cx.raise(window, None);
            }
            return;
        }
        if let Some(id) = window
            && let Some((shown, Screen::Machine(machine))) = cx.window(id)
        {
            let handled = match command {
                MACHINE_SEND_KEYS => {
                    machine.send_ctrl_alt_delete();
                    true
                }
                MACHINE_PAUSE => {
                    machine.toggle_pause();
                    true
                }
                MACHINE_SNAPSHOT => {
                    machine.present_take_snapshot();
                    true
                }
                MACHINE_CLONE => {
                    machine.present_clone(None);
                    true
                }
                MACHINE_OPEN => true,
                MACHINE_SERIAL => {
                    machine.open_serial();
                    true
                }
                _ => false,
            };
            if handled {
                shown.request_redraw();
                return self.serve_machines(cx, id);
            }
        }
        if let Some(window) = window
            && let Some((shown, Screen::Machine(machine))) = cx.window(window)
            && (machine_act(command).is_some() || command == MACHINE_CONNECT)
        {
            match machine_act(command) {
                Some(act) => machine.act(act),
                None => machine.connect(),
            }
            shown.request_redraw();
            return self.serve_machines(cx, window);
        }
        if let Some(key) = command.strip_prefix(SHOW_SHELL) {
            let shown = cx.windows().find(|window| {
                matches!(cx.view(*window), Some(Screen::Shell(shell)) if shell.target().key() == key)
            });
            if let Some(window) = shown {
                cx.raise(window, None);
            }
            return;
        }
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
        if let Some(main) = self.main {
            self.serve_machines(cx, main);
        }
        self.update_sampling(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.show_main(cx);
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, view: Screen) {
        if self.main == Some(window) {
            self.main = None;
        }
        match view {
            Screen::Display(display) => {
                self.dismissed
                    .insert((display.machine().to_owned(), display.display()));
            }
            Screen::Machine(_) => self.sync_displays(cx),
            _ => {}
        }
        if cx.windows().next().is_none() {
            self.informer = None;
            self.hub = None;
        } else {
            self.update_sampling(cx);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.open_logs(cx);
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

    fn installed_changed(&mut self, cx: &mut Cx<Self>) {
        let terminal = cx.installed(TERMINAL_APP_ID);
        let windows: Vec<WindowId> = cx.windows().collect();
        for id in windows {
            if let Some((window, Screen::Machine(machine))) = cx.window(id) {
                machine.set_terminal(terminal);
                window.request_redraw();
            }
        }
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
        let (crashes, updates): (Vec<Update>, Vec<Update>) = informer
            .drain()
            .into_iter()
            .partition(|update| matches!(update, Update::Crashed { .. }));
        if !crashes.is_empty() && VeloxSettings::current().notify_crashes {
            for crash in crashes {
                if let Update::Crashed { name, code } = crash {
                    notify_crash(RUNNING_ID, &name, code);
                }
            }
        }
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
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some(bande_vm::START_FLAG) {
        let Some(id) = arguments.next() else {
            return ExitCode::FAILURE;
        };
        return match bande_vm::start_at_login(&id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("cannot start the machine {id}: {error}");
                ExitCode::FAILURE
            }
        };
    }
    bande_app::run::<Velox>(&INFO)
}
