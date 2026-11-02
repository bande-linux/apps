mod app;
mod bus;

use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use bande_app::{
    Cx, Info, Keys, Kind, Menus, Program, Settings as SettingsWindowInfo, View, WindowId,
};
use bande_app_settings::{SettingsWatch, SettingsWindow, Stored, settings_folder};
use bande_terminal::{
    CLOSE_WINDOW, CloseAsk, Command, GridSize, L10N, MenuCommand, Painter, SETTINGS_SIZE, Session,
    Settings, Terminal, app_menus, claim, left_behind, settings_view, window_file, windows_folder,
};
use bande_toolkit::{Appearance, Point, WindowDrag};

use crate::app::{Arguments, Ask, CarriedTab, Detached, Opened, TerminalWindow};
use crate::bus::Bus;

const APP_ID: &str = "org.bande.Terminal";
const PROGRAM: &str = "bande-terminal";

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("title"),
    keys: Keys::Shifted,
    settings: Some(SettingsWindowInfo {
        title: || L10N.tr("settings-title"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsWindow::<Settings>::new(
        appearance,
        settings_folder(),
        settings_view,
    ))
}

fn running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .is_ok_and(|name| name.trim_end() == PROGRAM)
}

struct Carried {
    from: WindowId,
    index: usize,
    tab: u32,
    holder: WindowId,
    floating: bool,
    drag: WindowDrag,
    grab: Point,
    size: (u32, u32),
}

struct Terminals {
    settings: Settings,
    settings_watch: Option<SettingsWatch>,
    home: Option<PathBuf>,
    bus: Rc<Bus>,
    carrying: Rc<Cell<bool>>,
    carried: Option<Carried>,
    next_serial: u32,
}

impl Terminals {
    fn painter(&self) -> Painter {
        let mut painter = Painter::new(
            Appearance::default(),
            self.settings.profile,
            self.settings.font_size,
            1.0,
        );
        painter.set_font(&self.settings.font);
        painter
    }

    fn next_window(&mut self, saved: bool) -> (u32, Option<PathBuf>) {
        let serial = self.next_serial;
        self.next_serial += 1;
        let saved_file = windows_folder()
            .filter(|_| saved)
            .map(|folder| window_file(&folder, std::process::id(), serial));
        (serial, saved_file)
    }

    fn show(
        &mut self,
        cx: &mut Cx<Self>,
        mut painter: Painter,
        size: (u32, u32),
        opened: Opened,
        serial: u32,
    ) -> Result<WindowId, bande_app::Error> {
        let (settings, home, bus, carrying) = (
            self.settings.clone(),
            self.home.clone(),
            Rc::clone(&self.bus),
            Rc::clone(&self.carrying),
        );
        let window = cx.open(Kind::Main, &L10N.tr("title"), size, |appearance| {
            painter.set_appearance(appearance);
            TerminalWindow::new(painter, settings, opened, home, bus, carrying, serial)
        })?;
        if let Some((shown, view)) = cx.window(window) {
            view.attach(shown);
        }
        Ok(window)
    }

    fn open_window(
        &mut self,
        cx: &mut Cx<Self>,
        folders: Vec<Option<PathBuf>>,
        active: usize,
        mut command: Option<alacritty_terminal::tty::Shell>,
    ) -> Result<WindowId, bande_app::Error> {
        let (serial, saved_file) = self.next_window(command.is_none());
        let grid = GridSize {
            columns: self.settings.columns,
            lines: self.settings.rows,
        };
        let painter = self.painter();
        let (width, height) = painter.metrics.content_size(grid);
        let size = (width.ceil() as u32, height.ceil() as u32);
        let mut sessions = Vec::new();
        for folder in folders {
            let mut terminal = Terminal::new(grid);
            terminal.resize(grid, painter.metrics.pixel_cell_size());
            let folder = folder
                .filter(|folder| folder.is_dir())
                .or_else(|| self.home.clone());
            terminal.set_options(self.settings.cursor.shape(), self.settings.scrollback);
            let shell = command.take().or_else(|| self.settings.shell());
            match Session::spawn(terminal, shell, folder) {
                Ok(session) => sessions.push(session),
                Err(error) if !sessions.is_empty() => {
                    tracing::warn!("cannot restore a tab: {error}");
                }
                Err(error) => return Err(error.into()),
            }
        }
        let opened = Opened {
            sessions,
            active,
            saved_file,
        };
        self.show(cx, painter, size, opened, serial)
    }

