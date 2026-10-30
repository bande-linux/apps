use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;

use unbound_app_menu::{AppMenu, Standard};
use unbound_monitor::{
    AppList, Catalog, ForceQuitView, Icons, L10N, Monitor, MonitorView, Request, Sampler, Screen,
    app_menus,
};
use unbound_toolkit::{
    App, Image, MenuPlacement, Role, Scene, TextSystem, Window, WindowError, WindowEvent,
};
use unbound_xdg::{Locale, applications, current_desktops, data_dirs, find_icon, spawn_detached};

const APP_ID: &str = "org.unbound.ActivityMonitor";
const WINDOW_SIZE: (u32, u32) = (860, 600);
const FORCE_QUIT_SIZE: (u32, u32) = (380, 440);
const INTERVAL: Duration = Duration::from_secs(2);
const FIRST_INTERVAL: Duration = Duration::from_millis(500);
const ICON_PIXELS: u32 = 32;
const FILES: &str = "unbound-files";

struct ActivityMonitor<S> {
    screen: S,
    sampler: Sampler,
    next_sample: Instant,
    app_menu: Option<AppMenu>,
}

impl<S: Screen> ActivityMonitor<S> {
    fn sample(&mut self) {
        self.sampler
            .measure_proportional_memory(self.screen.measures_proportional_memory());
        match self.sampler.sample() {
            Ok(snapshot) => self.screen.update(snapshot),
            Err(error) => tracing::warn!("cannot sample: {error}"),
        }
    }

    fn menu_action(&mut self, window: &mut Window, action: &str) {
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit | Standard::CloseWindow) => {
                window.close();
                Ok(())
            }
            None => {
                if self.screen.menu_action(action) {
                    window.request_redraw();
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
        self.serve(window);
    }

    fn serve(&mut self, window: &mut Window) {
        for request in self.screen.take_requests() {
            match request {
                Request::Resample => {
                    self.sample();
                    self.next_sample = Instant::now() + INTERVAL;
                    window.request_redraw();
                }
                Request::ShowInFiles(path) => show_in_files(window, &path),
                Request::Copy(text) => window.copy(&text),
                Request::Menu(menu, at) => {
                    if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
            }
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

impl<S: Screen> App for ActivityMonitor<S> {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.screen.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::AppearanceChanged(appearance) => {
                self.screen.set_palette(appearance.palette());
            }
            WindowEvent::Focus(focused) => {
                self.screen.set_focused(focused);
                if focused && let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                    tracing::warn!("{error}");
                }
                window.request_redraw();
            }
            WindowEvent::Input(input) => {
                if self.screen.handle(&input) {
                    window.request_redraw();
                }
                window.set_cursor(self.screen.cursor());
                self.serve(window);
            }
            WindowEvent::Accessibility(request) => {
                if self.screen.perform(&request) {
                    window.request_redraw();
                }
                self.serve(window);
            }
            WindowEvent::MenuAction(action) => self.menu_action(window, &action),
            WindowEvent::AppResponding {
                pid,
                app_id,
                responding,
            } => {
                let changed = u32::try_from(pid)
                    .is_ok_and(|pid| self.screen.set_responding(pid, &app_id, responding));
                if changed {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.app_menu.iter().map(AppMenu::fd).collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let actions = self
            .app_menu
            .as_mut()
            .map(AppMenu::take_actions)
            .unwrap_or_default();
        for action in actions {
            self.menu_action(window, &action);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        Some(self.next_sample)
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.sample();
        self.next_sample = now + INTERVAL;
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.screen.finished()
    }

    fn animating(&self) -> bool {
        self.screen.animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        self.screen.accessibility()
    }
}

fn icons() -> Icons {
    let dirs = data_dirs();
    Icons::new(move |name| {
        find_icon(name, ICON_PIXELS, &[], &dirs).and_then(|path| Image::load(&path, ICON_PIXELS))
    })
}

fn open(
    mut screen: impl Screen,
    title: &str,
    size: (u32, u32),
    app_menu: Option<AppMenu>,
) -> Result<(), WindowError> {
    let mut window = Window::open(Role::toplevel(title, APP_ID), size)?;
    screen.set_palette(window.appearance().palette());
    let mut app = ActivityMonitor {
        screen,
        sampler: Sampler::live(),
        next_sample: Instant::now() + FIRST_INTERVAL,
        app_menu,
    };
    app.sample();
    unbound_toolkit::run(&mut window, &mut app)
}

fn run() -> Result<(), WindowError> {
    let catalog = Catalog::new(&applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    ));
    let palette = unbound_toolkit::Appearance::default().palette();
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
