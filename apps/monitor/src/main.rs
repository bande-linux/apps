use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_monitor::{
    Asked, BatteryReport, Catalog, INSPECTOR_SIZE, Icons, InspectorView, L10N, MenuCommand,
    Monitor, MonitorView, Request, Sampler, Screen, SleepBlocker, Snapshot, Stop, TOOLBAR_HEIGHT,
    Tab, battery_reader, list_sleep_blockers, stop_all,
};
use bande_toolkit::{
    Appearance, FrameRegions, Image, MenuPlacement, Scene, TextSystem, Window, WindowEvent,
};
use bande_xdg::{
    ICON_THEMES, Locale, applications, current_desktops, data_dirs, find_icon, replace_file,
    spawn_detached,
};

const APP_ID: &str = "org.bande.ActivityMonitor";
const WINDOW_SIZE: (u32, u32) = (860, 600);
const INTERVAL: Duration = Duration::from_secs(2);
const FIRST_INTERVAL: Duration = Duration::from_millis(500);
const ICON_PIXELS: u32 = 32;
const FILES: &str = "bande-files";
const FILES_APP_ID: &str = "org.bande.Files";
const PROC_ROOT: &str = "/";
const STATE_FILE: &str = "monitor";
const MEMORY_FLAG: &str = "--memory";

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("window-title"),
    keys: Keys::Standard,
    settings: None,
};

struct Pane(Box<dyn Screen>);

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        self.0.set_full_screen(full_screen);
        if let Some(header) = self.0.header() {
            window.set_frame_regions(FrameRegions {
                sidebar: 0.0,
                toolbar: if full_screen { TOOLBAR_HEIGHT } else { 0.0 },
            });
            window.set_header(header);
        }
        let (width, height) = window.size();
        self.0.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match event {
            WindowEvent::Input(input) => {
                let handled = self.0.handle(&input);
                window.set_cursor(self.0.cursor());
                handled
            }
            WindowEvent::Accessibility(request) => self.0.perform(&request),
            WindowEvent::MenuAction(action) => self.0.menu_action(&action),
            WindowEvent::MenuClosed => self.0.menu_closed(),
            WindowEvent::Pasted(text) => self.0.pasted(&text),
            WindowEvent::AppResponding {
                pid,
                app_id,
                responding,
            } => {
                u32::try_from(pid).is_ok_and(|pid| self.0.set_responding(pid, &app_id, responding))
            }
            WindowEvent::ToplevelChanged(state) => !state.suspended,
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.set_focused(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.0.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.0.animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        self.0.accessibility()
    }
}

struct ActivityMonitor {
    main: Option<WindowId>,
    sampling: bool,
    sampler: Sampler,
    snapshot: Snapshot,
    next_sample: Instant,
    inhibitors: Option<Option<Asked<Vec<SleepBlocker>>>>,
    battery: Option<Option<Asked<Option<BatteryReport>>>>,
}

fn measured(
    cx: &Cx<ActivityMonitor>,
    windows: &[WindowId],
    measures: fn(&dyn Screen) -> bool,
) -> bool {
    windows
        .iter()
        .filter_map(|window| cx.view(*window))
        .any(|pane| measures(&*pane.0))
}

impl ActivityMonitor {
    fn sample(&mut self, cx: &mut Cx<Self>) {
        let windows: Vec<WindowId> = cx.windows().collect();
        self.sampler
            .measure_proportional_memory(measured(cx, &windows, |screen| {
                screen.measures_proportional_memory()
            }));
        self.sampler
            .measure_disk(measured(cx, &windows, |screen| screen.measures_disk()));
        self.sampler
            .measure_gpu(measured(cx, &windows, |screen| screen.measures_gpu()));
        self.sampler
            .measure_wakeups(measured(cx, &windows, |screen| screen.measures_wakeups()));
        let blockers = measured(cx, &windows, |screen| screen.measures_sleep_blockers());
        let battery = measured(cx, &windows, |screen| screen.measures_battery());
        let watched = windows.iter().any(|window| {
            cx.window(*window)
                .is_some_and(|(shown, _)| !shown.toplevel_state().suspended)
        });
        self.sampler.measure_processes(watched);
        match self.sampler.sample() {
            Ok(mut snapshot) => {
                if !watched {
                    snapshot.processes = std::mem::take(&mut self.snapshot.processes);
                }
                if blockers
                    && let Some(inhibitors) = self.inhibitors.get_or_insert_with(start_inhibitors)
                {
                    snapshot.mark_sleep_blockers(&inhibitors.latest());
                }
                if battery
                    && snapshot.battery
                    && let Some(reader) = self.battery.get_or_insert_with(start_battery)
                {
                    snapshot.power = reader.latest();
                }
                for window in &windows {
                    if let Some((_, pane)) = cx.window(*window) {
                        pane.0.update(snapshot.clone());
                    }
                }
                self.snapshot = snapshot;
            }
            Err(error) => tracing::warn!("cannot sample: {error}"),
        }
        for window in windows {
            if let Some((shown, _)) = cx.window(window)
                && !shown.toplevel_state().suspended
            {
                shown.request_redraw();
            }
        }
        self.next_sample = Instant::now() + INTERVAL;
    }

