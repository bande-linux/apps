use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, OnResize};
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use unbound_term::{
    COLUMNS, ClickCounter, Command, DEFAULT_FONT_SIZE, GridSize, L10N, Painter, ROWS,
    SelectionKind, Terminal, bracketed, encode_key, next_font_size, shortcut,
    wants_scroll_as_arrows,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{App, Point, Role, Scene, Window, WindowError, WindowEvent};

const APP_ID: &str = "org.unbound.Terminal";
const WRITE_RETRY: Duration = Duration::from_millis(5);

struct TerminalApp {
    terminal: Terminal,
    painter: Painter,
    scroll_remainder: f32,
    pty: Pty,
    output: Vec<u8>,
    selecting: bool,
    clicks: ClickCounter,
    exited: bool,
}

impl TerminalApp {
    fn master(&self) -> &File {
        self.pty.file()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
        self.flush_output();
    }

    fn flush_output(&mut self) {
        while !self.output.is_empty() {
            let mut master = self.pty.file();
            match master.write(&self.output) {
                Ok(0) => break,
                Ok(written) => {
                    self.output.drain(..written);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }

    fn handle_terminal_events(&mut self, window: &mut Window) {
        for event in self.terminal.take_events() {
            match event {
                TermEvent::PtyWrite(text) => self.send(text.as_bytes()),
                TermEvent::Title(title) => window.set_title(&title),
                TermEvent::ResetTitle => window.set_title(&L10N.tr("title")),
                TermEvent::ClipboardStore(_, text) => window.copy(&text),
                TermEvent::TextAreaSizeRequest(reply) => {
                    let text = reply(self.terminal.window_size());
                    self.send(text.as_bytes());
                }
                TermEvent::ChildExit(_) | TermEvent::Exit => self.exited = true,
                _ => {}
            }
        }
    }

    fn key(&mut self, window: &mut Window, key: Key, modifiers: Modifiers) {
        if let Some(command) = shortcut(key, modifiers) {
            return self.perform(window, command);
        }
        if modifiers.logo {
            return;
        }
        let mode = self.terminal.mode();
        let in_alternate_screen = mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN);
        if modifiers == Modifiers::default() && !in_alternate_screen {
            match key {
                Key::PageUp => return self.scroll_pages(window, 1),
                Key::PageDown => return self.scroll_pages(window, -1),
                _ => {}
            }
        }
        let unshifted = Modifiers {
            shift: false,
            ..modifiers
        };
        let key = match key {
            Key::PageUp | Key::PageDown if modifiers.shift && unshifted == Modifiers::default() => {
                if !in_alternate_screen {
                    return;
                }
                key
            }
            other => other,
        };
        let modifiers = match key {
            Key::PageUp | Key::PageDown => unshifted,
            _ => modifiers,
        };
        if let Some(bytes) = encode_key(key, modifiers, mode) {
            self.terminal.scroll_to_bottom();
            self.send(&bytes);
            window.request_redraw();
        }
    }

    fn perform(&mut self, window: &mut Window, command: Command) {
        match command {
            Command::Copy => {
                if let Some(text) = self.terminal.selected_text() {
                    window.copy(&text);
                }
            }
            Command::Paste => {
                if let Err(error) = window.paste() {
                    tracing::warn!("cannot paste: {error}");
                }
            }
            Command::SelectAll => self.terminal.select_all(),
            Command::ClearScrollback => self.terminal.clear_scrollback(),
            Command::BiggerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), true))
            }
            Command::SmallerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), false))
            }
            Command::DefaultTextSize => self.set_font_size(window, DEFAULT_FONT_SIZE),
        }
        window.request_redraw();
    }

    fn set_font_size(&mut self, window: &Window, size: f32) {
        self.painter.set_metrics(size, window.scale());
        self.scroll_remainder = 0.0;
    }

    fn press(&mut self, window: &mut Window, at: Point) {
        let modifiers = window.modifiers();
        let clicks = self.clicks.press(at, Instant::now());
        let cell = self.cell_at(at);
        self.selecting = true;
        if modifiers.shift && self.terminal.has_selection() {
            self.terminal.extend_selection(cell);
        } else {
            let kind = SelectionKind::for_clicks(clicks, modifiers.alt);
            self.terminal.start_selection(cell, kind);
        }
        window.request_redraw();
    }

    fn cell_at(&self, at: Point) -> unbound_term::CellPoint {
        self.painter.metrics.cell_at(at)
    }

    fn scroll_pages(&mut self, window: &mut Window, pages: i32) {
        self.terminal.scroll_pages(pages);
        window.request_redraw();
    }

    fn scroll(&mut self, window: &mut Window, dy: f32) {
        self.scroll_remainder += dy / self.painter.metrics.cell_height;
        let lines = self.scroll_remainder.trunc();
        self.scroll_remainder -= lines;
        let lines = lines as i32;
        if lines == 0 {
            return;
        }
        if wants_scroll_as_arrows(self.terminal.mode()) {
            let key = if lines < 0 { Key::Up } else { Key::Down };
            for _ in 0..lines.unsigned_abs() {
                if let Some(bytes) = encode_key(key, Modifiers::default(), self.terminal.mode()) {
                    self.send(&bytes);
                }
            }
            return;
        }
        self.terminal.scroll_lines(-lines);
        window.request_redraw();
    }
}

