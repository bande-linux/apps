mod app;
mod bus;
mod preferences;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use unbound_app_menu::AppMenu;
use unbound_term::{
    COLUMNS, GridSize, L10N, Painter, ROWS, SavedWindow, Session, Settings, Terminal, app_menus,
    claim, left_behind, settings_folder, window_file, windows_folder,
};
use unbound_toolkit::{Role, Window, WindowError};

use crate::app::{APP_ID, Arguments, RESTORE_OPTION, TerminalApp};

const PROGRAM: &str = "unbound-term";

fn running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .is_ok_and(|name| name.trim_end() == PROGRAM)
}

fn restore(arguments: &Arguments, folder: &Path, mine: &Path) -> Option<SavedWindow> {
    if let Some(file) = &arguments.restore {
        return claim(file, mine);
    }
    if !arguments.is_plain() {
        return None;
    }
    let mut left = left_behind(folder, running).into_iter();
    let first = left.find_map(|file| claim(&file, mine));
    for file in left {
        let command = [
            PROGRAM.to_owned(),
            RESTORE_OPTION.to_owned(),
            file.to_string_lossy().into_owned(),
        ];
        if let Err(error) = unbound_xdg::spawn_detached(APP_ID, &command, None, None) {
            tracing::warn!("cannot restore a window: {error}");
        }
    }
    first
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut arguments = Arguments::parse(std::env::args().skip(1));
    if arguments.settings {
        return preferences::run();
    }
    let settings = settings_folder()
        .map(|folder| Settings::load(&folder))
        .unwrap_or_default();
    let saved_file = windows_folder()
        .filter(|_| arguments.command.is_none())
        .map(|folder| {
            let mine = window_file(&folder, std::process::id());
            (folder, mine)
        });
    let restored = saved_file
        .as_ref()
        .and_then(|(folder, mine)| restore(&arguments, folder, mine));
    let (folders, active) = match restored {
        Some(window) => (window.tabs.into_iter().map(Some).collect(), window.active),
        None => (vec![arguments.directory.take().or_else(|| home.clone())], 0),
    };
    let grid = GridSize {
        columns: COLUMNS,
        lines: ROWS,
    };
    let mut painter = Painter::new(
        Default::default(),
        settings.profile,
        settings.font_size,
        1.0,
    );
    let (width, height) = painter.metrics.content_size(grid);
    let size = (width.ceil() as u32, height.ceil() as u32);
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), size)?;
    painter.set_appearance(window.appearance());
    painter.set_metrics(settings.font_size, window.scale());
    let mut command = arguments.command.take();
    let mut sessions = Vec::new();
    for folder in folders {
        let mut terminal = Terminal::new(grid);
        terminal.resize(grid, painter.metrics.pixel_cell_size());
        let folder = folder
            .filter(|folder| folder.is_dir())
            .or_else(|| home.clone());
        match Session::spawn(terminal, command.take(), folder) {
            Ok(session) => sessions.push(session),
            Err(error) if !sessions.is_empty() => tracing::warn!("cannot restore a tab: {error}"),
            Err(error) => return Err(error.into()),
        }
    }
    let app_menu = AppMenu::export(APP_ID, app_menus())
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    let mut app = TerminalApp::new(
        &mut window,
        painter,
        settings,
        (sessions, active),
        home,
        saved_file.map(|(_, mine)| mine),
        app_menu,
    );
    unbound_toolkit::run(&mut window, &mut app)
}

fn main() -> ExitCode {
    unbound_log::init(PROGRAM);
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
