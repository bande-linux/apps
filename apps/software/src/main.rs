use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::time::Instant;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_app_settings::{SettingsWindow, settings_folder};
use bande_l10n::{Text, text, tr};
use bande_notify::Urgency;
use bande_notify::client::{Request as Notification, notify};
use bande_packages::{Client, debian_arch};
use bande_software::settings::{SETTINGS_SIZE, SoftwareSettings, settings_form};
use bande_software::{
    COMMAND_BACK, COMMAND_DISCOVER, COMMAND_FIND, COMMAND_INSTALLED, COMMAND_UPDATES, Finished,
    SETTINGS_ID, Software, WINDOW_SIZE,
};
use bande_toolkit::{Appearance, Rect, Scene, TextSystem, Window, WindowEvent};

const FLATPAK: &str = "flatpak";

const INFO: Info = Info {
    id: "org.bande.Software",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Software"),
    keys: Keys::Standard,
    settings: None,
};

const WITH_FLATPAK: Info = Info {
    settings: Some(Settings {
        title: || tr!("Software Settings"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
    ..INFO
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsWindow::<SoftwareSettings>::new(
        appearance,
        settings_folder(),
        settings_form,
    ))
}

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
        self.0.fds()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        self.0.pump();
        self.0.take_finished();
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

    fn text_input_area(&self) -> Option<Rect> {
        self.0.text_input_area()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.0.accessibility())
    }
}

struct Store {
    main: Option<WindowId>,
    parked: Option<Pane>,
}

fn open_main(cx: &mut Cx<Store>, mut pane: Pane) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(Kind::Main, &tr!("Software"), WINDOW_SIZE, |appearance| {
        pane.0.ui().set_appearance(appearance);
        pane
    })
}

impl Program for Store {
    type View = Pane;

    fn start(_cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let system = zbus::blocking::Connection::system()?;
        let software = Software::new(
            Client::new(&system),
            Client::new(&system),
            debian_arch(),
            Appearance::default().palette(),
            TextSystem::new(),
        )?;
        Ok(Self {
            main: None,
            parked: Some(Pane(software)),
        })
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        let Some(mut pane) = self.parked.take() else {
            return;
        };
        pane.0.reload();
        pane.0.set_settings_installed(cx.installed(SETTINGS_ID));
        match open_main(cx, pane) {
            Ok(main) => self.main = Some(main),
            Err(error) => tracing::warn!("cannot open Software: {error}"),
        }
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, mut view: Pane) {
        if self.main == Some(window) {
            view.0.take_finished();
            self.main = None;
            self.parked = Some(view);
        }
    }

    fn installed_changed(&mut self, cx: &mut Cx<Self>) {
        let settings = cx.installed(SETTINGS_ID);
        if let Some((window, pane)) = self.main.and_then(|main| cx.window(main)) {
            pane.0.set_settings_installed(settings);
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.parked.iter().flat_map(|pane| pane.0.fds()).collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        let Some(pane) = &mut self.parked else {
            return;
        };
        pane.0.pump();
        let finished = pane.0.take_finished();
        let Some(session) = cx.session() else {
            return;
        };
        for done in finished {
            let summary = match &done {
                Finished::Installed(name) => {
                    tr!("{name} is installed", name = name.clone())
                }
                Finished::Failed(name) => {
                    tr!("{name} couldn’t be installed", name = name.clone())
                }
            };
            let request = Notification {
                app_name: &tr!("Software"),
                replaces_id: 0,
                app_icon: INFO.id,
                summary: &summary,
                body: "",
                actions: &[],
                urgency: Urgency::Normal,
                sound: None,
                desktop_entry: Some(INFO.id),
            };
            if let Err(error) = notify(session, &request) {
                tracing::warn!("cannot announce the installation: {error}");
            }
        }
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        let item = |label: Text, action: &str, shortcut: &str| {
            Entry::item(&label.tr(), action).shortcut(shortcut)
        };
        Menus {
            menus: vec![
                edit_submenu(vec![item(text!("Find"), COMMAND_FIND, "Ctrl+F")]),
                Entry::submenu(
                    &tr!("View"),
                    vec![
                        item(text!("Back"), COMMAND_BACK, "Alt+Left"),
                        Entry::separator(),
                        item(text!("Discover"), COMMAND_DISCOVER, "Ctrl+1"),
                        item(
                            text!(context = "section", "Installed"),
                            COMMAND_INSTALLED,
                            "Ctrl+2",
                        ),
                        item(text!("Updates"), COMMAND_UPDATES, "Ctrl+3"),
                    ],
                ),
            ],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        if let Some((window, pane)) = self.main.and_then(|main| cx.window(main))
            && pane.0.command(command)
        {
            window.request_redraw();
        }
    }
}

fn main() -> ExitCode {
    let info = match bande_xdg::find_executable(FLATPAK) {
        Some(_) => &WITH_FLATPAK,
        None => &INFO,
    };
    bande_app::run::<Store>(info)
}