    fn inspect(&mut self, cx: &mut Cx<Self>, pid: u32) {
        let shown = cx
            .windows()
            .find(|window| cx.view(*window).is_some_and(|pane| pane.0.inspects(pid)));
        if let Some((window, _)) = shown.and_then(|window| cx.window(window)) {
            window.request_redraw();
            return;
        }
        let Some(process) = self
            .snapshot
            .processes
            .iter()
            .find(|process| process.pid == pid)
        else {
            return;
        };
        let mut screen = InspectorView::new(
            process,
            Path::new(PROC_ROOT),
            rustix::param::clock_ticks_per_second(),
            Appearance::default().palette(),
            TextSystem::new(),
            icons(),
        );
        screen.update(self.snapshot.clone());
        let title = screen.title();
        let opened = cx.open(Kind::Utility, &title, INSPECTOR_SIZE, |appearance| {
            screen.set_appearance(appearance);
            Pane(Box::new(screen))
        });
        if let Err(error) = opened {
            tracing::warn!("cannot open the inspector: {error}");
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>, id: WindowId) {
        let requests = cx
            .window(id)
            .map(|(_, pane)| pane.0.take_requests())
            .unwrap_or_default();
        for request in requests {
            match request {
                Request::Resample => self.sample(cx),
                Request::Inspect(pid) => self.inspect(cx, pid),
                Request::SaveState(text) => save_state(cx, &text),
                request => {
                    if let Some((window, _)) = cx.window(id) {
                        serve_in(window, request);
                    }
                }
            }
        }
        if Some(id) != self.main && cx.view(id).is_some_and(|pane| pane.0.finished()) {
            cx.close(id);
        }
    }
}

fn serve_in(window: &mut Window, request: Request) {
    match request {
        Request::ShowInFiles(path) => show_in_files(window, &path),
        Request::Copy(text) => window.copy(&text),
        Request::Clipboard(request) => {
            if let Err(error) = window.serve_clipboard(request) {
                tracing::warn!("cannot use the clipboard: {error}");
            }
        }
        Request::EmojiPicker => {
            if let Err(error) = window.show_emoji_picker() {
                tracing::warn!("cannot show the emoji picker: {error}");
            }
        }
        Request::QuitApp { app_id, roots } => {
            if let Err(error) = window.quit_app(&app_id) {
                tracing::debug!("cannot ask {app_id} to quit: {error}");
                if let Err(error) = stop_all(&roots, Stop::Quit) {
                    tracing::warn!("cannot quit {app_id}: {error:?}");
                }
            }
        }
        Request::Menu(menu, at) => {
            if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
                tracing::warn!("cannot show the menu: {error}");
            }
        }
        Request::Resample | Request::Inspect(_) | Request::SaveState(_) => {}
    }
}

fn save_state(cx: &Cx<ActivityMonitor>, text: &str) {
    let Some(file) = cx.state_file(STATE_FILE) else {
        return;
    };
    if let Err(error) = replace_file(&file, text) {
        tracing::warn!("cannot remember the view: {error}");
    }
}

fn show_in_files(window: &mut Window, path: &Path) {
    let token = window
        .activation_token(APP_ID)
        .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
        .ok();
    let command = [FILES.to_owned(), path.to_string_lossy().into_owned()];
    if let Err(error) = spawn_detached(FILES_APP_ID, &command, None, token.as_deref()) {
        tracing::warn!("cannot show {} in Files: {error}", path.display());
    }
}