    fn detach(&mut self, cx: &mut Cx<Self>, from: WindowId, detached: Detached) {
        let Detached {
            tab,
            index,
            drag,
            grab,
            size,
        } = detached;
        let id = tab.id();
        match self.float(cx, tab, &drag, grab, size) {
            Ok(holder) => {
                self.carried = Some(Carried {
                    from,
                    index,
                    tab: id,
                    holder,
                    floating: true,
                    drag,
                    grab,
                    size,
                });
            }
            Err(tab) => {
                self.carrying.set(false);
                if let Some((shown, view)) = cx.window(from) {
                    view.adopt_tab(shown, tab, index);
                }
            }
        }
    }

    fn float(
        &mut self,
        cx: &mut Cx<Self>,
        tab: CarriedTab,
        drag: &WindowDrag,
        grab: Point,
        size: (u32, u32),
    ) -> Result<WindowId, CarriedTab> {
        let (serial, saved_file) = self.next_window(true);
        let opened = Opened {
            sessions: Vec::new(),
            active: 0,
            saved_file,
        };
        let window = match self.show(cx, self.painter(), size, opened, serial) {
            Ok(window) => window,
            Err(error) => {
                tracing::warn!("cannot open a window for the tab: {error}");
                return Err(tab);
            }
        };
        if let Some((shown, view)) = cx.window(window) {
            view.adopt_tab(shown, tab, 0);
            if let Err(error) = shown.attach_to_drag(drag, view.grab_point(grab)) {
                tracing::warn!("cannot carry the window with the tab: {error}");
            }
        }
        Ok(window)
    }

    fn hover(&mut self, cx: &mut Cx<Self>, over: WindowId, x: f32) {
        let Some(carried) = &mut self.carried else {
            return;
        };
        if carried.holder == over {
            if let Some((shown, view)) = cx.window(over) {
                view.slide_tab_to(shown, carried.tab, x);
            }
            return;
        }
        if !carried.floating {
            return;
        }
        let Some(index) = cx.view(over).map(|view| view.insertion_index(x)) else {
            return;
        };
        let Some(tab) = cx
            .window(carried.holder)
            .and_then(|(shown, view)| view.release_tab(shown, carried.tab))
        else {
            return;
        };
        cx.close(carried.holder);
        if let Some((shown, view)) = cx.window(over) {
            view.adopt_tab(shown, tab, index);
        }
        carried.holder = over;
        carried.floating = false;
    }

    fn unhover(&mut self, cx: &mut Cx<Self>, from: WindowId) {
        let Some(carried) = self.carried.take() else {
            return;
        };
        if carried.floating || carried.holder != from {
            self.carried = Some(carried);
            return;
        }
        let Some(tab) = cx
            .window(from)
            .and_then(|(shown, view)| view.release_tab(shown, carried.tab))
        else {
            self.carried = Some(carried);
            return;
        };
        match self.float(cx, tab, &carried.drag, carried.grab, carried.size) {
            Ok(holder) => {
                self.carried = Some(Carried {
                    holder,
                    floating: true,
                    ..carried
                });
            }
            Err(tab) => {
                if let Some((shown, view)) = cx.window(from) {
                    view.adopt_tab(shown, tab, 0);
                }
                self.carried = Some(carried);
            }
        }
    }

    fn end_carry(&mut self, cx: &mut Cx<Self>, placed: bool) {
        self.carrying.set(false);
        let Some(carried) = self.carried.take() else {
            return;
        };
        if placed {
            cx.raise(carried.holder, None);
            return;
        }
        if carried.holder != carried.from
            && let Some(tab) = cx
                .window(carried.holder)
                .and_then(|(shown, view)| view.release_tab(shown, carried.tab))
        {
            if carried.floating {
                cx.close(carried.holder);
            }
            if let Some((shown, view)) = cx.window(carried.from) {
                view.adopt_tab(shown, tab, carried.index);
            }
        }
        cx.raise(carried.from, None);
    }

    fn open_arguments(
        &mut self,
        cx: &mut Cx<Self>,
        arguments: Vec<String>,
        directory: &Path,
    ) -> Result<WindowId, bande_app::Error> {
        let arguments = Arguments::parse(arguments.into_iter());
        let folder = arguments
            .directory
            .map(|folder| directory.join(folder))
            .or_else(|| self.home.clone());
        self.open_window(cx, vec![folder], 0, arguments.command)
    }

