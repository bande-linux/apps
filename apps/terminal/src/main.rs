mod app;
mod bus;

use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use bande_app::{
    Cx, Detached, Info, Keys, Kind, Menus, Program, Saved, Settings as SettingsWindowInfo,
    TabCarry, TabDrop, View, WindowId,
};
use bande_app_settings::{SettingsWatch, SettingsWindow, Stored, settings_folder};
use bande_terminal::{
    CLOSE_WINDOW, CloseAsk, Command, GridSize, L10N, MenuCommand, Painter, SETTINGS_SIZE, Session,
    Settings, Terminal, app_menus, settings_view,
};
use bande_toolkit::Appearance;

use crate::app::{Arguments, Ask, CarriedTab, Opened, SavedWindow, TerminalWindow};
use crate::bus::Bus;

const APP_ID: &str = "org.bande.Terminal";

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

struct Terminals {
    settings: Settings,
    settings_watch: Option<SettingsWatch>,
    home: Option<PathBuf>,
    bus: Rc<Bus>,
    carrying: Rc<Cell<bool>>,
    carry: TabCarry,
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

    fn next_serial(&mut self) -> u32 {
        let serial = self.next_serial;
        self.next_serial += 1;
        serial
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
        let serial = self.next_serial();
        let remembered = command.is_none();
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
            remembered,
        };
        self.show(cx, painter, size, opened, serial)
    }

    fn float_window(
        &mut self,
        cx: &mut Cx<Self>,
        size: (u32, u32),
    ) -> Result<WindowId, bande_app::Error> {
        let serial = self.next_serial();
        let opened = Opened {
            sessions: Vec::new(),
            active: 0,
            remembered: true,
        };
        self.show(cx, self.painter(), size, opened, serial)
    }

    fn detach(&mut self, cx: &mut Cx<Self>, from: WindowId, detached: Detached<CarriedTab>) {
        let mut carry = std::mem::take(&mut self.carry);
        carry.detach(cx, from, detached, &mut |cx, size| {
            self.float_window(cx, size)
        });
        self.carry = carry;
        self.carrying.set(self.carry.is_carrying());
    }

    fn tab_drop(&mut self, cx: &mut Cx<Self>, window: WindowId, drop: TabDrop) {
        let mut carry = std::mem::take(&mut self.carry);
        carry.drop_event(cx, window, drop, &mut |cx, size| {
            self.float_window(cx, size)
        });
        self.carry = carry;
        self.carrying.set(self.carry.is_carrying());
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
                Ask::TabDrop(drop) => self.tab_drop(cx, window, drop),
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
            carry: TabCarry::default(),
            next_serial: 0,
        };
        if !arguments.is_empty() {
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

    fn save_window(&self, cx: &Cx<Self>, window: WindowId) -> Option<Saved> {
        Saved::of(&cx.view(window)?.saved_window()?)
    }

    fn restore_window(&mut self, cx: &mut Cx<Self>, saved: Saved) {
        let Some(window) = saved
            .read::<SavedWindow>()
            .filter(|window| !window.tabs.is_empty())
        else {
            return;
        };
        let folders = window.tabs.into_iter().map(Some).collect();
        if let Err(error) = self.open_window(cx, folders, window.active, None) {
            tracing::warn!("cannot reopen a window: {error}");
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
