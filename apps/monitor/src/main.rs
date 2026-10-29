use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use unbound_monitor::{
    AppList, Catalog, ForceQuitView, Icons, L10N, Monitor, MonitorView, Request, Sampler, Screen,
};
use unbound_toolkit::{App, Image, Role, Scene, TextSystem, Window, WindowError, WindowEvent};
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

    fn serve(&mut self, window: &mut Window) {
        for request in self.screen.take_requests() {
            match request {
                Request::Resample => {
                    self.sample();
                    self.next_sample = Instant::now() + INTERVAL;
                    window.request_redraw();
                }
                Request::ShowInFiles(path) => show_in_files(&path),
            }
        }
    }
}

fn show_in_files(path: &Path) {
    let folder = path.parent().unwrap_or(path);
    let command = [FILES.to_owned(), folder.to_string_lossy().into_owned()];
    if let Err(error) = spawn_detached(&command, None, None) {
        tracing::warn!("cannot open {}: {error}", folder.display());
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
                window.request_redraw();
            }
            WindowEvent::Input(input) => {
                if self.screen.handle(&input) {
                    window.request_redraw();
                }
                self.serve(window);
            }
            _ => {}
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
}

fn icons() -> Icons {
    let dirs = data_dirs();
    Icons::new(move |name| {
        find_icon(name, ICON_PIXELS, &[], &dirs).and_then(|path| Image::load(&path, ICON_PIXELS))
    })
}

fn open(mut screen: impl Screen, title: &str, size: (u32, u32)) -> Result<(), WindowError> {
    let mut window = Window::open(Role::toplevel(title, APP_ID), size)?;
    screen.set_palette(window.appearance().palette());
    let mut app = ActivityMonitor {
        screen,
        sampler: Sampler::live(),
        next_sample: Instant::now() + FIRST_INTERVAL,
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
        open(screen, &L10N.tr("force-quit-title"), FORCE_QUIT_SIZE)
    } else {
        let screen = MonitorView::new(Monitor::new(catalog), palette, TextSystem::new(), icons());
        open(screen, &L10N.tr("window-title"), WINDOW_SIZE)
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
