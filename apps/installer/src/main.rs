use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::edit_submenu;
use bande_installer::{Installer, Machine, WINDOW_SIZE};
use bande_l10n::tr;
use bande_packages::Client;
use bande_toolkit::{Appearance, Scene, TextSystem, Window, WindowEvent};
use zbus::blocking::Connection;

static INFO: Info = Info {
    id: "org.bande.Installer",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Package Installer"),
    keys: Keys::Standard,
    settings: None,
};

struct Pane(Installer);

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

    fn animating(&self) -> bool {
        self.0.animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.0.accessibility())
    }
}

struct Packages {
    system: Connection,
    parked: Vec<Pane>,
}

fn show(cx: &mut Cx<Packages>, mut pane: Pane) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(
        Kind::Main,
        &tr!("Package Installer"),
        WINDOW_SIZE,
        |appearance| {
            pane.0.ui().set_appearance(appearance);
            pane
        },
    )
}

impl Packages {
    fn install(&self, cx: &mut Cx<Self>, file: &Path) -> Result<WindowId, bande_app::Error> {
        let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let installer = Installer::new(
            Client::new(&self.system),
            file,
            Machine::current()?,
            Appearance::default().palette(),
            TextSystem::new(),
        )?;
        Ok(show(cx, Pane(installer))?)
    }
}

impl Program for Packages {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let file = arguments
            .first()
            .map(PathBuf::from)
            .ok_or("usage: bande-installer FILE.deb")?;
        let packages = Self {
            system: Connection::system()?,
            parked: Vec::new(),
        };
        packages.install(cx, &file)?;
        Ok(packages)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        Menus {
            menus: vec![edit_submenu(Vec::new())],
            window: Vec::new(),
        }
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        for file in arguments {
            if let Err(error) = self.install(cx, &directory.join(file)) {
                tracing::warn!("cannot open the package: {error}");
            }
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        let Some(pane) = self.parked.pop() else {
            return;
        };
        if let Err(error) = show(cx, pane) {
            tracing::warn!("cannot show the package: {error}");
        }
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, view: Pane) {
        self.parked.retain(|pane| !pane.0.closed());
        self.parked.push(view);
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        if cx.view(window).is_some_and(|pane| pane.0.closed()) {
            cx.close(window);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.parked.iter().map(|pane| pane.0.fd()).collect()
    }

    fn source_ready(&mut self, _cx: &mut Cx<Self>, index: usize) {
        if let Some(pane) = self.parked.get_mut(index) {
            pane.0.pump();
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Packages>(&INFO)
}
