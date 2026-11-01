use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::time::Instant;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View};
use bande_app_menu::edit_submenu;
use bande_packages::{Client, debian_arch};
use bande_software::{L10N, Software, WINDOW_SIZE, desktop_ids, installed_keys};
use bande_toolkit::{Appearance, Scene, TextSystem, Window, WindowEvent};

static INFO: Info = Info {
    id: "org.bande.Software",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("title"),
    keys: Keys::Standard,
    settings: None,
};

struct Pane(Software);

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        View::scene(self.0.ui(), window)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(self.0.ui(), window, event);
        self.0.pump() || handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.ui().set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.ui().set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.0.fd()]
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        self.0.pump();
        true
    }

    fn deadline(&self) -> Option<Instant> {
        self.0.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.0.accessibility())
    }
}

struct Store;

impl Program for Store {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let apps = bande_software::catalog::system_catalog();
        let installed = installed_keys(&apps, &desktop_ids(&bande_xdg::data_dirs()));
        let system = zbus::blocking::Connection::system()?;
        let mut software = Software::new(
            apps,
            installed,
            Client::new(&system),
            debian_arch(),
            Appearance::default().palette(),
            TextSystem::new(),
        )?;
        cx.open(Kind::Main, &L10N.tr("title"), WINDOW_SIZE, |appearance| {
            software.ui().set_appearance(appearance);
            Pane(software)
        })?;
        Ok(Self)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        Menus {
            menus: vec![edit_submenu(Vec::new())],
            window: Vec::new(),
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Store>(&INFO)
}