impl App for TerminalApp {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.painter
            .set_metrics(self.painter.font_size(), window.scale());
        let metrics = self.painter.metrics;
        let grid = metrics.grid_for(width as f32, height as f32);
        if self.terminal.resize(grid, metrics.pixel_cell_size()) {
            self.pty.on_resize(self.terminal.window_size());
        }
        self.painter.scene(&self.terminal, width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key { key, modifiers }) => self.key(window, key, modifiers),
            WindowEvent::Input(Event::PointerDown(at)) => self.press(window, at),
            WindowEvent::Input(Event::PointerMove(at)) if self.selecting => {
                self.terminal.extend_selection(self.cell_at(at));
                window.request_redraw();
            }
            WindowEvent::Input(Event::PointerUp(_)) => {
                self.selecting = false;
                self.terminal.finish_selection();
                window.request_redraw();
            }
            WindowEvent::Scroll { dy, .. } => self.scroll(window, dy),
            WindowEvent::Pasted(text) => {
                let bytes = bracketed(&text, self.terminal.mode());
                self.terminal.scroll_to_bottom();
                self.send(&bytes);
            }
            WindowEvent::Focus(focused) => {
                if let Some(report) = self.terminal.set_focus(focused) {
                    self.send(report);
                }
                window.request_redraw();
            }
            WindowEvent::AppearanceChanged(appearance) => self.painter.set_appearance(appearance),
            _ => {}
        }
        self.handle_terminal_events(window);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.master().as_fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let mut buffer = [0u8; 65536];
        loop {
            let mut master = self.pty.file();
            match master.read(&mut buffer) {
                Ok(0) => {
                    self.exited = true;
                    break;
                }
                Ok(read) => self.terminal.feed(&buffer[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.exited = true;
                    break;
                }
            }
        }
        window.request_redraw();
        self.handle_terminal_events(window);
        self.flush_output();
    }

    fn deadline(&self) -> Option<Instant> {
        (!self.output.is_empty()).then(|| Instant::now() + WRITE_RETRY)
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) {
        self.flush_output();
    }

    fn finished(&self) -> bool {
        self.exited
    }
}

fn command() -> Option<Shell> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-e" {
            let program = args.next()?;
            return Some(Shell::new(program, args.collect()));
        }
    }
    None
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
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
    let options = Options {
        shell: command(),
        working_directory: home,
        drain_on_exit: true,
        env: HashMap::from([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
            ("TERM_PROGRAM".to_owned(), "unbound-term".to_owned()),
        ]),
    };
    let pty = tty::new(&options, terminal.window_size(), 0)?;
    let mut app = TerminalApp {
        terminal,
        painter,
        scroll_remainder: 0.0,
        pty,
        output: Vec::new(),
        selecting: false,
        clicks: ClickCounter::default(),
        exited: false,
    };
    app.terminal.set_focus(false);
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