    fn restore(&mut self, cx: &mut Cx<Self>) -> usize {
        let Some(folder) = windows_folder() else {
            return 0;
        };
        let mut restored = 0;
        for file in left_behind(&folder, running) {
            let mine = window_file(&folder, std::process::id(), self.next_serial);
            let Some(window) = claim(&file, &mine) else {
                continue;
            };
            let folders = window.tabs.into_iter().map(Some).collect();
            match self.open_window(cx, folders, window.active, None) {
                Ok(_) => restored += 1,
                Err(error) => tracing::warn!("cannot restore a window: {error}"),
            }
        }
        restored
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let asks = cx
            .window(window)
            .map(|(_, view)| view.take_asks())
            .unwrap_or_default();
        for ask in asks {
            match ask {
                Ask::NewWindow(folder) => {
                    if let Err(error) = self.open_window(cx, vec![folder], 0, None) {
                        tracing::warn!("cannot open a window: {error}");
                    }
                }
                Ask::Settings => cx.open_settings(),
                Ask::Quit => cx.quit(),
                Ask::Detach(detached) => self.detach(cx, window, detached),
                Ask::Hover(x) => self.hover(cx, window, x),
                Ask::Unhover => self.unhover(cx, window),
                Ask::Drop => self.end_carry(cx, true),
                Ask::DragEnded { placed } => self.end_carry(cx, placed),
            }
        }
        if cx.view(window).is_some_and(TerminalWindow::finished) {
            cx.close(window);
        }
    }
}

impl Program for Terminals {
    type View = TerminalWindow;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let folder = settings_folder();
        let mut terminals = Self {
            settings: folder.as_deref().map(Settings::load).unwrap_or_default(),
            settings_watch: folder
                .as_deref()
                .and_then(|folder| SettingsWatch::start(folder, Settings::FILE)),
            home: std::env::var_os("HOME").map(PathBuf::from),
            bus: Rc::new(Bus::start(APP_ID)),
            carrying: Rc::new(Cell::new(false)),
            carried: None,
            next_serial: 0,
        };
        let plain = Arguments::parse(arguments.iter().cloned()).is_plain();
        if !plain || terminals.restore(cx) == 0 {
            let directory = std::env::current_dir().unwrap_or_default();
            terminals.open_arguments(cx, arguments, &directory)?;
        }
        Ok(terminals)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        app_menus(cx.windows().next().is_some())
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        let Some(window) = window else {
            if let Some(MenuCommand::Run(Command::NewWindow | Command::NewTab)) =
                MenuCommand::parse(command)
            {
                self.reopen(cx);
            }
            return;
        };
        if command == CLOSE_WINDOW {
            if self.should_close(cx, window) {
                cx.close(window);
            }
            return;
        }
        if let Some((shown, view)) = cx.window(window) {
            view.menu_action(shown, command);
            shown.request_redraw();
        }
        self.serve(cx, window);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        if let Err(error) = self.open_arguments(cx, arguments, directory) {
            tracing::warn!("cannot open a window: {error}");
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if let Err(error) = self.open_window(cx, vec![self.home.clone()], 0, None) {
            tracing::warn!("cannot open a window: {error}");
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn close_tab(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        if let Some((shown, view)) = cx.window(window) {
            view.close_command(shown);
            shown.request_redraw();
        }
        self.serve(cx, window);
        true
    }

    fn should_close(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        let closing = cx
            .window(window)
            .is_some_and(|(shown, view)| view.close_by_button(shown));
        self.serve(cx, window);
        closing
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        let windows: Vec<WindowId> = cx.windows().collect();
        let busy: usize = windows
            .iter()
            .filter_map(|window| cx.view(*window))
            .map(TerminalWindow::busy_tabs)
            .sum();
        let asks = match self.settings.close_ask {
            CloseAsk::Always => !windows.is_empty(),
            CloseAsk::WhenRunning => busy > 0,
            CloseAsk::Never => false,
        };
        if !asks {
            return true;
        }
        let asking = cx.focused().filter(|window| windows.contains(window));
        if let Some((shown, view)) = asking
            .or_else(|| windows.first().copied())
            .and_then(|window| cx.window(window))
        {
            view.ask_to_quit(shown, busy);
        }
        false
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.settings_watch.iter().map(SettingsWatch::fd).collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        if let Some(watch) = &mut self.settings_watch {
            watch.drain();
        }
        if let Some(folder) = settings_folder() {
            self.settings = Settings::load(&folder);
        }
        for window in cx.windows().collect::<Vec<_>>() {
            if let Some((shown, view)) = cx.window(window) {
                view.reload_settings(shown);
            }
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Terminals>(&INFO)
}
