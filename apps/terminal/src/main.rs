mod app;
mod bus;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use bande_app::{
    Cx, Info, Keys, Kind, Menus, Program, Settings as SettingsWindowInfo, View, WindowId,
};
use bande_app_settings::{SettingsWatch, SettingsWindow, Stored, settings_folder};
use bande_terminal::{
    COLUMNS, Command, GridSize, L10N, MenuCommand, Painter, ROWS, SETTINGS_SIZE, Session, Settings,
    Terminal, app_menus, claim, left_behind, settings_view, window_file, windows_folder,
};
use bande_toolkit::Appearance;

use crate::app::{Arguments, Ask, Opened, TerminalWindow};
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

struct Terminals {
    settings: Settings,
    settings_watch: Option<SettingsWatch>,
    home: Option<PathBuf>,
    bus: Rc<Bus>,
    next_serial: u32,
}

impl Terminals {
    fn open_window(
        &mut self,
        cx: &mut Cx<Self>,
        folders: Vec<Option<PathBuf>>,
        active: usize,
        mut command: Option<alacritty_terminal::tty::Shell>,
    ) -> Result<WindowId, bande_app::Error> {
        let serial = self.next_serial;
        self.next_serial += 1;
        let saved_file = windows_folder()
            .filter(|_| command.is_none())
            .map(|folder| window_file(&folder, std::process::id(), serial));
        let grid = GridSize {
            columns: COLUMNS,
            lines: ROWS,
        };
        let mut painter = Painter::new(
            Appearance::default(),
            self.settings.profile,
            self.settings.font_size,
            1.0,
        );
        let (width, height) = painter.metrics.content_size(grid);
        let size = (width.ceil() as u32, height.ceil() as u32);
        let mut sessions = Vec::new();
        for folder in folders {
            let mut terminal = Terminal::new(grid);
            terminal.resize(grid, painter.metrics.pixel_cell_size());
            let folder = folder
                .filter(|folder| folder.is_dir())
                .or_else(|| self.home.clone());
            match Session::spawn(terminal, command.take(), folder) {
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
        let (settings, home, bus) = (self.settings, self.home.clone(), Rc::clone(&self.bus));
        let window = cx.open(Kind::Main, &L10N.tr("title"), size, |appearance| {
            painter.set_appearance(appearance);
            TerminalWindow::new(painter, settings, opened, home, bus, serial)
        })?;
        if let Some((shown, view)) = cx.window(window) {
            view.attach(shown);
        }
        Ok(window)
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
            next_serial: 0,
        };
        let plain = Arguments::parse(arguments.iter().cloned()).is_plain();
        if !plain || terminals.restore(cx) == 0 {
            let directory = std::env::current_dir().unwrap_or_default();
            terminals.open_arguments(cx, arguments, &directory)?;
        }
        Ok(terminals)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        app_menus()
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
        if busy == 0 {
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
