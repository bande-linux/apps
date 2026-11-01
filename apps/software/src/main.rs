use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::time::Instant;

use bande_packages::{Client, debian_arch};
use bande_software::{L10N, Software, WINDOW_SIZE, desktop_ids, installed_keys};
use bande_toolkit::{App, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.bande.Software";

struct Running {
    software: Software,
}

impl App for Running {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.software.ui().scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let ui = self.software.ui();
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
        self.software.pump();
        window.request_redraw();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.software.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        self.software.pump();
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        self.software.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.software.accessibility())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
    let appearance = window.appearance();
    let apps = bande_catalog::system_catalog();
    let installed = installed_keys(&apps, &desktop_ids(&bande_xdg::data_dirs()));
    let system = zbus::blocking::Connection::system()?;
    let mut software = Software::new(
        apps,
        installed,
        Client::new(&system),
        debian_arch(),
        appearance.palette(),
        TextSystem::new(),
    )?;
    software.ui().set_appearance(appearance);
    bande_toolkit::run(&mut window, &mut Running { software })?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-software");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