fn open_main(
    cx: &mut Cx<ActivityMonitor>,
    sampler: &mut Sampler,
    memory: bool,
) -> Result<(WindowId, Snapshot), bande_app::Error> {
    let catalog = Catalog::new(&applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    ));
    let mut monitor = Monitor::new(catalog, rustix::process::getuid().as_raw());
    if let Some(text) = cx
        .state_file(STATE_FILE)
        .and_then(|file| std::fs::read_to_string(file).ok())
    {
        monitor.restore(&text);
    }
    if memory {
        monitor.set_tab(Tab::Memory);
    }
    let mut snapshot = Snapshot::default();
    let main = cx.open(
        Kind::Main,
        &L10N.tr("window-title"),
        WINDOW_SIZE,
        |appearance| {
            let mut screen =
                MonitorView::new(monitor, appearance.palette(), TextSystem::new(), icons());
            screen.set_appearance(appearance);
            snapshot = first_sample(sampler, &mut screen);
            Pane(Box::new(screen))
        },
    )?;
    if let Some((window, _)) = cx.window(main)
        && let Err(error) = window.watch_apps()
    {
        tracing::debug!("cannot watch apps answer: {error}");
    }
    Ok((main, snapshot))
}

impl Program for ActivityMonitor {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut sampler = Sampler::live();
        let memory = arguments.iter().any(|argument| argument == MEMORY_FLAG);
        let (main, snapshot) = open_main(cx, &mut sampler, memory)?;
        Ok(Self {
            main: Some(main),
            sampling: true,
            sampler,
            snapshot,
            next_sample: Instant::now() + FIRST_INTERVAL,
            inhibitors: None,
            battery: None,
        })
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        Menus {
            menus: self
                .main
                .and_then(|main| cx.view(main))
                .and_then(|pane| pane.0.app_menus())
                .unwrap_or_default(),
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        let Some(window) = window else {
            return;
        };
        if let Some((shown, pane)) = cx.window(window)
            && pane.0.menu_action(command)
        {
            shown.request_redraw();
        }
        self.serve(cx, window);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        if arguments.iter().any(|argument| argument == MEMORY_FLAG) {
            if self.main.is_none() {
                self.reopen(cx);
            }
            let main = self.main;
            self.command(cx, main, MenuCommand::Show(Tab::Memory).action());
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.main.is_some() {
            return;
        }
        match open_main(cx, &mut self.sampler, false) {
            Ok((main, snapshot)) => {
                self.main = Some(main);
                self.snapshot = snapshot;
                self.sampling = true;
                self.next_sample = Instant::now() + FIRST_INTERVAL;
            }
            Err(error) => tracing::warn!("cannot open Activity Monitor: {error}"),
        }
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.main == Some(window) {
            self.main = None;
        }
        self.sampling = cx.windows().next().is_some();
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
        let shown = cx.windows().collect::<Vec<_>>().into_iter().any(|window| {
            cx.window(window)
                .is_some_and(|(shown, _)| !shown.toplevel_state().suspended)
        });
        if shown && !self.sampler.measures_processes() {
            self.sample(cx);
        }
        cx.refresh_menus();
    }

    fn deadline(&self) -> Option<Instant> {
        self.sampling.then_some(self.next_sample)
    }

    fn tick(&mut self, cx: &mut Cx<Self>, _now: Instant) {
        self.sample(cx);
    }
}

fn system_bus() -> Option<zbus::blocking::Connection> {
    zbus::blocking::Connection::system()
        .inspect_err(|error| tracing::warn!("cannot reach the system bus: {error}"))
        .ok()
}

fn start_inhibitors() -> Option<Asked<Vec<SleepBlocker>>> {
    system_bus().map(|bus| {
        Asked::start(move || {
            list_sleep_blockers(&bus)
                .inspect_err(|error| tracing::debug!("cannot list inhibitors: {error}"))
                .ok()
        })
    })
}

fn start_battery() -> Option<Asked<Option<BatteryReport>>> {
    system_bus().map(|bus| Asked::start(battery_reader(bus)))
}

fn icons() -> Icons {
    let dirs = data_dirs();
    Icons::new(move |name| {
        find_icon(name, ICON_PIXELS, &ICON_THEMES, &dirs)
            .and_then(|path| Image::load(&path, ICON_PIXELS))
    })
}

fn first_sample(sampler: &mut Sampler, screen: &mut impl Screen) -> Snapshot {
    sampler.measure_gpu(screen.measures_gpu());
    sampler.measure_wakeups(screen.measures_wakeups());
    sampler.measure_disk(screen.measures_disk());
    match sampler.sample() {
        Ok(snapshot) => {
            screen.update(snapshot.clone());
            snapshot
        }
        Err(error) => {
            tracing::warn!("cannot sample: {error}");
            Snapshot::default()
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<ActivityMonitor>(&INFO)
}
