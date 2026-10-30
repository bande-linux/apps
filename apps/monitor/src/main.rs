use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;

use unbound_app_menu::{AppMenu, Standard};
use unbound_monitor::{
    AppList, Catalog, ForceQuitView, INSPECTOR_SIZE, Icons, InspectorView, L10N, Monitor,
    MonitorView, Request, Sampler, Screen, Snapshot, TOOLBAR_HEIGHT, app_menus,
};
use unbound_toolkit::{
    Appearance, FrameRegions, Image, MenuPlacement, Role, Scene, Surfaces, TextSystem, Window,
    WindowError, WindowEvent,
};
use unbound_xdg::{
    ICON_THEMES, Locale, applications, current_desktops, data_dirs, find_icon, spawn_detached,
};

const APP_ID: &str = "org.unbound.ActivityMonitor";
const WINDOW_SIZE: (u32, u32) = (860, 600);
const FORCE_QUIT_SIZE: (u32, u32) = (380, 440);
const INTERVAL: Duration = Duration::from_secs(2);
const FIRST_INTERVAL: Duration = Duration::from_millis(500);
const ICON_PIXELS: u32 = 32;
const FILES: &str = "unbound-files";
const PROC_ROOT: &str = "/";

struct ActivityMonitor {
    windows: Vec<Window>,
    screens: Vec<Box<dyn Screen>>,
    sampler: Sampler,
    snapshot: Snapshot,
    next_sample: Instant,
    app_menu: Option<AppMenu>,
    focused: usize,
}

impl ActivityMonitor {
    fn sample(&mut self) {
        let proportional = self
            .screens
            .iter()
            .any(|screen| screen.measures_proportional_memory());
        self.sampler.measure_proportional_memory(proportional);
        match self.sampler.sample() {
            Ok(snapshot) => {
                for screen in &mut self.screens {
                    screen.update(snapshot.clone());
                }
                self.snapshot = snapshot;
            }
            Err(error) => tracing::warn!("cannot sample: {error}"),
        }
        for window in &mut self.windows {
            window.request_redraw();
        }
    }

