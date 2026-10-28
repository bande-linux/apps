use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, OnResize};
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use unbound_config::{Appearance, Config, config_path};
use unbound_term::{
    COLUMNS, GridSize, ROWS, Terminal, bracketed, encode_key, wants_scroll_as_arrows,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{App, Role, Scene, Scheme, Window, WindowError, WindowEvent};

const TITLE: &str = "Terminal";
const WRITE_RETRY: Duration = Duration::from_millis(5);

struct TerminalApp {
    terminal: Terminal,
    pty: Pty,
    output: Vec<u8>,
    selecting: bool,
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
                TermEvent::ResetTitle => window.set_title(TITLE),
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
        let only_logo = Modifiers {
            logo: true,
            ..Modifiers::default()
        };
        if modifiers == only_logo {
            match key {
                Key::Character('c') => {
                    if let Some(text) = self.terminal.selected_text() {
                        window.copy(&text);
                    }
                }
                Key::Character('v') => {
                    if let Err(error) = window.paste() {
                        eprintln!("unbound-term: cannot paste: {error}");
                    }
                }
                Key::Character('k') => {
                    self.terminal.clear_scrollback();
                    window.request_redraw();
                }
                _ => {}
            }
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

    fn scroll_pages(&mut self, window: &mut Window, pages: i32) {
        self.terminal.scroll_pages(pages);
        window.request_redraw();
    }

    fn scroll(&mut self, window: &mut Window, dy: f32) {
        let lines = self.terminal.scroll_pixels(dy);
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
        let grid = self.terminal.metrics.grid_for(width, height);
        if self.terminal.resize(grid) {
            self.pty.on_resize(self.terminal.window_size());
        }
        self.terminal.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key { key, modifiers }) => self.key(window, key, modifiers),
            WindowEvent::Input(Event::PointerDown(at)) => {
                self.selecting = true;
                self.terminal.start_selection(at);
                window.request_redraw();
            }
            WindowEvent::Input(Event::PointerMove(at)) if self.selecting => {
                self.terminal.extend_selection(at);
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
                self.terminal.set_focus(focused);
                window.request_redraw();
            }
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
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let config = config_path(config_home.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default();
    let scheme = match config.appearance {
        Appearance::Dark => Scheme::Dark,
        Appearance::Light | Appearance::Automatic => Scheme::Light,
    };
    let terminal = Terminal::new(
        GridSize {
            columns: COLUMNS,
            lines: ROWS,
        },
        scheme,
    );
    let size = terminal.metrics.window_size(terminal.size());
    let mut window = Window::open(Role::toplevel(TITLE, "unbound-term"), size)?;
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
        pty,
        output: Vec::new(),
        selecting: false,
        exited: false,
    };
    app.terminal.set_focus(false);
    unbound_toolkit::run(&mut window, &mut app)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-term: {error}");
            ExitCode::FAILURE
        }
    }
}
