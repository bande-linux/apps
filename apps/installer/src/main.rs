use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use bande_installer::{Installer, L10N, Machine, WINDOW_SIZE};
use bande_packages::Client;
use bande_toolkit::{App, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.bande.Installer";

struct Running {
    installer: Installer,
}

impl Running {
    fn settle(&mut self, window: &mut Window) {
        self.installer.pump();
        if self.installer.closed() {
            window.close();
        }
        window.request_redraw();
    }
}

impl App for Running {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.installer.ui().scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let ui = self.installer.ui();
        match event {
            WindowEvent::Input(input) => {
                ui.handle(&input);
            }
            WindowEvent::Accessibility(request) => {
                ui.perform(&request);
            }
            WindowEvent::AppearanceChanged(appearance) => ui.set_appearance(appearance),
            _ => return,
        }
        self.settle(window);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.installer.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        self.settle(window);
    }

    fn deadline(&self) -> Option<Instant> {
        self.installer.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.installer.accessibility())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: bande-installer FILE.deb")?;
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let system = zbus::blocking::Connection::system()?;
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
    let appearance = window.appearance();
    let mut installer = Installer::new(
        Client::new(&system),
        path,
        Machine::current()?,
        appearance.palette(),
        TextSystem::new(),
    )?;
    installer.ui().set_appearance(appearance);
    bande_toolkit::run(&mut window, &mut Running { installer })?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-installer");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