    fn menu_action(&mut self, index: usize, action: &str) {
        let window = &mut self.windows[index];
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit) => {
                self.windows[0].close();
                Ok(())
            }
            Some(Standard::CloseWindow) => {
                self.close(index);
                return;
            }
            None => {
                if self.screens[index].menu_action(action) {
                    window.request_redraw();
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
        self.serve(index);
    }

    fn close(&mut self, index: usize) {
        if index == 0 {
            self.windows[0].close();
            return;
        }
        self.windows.remove(index);
        self.screens.remove(index);
        self.focused = 0;
    }

    fn inspect(&mut self, pid: u32) {
        if let Some(index) = self.screens.iter().position(|screen| screen.inspects(pid)) {
            self.windows[index].request_redraw();
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
        let palette = self.windows[0].appearance().palette();
        let mut screen = InspectorView::new(
            process,
            Path::new(PROC_ROOT),
            rustix::param::clock_ticks_per_second(),
            palette,
            TextSystem::new(),
            icons(),
        );
        screen.update(self.snapshot.clone());
        let role = Role::toplevel(&screen.title(), APP_ID);
        match self.windows[0].open_beside(role, INSPECTOR_SIZE) {
            Ok(window) => {
                self.windows.push(window);
                self.screens.push(Box::new(screen));
            }
            Err(error) => tracing::warn!("cannot open the inspector: {error}"),
        }
    }

    fn serve(&mut self, index: usize) {
        for request in self.screens[index].take_requests() {
            let window = &mut self.windows[index];
            match request {
                Request::Resample => {
                    self.sample();
                    self.next_sample = Instant::now() + INTERVAL;
                }
                Request::ShowInFiles(path) => show_in_files(window, &path),
                Request::Copy(text) => window.copy(&text),
                Request::Paste => {
                    if let Err(error) = window.paste() {
                        tracing::warn!("cannot paste: {error}");
                    }
                }
                Request::EmojiPicker => {
                    if let Err(error) = window.show_emoji_picker() {
                        tracing::warn!("cannot show the emoji picker: {error}");
                    }
                }
                Request::Inspect(pid) => self.inspect(pid),
                Request::Menu(menu, at) => {
                    if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
            }
        }
        if index > 0
            && self
                .screens
                .get(index)
                .is_some_and(|screen| screen.finished())
        {
            self.close(index);
        }
    }

    fn appearance(&mut self, appearance: Appearance) {
        for (window, screen) in self.windows.iter_mut().zip(&mut self.screens) {
            screen.set_palette(appearance.palette());
            window.request_redraw();
        }
    }
}

fn show_in_files(window: &mut Window, path: &Path) {
    let token = window
        .activation_token(APP_ID)
        .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
        .ok();
    let command = [FILES.to_owned(), path.to_string_lossy().into_owned()];
    if let Err(error) = spawn_detached(&command, None, token.as_deref()) {
        tracing::warn!("cannot show {} in Files: {error}", path.display());
    }
}

impl Surfaces for ActivityMonitor {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let window = &mut self.windows[index];
        let screen = &mut self.screens[index];
        let full_screen = window.header_height() == 0;
        screen.set_full_screen(full_screen);
        if let Some(header) = screen.header() {
            window.set_frame_regions(FrameRegions {
                sidebar: 0.0,
                toolbar: if full_screen { TOOLBAR_HEIGHT } else { 0.0 },
            });
            window.set_header(header);
        }
        let (width, height) = window.size();
        screen.scene(width, height)
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        if index >= self.screens.len() {
            return;
        }
        match event {
            WindowEvent::AppearanceChanged(appearance) => self.appearance(appearance),
            WindowEvent::CloseRequested => self.close(index),
            WindowEvent::QuitRequested => self.windows[0].close(),
            WindowEvent::Focus(focused) => {
                self.screens[index].set_focused(focused);
                if focused {
                    self.focused = index;
                    if let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                        tracing::warn!("{error}");
                    }
                }
                self.windows[index].request_redraw();
            }
            WindowEvent::Input(input) => {
                if self.screens[index].handle(&input) {
                    self.windows[index].request_redraw();
                }
                let cursor = self.screens[index].cursor();
                self.windows[index].set_cursor(cursor);
                self.serve(index);
            }
            WindowEvent::Accessibility(request) => {
                if self.screens[index].perform(&request) {
                    self.windows[index].request_redraw();
                }
                self.serve(index);
            }
            WindowEvent::MenuAction(action) => self.menu_action(index, &action),
            WindowEvent::MenuClosed => {
                if self.screens[index].menu_closed() {
                    self.windows[index].request_redraw();
                }
            }
            WindowEvent::Pasted(text) => {
                if self.screens[index].pasted(&text) {
                    self.windows[index].request_redraw();
                }
                self.serve(index);
            }
            WindowEvent::AppResponding {
                pid,
                app_id,
                responding,
            } => {
                let changed = u32::try_from(pid)
                    .is_ok_and(|pid| self.screens[0].set_responding(pid, &app_id, responding));
                if changed {
                    self.windows[0].request_redraw();
                }
            }
            _ => {}
        }
        if index == 0 && self.screens[0].finished() {
            self.windows[0].close();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.app_menu.iter().map(AppMenu::fd).collect()
    }

    fn source_ready(&mut self, _index: usize) {
        let actions = self
            .app_menu
            .as_mut()
            .map(AppMenu::take_actions)
            .unwrap_or_default();
        for action in actions {
            let focused = self.focused.min(self.windows.len() - 1);
            self.menu_action(focused, &action);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.screens
            .iter()
            .filter_map(|screen| screen.deadline())
            .chain([self.next_sample])
            .min()
    }

    fn tick(&mut self, now: Instant) {
        if now >= self.next_sample {
            self.sample();
            self.next_sample = now + INTERVAL;
        }
        for window in &mut self.windows {
            window.request_redraw();
        }
    }

    fn animating(&self, index: usize) -> bool {
        self.screens
            .get(index)
            .is_some_and(|screen| screen.animating())
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        self.screens.get(index)?.accessibility()
    }
}

fn icons() -> Icons {
    let dirs = data_dirs();
    Icons::new(move |name| {
        find_icon(name, ICON_PIXELS, &ICON_THEMES, &dirs)
            .and_then(|path| Image::load(&path, ICON_PIXELS))
    })
}

fn open(
    mut screen: impl Screen + 'static,
    title: &str,
    size: (u32, u32),
    app_menu: Option<AppMenu>,
) -> Result<(), WindowError> {
    let window = Window::open(Role::toplevel(title, APP_ID), size)?;
    screen.set_palette(window.appearance().palette());
    let mut app = ActivityMonitor {
        windows: vec![window],
        screens: vec![Box::new(screen)],
        sampler: Sampler::live(),
        snapshot: Snapshot::default(),
        next_sample: Instant::now() + FIRST_INTERVAL,
        app_menu,
        focused: 0,
    };
    app.sample();
    unbound_toolkit::run_surfaces(&mut app)
}

fn run() -> Result<(), WindowError> {
    let catalog = Catalog::new(&applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    ));
    let palette = Appearance::default().palette();
    if std::env::args().any(|argument| argument == "--force-quit") {
        let apps = AppList::new(catalog, rustix::process::getuid().as_raw());
        let screen = ForceQuitView::new(apps, palette, TextSystem::new(), icons());
        open(screen, &L10N.tr("force-quit-title"), FORCE_QUIT_SIZE, None)
    } else {
        let screen = MonitorView::new(Monitor::new(catalog), palette, TextSystem::new(), icons());
        let app_menu = AppMenu::export(APP_ID, app_menus())
            .inspect_err(|error| tracing::warn!("{error}"))
            .ok();
        open(screen, &L10N.tr("window-title"), WINDOW_SIZE, app_menu)
    }
}

fn main() -> ExitCode {
    unbound_log::init("unbound-monitor");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
