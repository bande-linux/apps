mod app;

use std::path::PathBuf;
use std::process::ExitCode;

use unbound_term::{COLUMNS, DEFAULT_FONT_SIZE, GridSize, L10N, Painter, ROWS, Session, Terminal};
use unbound_toolkit::{Role, Window, WindowError};

use crate::app::{APP_ID, Arguments, TerminalApp};

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let arguments = Arguments::parse(std::env::args().skip(1));
    let directory = arguments.directory.or_else(|| home.clone());
    let grid = GridSize {
        columns: COLUMNS,
        lines: ROWS,
    };
    let mut painter = Painter::new(Default::default(), DEFAULT_FONT_SIZE, 1.0);
    let (width, height) = painter.metrics.content_size(grid);
    let size = (width.ceil() as u32, height.ceil() as u32);
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), size)?;
    painter.set_appearance(window.appearance());
    painter.set_metrics(DEFAULT_FONT_SIZE, window.scale());
    let mut terminal = Terminal::new(grid);
    terminal.resize(grid, painter.metrics.pixel_cell_size());
    let session = Session::spawn(terminal, arguments.command, directory)?;
    let mut app = TerminalApp::new(&window, painter, session, home);
    unbound_toolkit::run(&mut window, &mut app)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-term");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
