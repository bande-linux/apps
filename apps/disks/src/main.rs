use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::Instant;

use bande_l10n::tr;
use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_disks::{
    ADD_PARTITION, Abilities, DELETE_PARTITION, DisksView, EJECT, ERASE, FIRST_AID, IMAGE_FROM,
    MOUNT, Model, OPEN_IMAGE, RENAME, RESIZE, RESTORE, Report, Request, SIDEBAR_WIDTH, Worker,
};
use bande_toolkit::{
    Appearance, FrameRegions, Rect, Scene, WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};
use bande_volumes::udisks::Udisks;
use zbus::blocking::Connection;

const WINDOW_SIZE: (u32, u32) = (920, 600);
const SHOW_MAIN: &str = "show-main";

static INFO: Info = Info {
    id: "org.bande.DiskUtility",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Disk Utility"),
    keys: Keys::Standard,
    settings: None,
};

struct Pane {
    view: DisksView,
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
        window.set_header(self.view.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(self.view.ui(), window, event);
        self.view.pump() || handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.view.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.view.ui().set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.ui_ref().deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
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

struct Live {
    udisks: Udisks,
    changes: UnixStream,
    worker: Worker,
}

impl Live {
    fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let system = Connection::system()?;
        let (changes, ring) = UnixStream::pair()?;
        changes.set_nonblocking(true)?;
        ring.set_nonblocking(true)?;
        let udisks = Udisks::follow(&system, move |_, _| {
            let _ = (&ring).write(&[1]);
        })?;
        let worker = Worker::start(system)?;
        Ok(Self {
            udisks,
            changes,
            worker,
        })
    }
}

struct Disks {
    main: Option<WindowId>,
    live: Option<Live>,
    model: Model,
    abilities: Option<Abilities>,
}

impl Disks {
    fn pane<'a>(&self, cx: &'a mut Cx<Self>) -> Option<(&'a mut Window, &'a mut Pane)> {
        cx.window(self.main?)
    }

    fn show_main(&mut self, cx: &mut Cx<Self>) {
        if let Some(main) = self.main {
            return cx.raise(main, None);
        }
        let model = self.model.clone();
        let abilities = self.abilities.clone();
        let opened = cx.open(
            Kind::Main,
            &tr!("Disk Utility"),
            WINDOW_SIZE,
            |appearance| {
                let mut view = DisksView::new(appearance);
                if let Some(abilities) = abilities {
                    view.set_abilities(abilities);
                }
                view.set_model(model);
                Pane { view }
            },
        );
        match opened {
            Ok(window) => self.main = Some(window),
            Err(error) => tracing::warn!("cannot open a window: {error}"),
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>) {
        let session = cx.session().cloned();
        let main = self.main;
        let Some((window, pane)) = main.and_then(|main| cx.window(main)) else {
            return;
        };
        let requests = pane.view.take_requests();
        window.request_redraw();
        if let Some(live) = &self.live {
            for request in requests {
                match request {
                    Request::Run(job) => live.worker.run(job),
                    Request::Choose(choose) => match &session {
                        Some(session) => live.worker.choose(session.clone(), choose),
                        None => tracing::warn!("no session bus to ask for a file"),
                    },
                    Request::Stop => live.worker.cancel(),
                }
            }
        }
        cx.refresh_menus();
    }

    fn item(&self, cx: &Cx<Self>, title: String, command: &str) -> Entry {
        let enabled = self
            .main
            .and_then(|main| cx.view(main))
            .is_some_and(|pane| pane.view.can(command));
        Entry::item(&title, command).enabled(enabled)
    }
}

impl Program for Disks {
    type View = Pane;

    fn start(_cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let live = Live::start()
            .inspect_err(|error| tracing::warn!("cannot follow UDisks2: {error}"))
            .ok();
        Ok(Self {
            main: None,
            live,
            model: Model::default(),
            abilities: None,
        })
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let mount = self
            .main
            .and_then(|main| cx.view(main))
            .map(|pane| pane.view.mount_title())
            .unwrap_or_else(|| tr!("Mount"));
        let file = vec![
            Entry::item(&tr!("Open Disk Image…"), OPEN_IMAGE)
                .shortcut("Ctrl+O")
                .without_window(),
            self.item(cx, tr!("Image from Selection…"), IMAGE_FROM),
        ];
        let edit = edit_submenu(vec![
            Entry::separator(),
            self.item(cx, tr!("First Aid…"), FIRST_AID),
            self.item(cx, tr!("Add Partition…"), ADD_PARTITION),
            self.item(cx, tr!("Delete Partition…"), DELETE_PARTITION),
            self.item(cx, tr!("Resize Partition…"), RESIZE),
            self.item(cx, tr!("Erase…"), ERASE),
            self.item(cx, tr!("Restore…"), RESTORE),
            self.item(cx, tr!("Rename…"), RENAME),
            Entry::separator(),
            self.item(cx, mount, MOUNT),
            self.item(cx, tr!("Eject"), EJECT).shortcut("Ctrl+E"),
        ]);
        Menus {
            menus: vec![Entry::submenu(&tr!("File"), file), edit],
            window: vec![Entry::item(&tr!("Disk Utility"), SHOW_MAIN)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        self.show_main(cx);
        if command == SHOW_MAIN {
            return;
        }
        if let Some((_, pane)) = self.pane(cx) {
            pane.view.command(command);
        }
        self.serve(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.show_main(cx);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.main == Some(window) {
            self.main = None;
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.serve(cx);
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        let Some((window, pane)) = self.pane(cx) else {
            return true;
        };
        if !pane.view.busy() {
            return true;
        }
        pane.view.warn_busy();
        window.request_redraw();
        false
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.live
            .iter()
            .flat_map(|live| [live.changes.as_fd(), live.worker.fd()])
            .collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        let Some(live) = &self.live else {
            return;
        };
        let reports = match index {
            0 => {
                let mut bytes = [0; 64];
                while matches!((&live.changes).read(&mut bytes), Ok(read) if read > 0) {}
                self.model = Model::from_objects(&live.udisks.objects());
                Vec::new()
            }
            _ => live.worker.drain(),
        };
        for report in &reports {
            if let Report::Abilities(abilities) = report {
                self.abilities = Some(abilities.clone());
            }
        }
        let model = self.model.clone();
        if let Some((_, pane)) = self.pane(cx) {
            pane.view.set_model(model);
            for report in reports {
                pane.view.report(report);
            }
        }
        self.serve(cx);
    }
}

fn main() -> ExitCode {
    bande_app::run::<Disks>(&INFO)
}
