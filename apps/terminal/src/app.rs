use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::Shell;
use unbound_term::{
    CellPoint, ClickCounter, Command, DEFAULT_FONT_SIZE, GridSize, L10N, MouseButton, MouseKind,
    Notice, Painter, SelectionKind, Session, TAB_BAR_HEIGHT, TabBar, TabBarLayout, TabTarget,
    Terminal, bracketed, encode_key, mouse_report, next_font_size, reports_mouse, shortcut,
    wants_scroll_as_arrows,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{App, Point, Scene, Window, WindowEvent};

pub const APP_ID: &str = "org.unbound.Terminal";
const WRITE_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Idle,
    Selecting,
    Reporting(MouseButton),
    Tab(TabTarget),
}

pub struct TerminalApp {
    tabs: Vec<Session>,
    active: usize,
    painter: Painter,
    home: Option<PathBuf>,
    focused: bool,
    pointer: Point,
    gesture: Gesture,
    hover: Option<TabTarget>,
    clicks: ClickCounter,
    reported_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
    title: String,
    size: (u32, u32),
}

impl TerminalApp {
    pub fn new(window: &Window, painter: Painter, first: Session, home: Option<PathBuf>) -> Self {
        let mut app = Self {
            tabs: vec![first],
            active: 0,
            painter,
            home,
            focused: false,
            pointer: Point::default(),
            gesture: Gesture::Idle,
            hover: None,
            clicks: ClickCounter::default(),
            reported_cell: None,
            scroll_remainder: 0.0,
            title: String::new(),
            size: window.size(),
        };
        app.tabs[0].terminal.set_focus(false);
        app
    }

    fn session(&mut self) -> &mut Session {
        &mut self.tabs[self.active]
    }

    fn terminal(&self) -> &Terminal {
        &self.tabs[self.active].terminal
    }

    fn mode(&self) -> TermMode {
        self.terminal().mode()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.session().send(bytes);
    }

    fn shows_tab_bar(&self) -> bool {
        self.tabs.len() > 1
    }

    fn content_origin(&self) -> Point {
        let top = if self.shows_tab_bar() {
            TAB_BAR_HEIGHT
        } else {
            0.0
        };
        Point::new(0.0, top)
    }

    fn grid(&self) -> GridSize {
        let origin = self.content_origin();
        self.painter
            .metrics
            .grid_for(self.size.0 as f32 - origin.x, self.size.1 as f32 - origin.y)
    }

    fn layout(&mut self) {
        let grid = self.grid();
        let cell_size = self.painter.metrics.pixel_cell_size();
        for tab in &mut self.tabs {
            tab.resize(grid, cell_size);
        }
    }

    fn cell_at(&self, at: Point) -> CellPoint {
        let origin = self.content_origin();
        self.painter
            .metrics
            .cell_at(Point::new(at.x - origin.x, at.y - origin.y))
    }

    fn tab_target(&self, at: Point) -> Option<TabTarget> {
        if !self.shows_tab_bar() || at.y >= TAB_BAR_HEIGHT {
            return None;
        }
        TabBarLayout::new(self.tabs.len(), self.size.0 as f32).hit(at)
    }

    fn titles(&self) -> Vec<String> {
        self.tabs
            .iter()
            .map(|tab| {
                let title = tab.title(self.home.as_deref());
                if title.is_empty() {
                    L10N.tr("title")
                } else {
                    title
                }
            })
            .collect()
    }

    fn sync_title(&mut self, window: &mut Window) {
        let title = self.titles().swap_remove(self.active);
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
    }

    fn handle_notices(&mut self, window: &mut Window, notices: Vec<Notice>) {
        for notice in notices {
            match notice {
                Notice::Copy(text) => window.copy(&text),
                Notice::Bell => {}
            }
        }
    }

    fn select_tab(&mut self, window: &mut Window, index: usize) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        let focused = self.focused;
        if let Some(report) = self.session().terminal.set_focus(false) {
            self.send(report);
        }
        self.active = index;
        self.gesture = Gesture::Idle;
        if let Some(report) = self.session().terminal.set_focus(focused) {
            self.send(report);
        }
        window.request_redraw();
    }

    fn new_tab(&mut self, window: &mut Window) {
        let directory = self
            .session()
            .working_directory()
            .or_else(|| self.home.clone());
        let mut terminal = Terminal::new(self.terminal().size());
        terminal.resize(
            self.terminal().size(),
            self.painter.metrics.pixel_cell_size(),
        );
        match Session::spawn(terminal, None, directory) {
            Ok(session) => {
                self.tabs.push(session);
                self.layout();
                self.select_tab(window, self.tabs.len() - 1);
            }
            Err(error) => tracing::warn!("cannot open a tab: {error}"),
        }
    }

    fn new_window(&mut self, window: &mut Window) {
        let directory = self
            .session()
            .working_directory()
            .or_else(|| self.home.clone());
        let program = match std::env::current_exe() {
            Ok(program) => program.to_string_lossy().into_owned(),
            Err(error) => return tracing::warn!("cannot find the terminal program: {error}"),
        };
        let token = window
            .activation_token(APP_ID)
            .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
            .ok();
        let mut command = vec![program];
        if let Some(directory) = directory {
            command.push(DIRECTORY_OPTION.to_owned());
            command.push(directory.to_string_lossy().into_owned());
        }
        if let Err(error) = unbound_xdg::spawn_detached(&command, None, token.as_deref()) {
            tracing::warn!("cannot open a window: {error}");
        }
    }

    pub fn close_tab(&mut self, window: &mut Window, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let was_active = index == self.active;
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return;
        }
        if index < self.active || self.active >= self.tabs.len() {
            self.active -= 1;
        }
        if was_active {
            let focused = self.focused;
            if let Some(report) = self.session().terminal.set_focus(focused) {
                self.send(report);
            }
        }
        self.gesture = Gesture::Idle;
        self.hover = None;
        self.layout();
        window.request_redraw();
    }

    fn remove_exited(&mut self, window: &mut Window) {
        while let Some(index) = self.tabs.iter().position(Session::exited) {
            self.close_tab(window, index);
        }
    }

    fn key(&mut self, window: &mut Window, key: Key, modifiers: Modifiers) {
        if let Some(command) = shortcut(key, modifiers) {
            return self.perform(window, command);
        }
        if modifiers.logo {
            return;
        }
        let mode = self.mode();
        let in_alternate_screen = mode.contains(TermMode::ALT_SCREEN);
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
        let shifted_page = matches!(key, Key::PageUp | Key::PageDown)
            && modifiers.shift
            && unshifted == Modifiers::default();
        if shifted_page && !in_alternate_screen {
            return;
        }
        let modifiers = if matches!(key, Key::PageUp | Key::PageDown) {
            unshifted
        } else {
            modifiers
        };
        if let Some(bytes) = encode_key(key, modifiers, mode) {
            self.session().terminal.scroll_to_bottom();
            self.send(&bytes);
            window.request_redraw();
        }
    }

    fn perform(&mut self, window: &mut Window, command: Command) {
        match command {
            Command::Copy => {
                if let Some(text) = self.terminal().selected_text() {
                    window.copy(&text);
                }
            }
            Command::Paste => {
                if let Err(error) = window.paste() {
                    tracing::warn!("cannot paste: {error}");
                }
            }
            Command::SelectAll => self.session().terminal.select_all(),
            Command::ClearScrollback => self.session().terminal.clear_scrollback(),
            Command::BiggerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), true))
            }
            Command::SmallerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), false))
            }
            Command::DefaultTextSize => self.set_font_size(window, DEFAULT_FONT_SIZE),
            Command::NewTab => self.new_tab(window),
            Command::NewWindow => self.new_window(window),
            Command::NextTab => self.select_tab(window, (self.active + 1) % self.tabs.len()),
            Command::PreviousTab => {
                let count = self.tabs.len();
                self.select_tab(window, (self.active + count - 1) % count);
            }
            Command::SelectTab(index) => self.select_tab(window, index),
            Command::LastTab => self.select_tab(window, self.tabs.len() - 1),
        }
        window.request_redraw();
    }

    fn set_font_size(&mut self, window: &Window, size: f32) {
        self.painter.set_metrics(size, window.scale());
        self.scroll_remainder = 0.0;
        self.layout();
    }

    fn report_mouse(&mut self, kind: MouseKind, at: Point, modifiers: Modifiers) -> bool {
        let cell = self.cell_at(at);
        let size = self.terminal().size();
        let place = (
            cell.column.min(size.columns - 1),
            cell.line.min(size.lines - 1),
        );
        let moving = matches!(kind, MouseKind::Drag(_) | MouseKind::Move);
        if moving && self.reported_cell == Some(place) {
            return true;
        }
        let Some(bytes) = mouse_report(kind, place.0, place.1, modifiers, self.mode()) else {
            return false;
        };
        self.reported_cell = Some(place);
        self.send(&bytes);
        true
    }

    fn pointer_moved(&mut self, window: &mut Window, at: Point) {
        self.pointer = at;
        let modifiers = window.modifiers();
        let hover = self.tab_target(at);
        if hover != self.hover {
            self.hover = hover;
            window.request_redraw();
        }
        match self.gesture {
            Gesture::Reporting(button) => {
                self.report_mouse(MouseKind::Drag(button), at, modifiers);
            }
            Gesture::Selecting => {
                let cell = self.cell_at(at);
                self.session().terminal.extend_selection(cell);
                window.request_redraw();
            }
            Gesture::Tab(_) => {}
            Gesture::Idle if hover.is_none() => {
                self.report_mouse(MouseKind::Move, at, modifiers);
            }
            Gesture::Idle => {}
        }
    }

    fn release(&mut self, window: &mut Window, at: Point) {
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Reporting(button) => {
                self.report_mouse(MouseKind::Release(button), at, window.modifiers());
            }
            Gesture::Selecting => {
                self.session().terminal.finish_selection();
                window.request_redraw();
            }
            Gesture::Tab(target) if self.tab_target(at) == Some(target) => match target {
                TabTarget::Close(index) => self.close_tab(window, index),
                TabTarget::NewTab => self.new_tab(window),
                TabTarget::Select(_) => {}
            },
            Gesture::Tab(_) | Gesture::Idle => {}
        }
    }

    fn press(&mut self, window: &mut Window, at: Point) {
        if let Some(target) = self.tab_target(at) {
            if let TabTarget::Select(index) = target {
                self.select_tab(window, index);
            }
            self.gesture = Gesture::Tab(target);
            return;
        }
        let modifiers = window.modifiers();
        if !modifiers.shift && reports_mouse(self.mode()) {
            self.session().terminal.clear_selection();
            self.reported_cell = None;
            if self.report_mouse(MouseKind::Press(MouseButton::Left), at, modifiers) {
                self.gesture = Gesture::Reporting(MouseButton::Left);
            }
            window.request_redraw();
            return;
        }
        let clicks = self.clicks.press(at, Instant::now());
        let cell = self.cell_at(at);
        self.gesture = Gesture::Selecting;
        let terminal = &mut self.session().terminal;
        if modifiers.shift && terminal.has_selection() {
            terminal.extend_selection(cell);
        } else {
            terminal.start_selection(cell, SelectionKind::for_clicks(clicks, modifiers.alt));
        }
        window.request_redraw();
    }

    fn scroll_pages(&mut self, window: &mut Window, pages: i32) {
        self.session().terminal.scroll_pages(pages);
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
        let modifiers = window.modifiers();
        if !modifiers.shift && reports_mouse(self.mode()) {
            let button = if lines < 0 {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                self.reported_cell = None;
                self.report_mouse(MouseKind::Press(button), self.pointer, modifiers);
            }
            return;
        }
        if wants_scroll_as_arrows(self.mode()) {
            let key = if lines < 0 { Key::Up } else { Key::Down };
            for _ in 0..lines.unsigned_abs() {
                if let Some(bytes) = encode_key(key, Modifiers::default(), self.mode()) {
                    self.send(&bytes);
                }
            }
            return;
        }
        self.session().terminal.scroll_lines(-lines);
        window.request_redraw();
    }

    fn focus(&mut self, window: &mut Window, focused: bool) {
        self.focused = focused;
        if let Some(report) = self.session().terminal.set_focus(focused) {
            self.send(report);
        }
        window.request_redraw();
    }
}

impl App for TerminalApp {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let size = window.size();
        let rescaled = self
            .painter
            .set_metrics(self.painter.font_size(), window.scale());
        if rescaled || size != self.size {
            self.size = size;
            self.layout();
        }
        let mut items = Vec::new();
        if self.shows_tab_bar() {
            let titles = self.titles();
            let bar = TabBar {
                titles: &titles,
                active: self.active,
                hover: self.hover,
                focused: self.focused,
            };
            items.extend(self.painter.tab_bar(&bar, size.0 as f32));
        }
        let origin = self.content_origin();
        items.extend(
            self.painter
                .terminal(&self.tabs[self.active].terminal, origin),
        );
        Scene {
            width: size.0,
            height: size.1,
            background: self.painter.theme.background,
            items,
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key { key, modifiers }) => self.key(window, key, modifiers),
            WindowEvent::Input(Event::PointerDown(at)) => self.press(window, at),
            WindowEvent::Input(Event::PointerMove(at)) => self.pointer_moved(window, at),
            WindowEvent::Input(Event::PointerUp(at)) => self.release(window, at),
            WindowEvent::PointerLeft => {
                if self.hover.take().is_some() {
                    window.request_redraw();
                }
            }
            WindowEvent::Scroll { dy, .. } => self.scroll(window, dy),
            WindowEvent::Pasted(text) => {
                let bytes = bracketed(&text, self.mode());
                self.session().terminal.scroll_to_bottom();
                self.send(&bytes);
            }
            WindowEvent::Focus(focused) => self.focus(window, focused),
            WindowEvent::AppearanceChanged(appearance) => self.painter.set_appearance(appearance),
            _ => {}
        }
        let notices = self.session().take_notices();
        self.handle_notices(window, notices);
        self.remove_exited(window);
        if !self.tabs.is_empty() {
            self.sync_title(window);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.tabs.iter().map(Session::fd).collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let notices = tab.read();
        self.handle_notices(window, notices);
        if index == self.active || self.shows_tab_bar() {
            window.request_redraw();
        }
        self.remove_exited(window);
        if !self.tabs.is_empty() {
            self.sync_title(window);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.tabs
            .iter()
            .any(Session::has_pending_output)
            .then(|| Instant::now() + WRITE_RETRY)
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) {
        for tab in &mut self.tabs {
            tab.flush();
        }
    }

    fn finished(&self) -> bool {
        self.tabs.is_empty()
    }
}

pub const DIRECTORY_OPTION: &str = "--working-directory";

#[derive(Debug, Default)]
pub struct Arguments {
    pub command: Option<Shell>,
    pub directory: Option<PathBuf>,
}

impl Arguments {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Self {
        let mut parsed = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-e" => {
                    if let Some(program) = args.next() {
                        parsed.command = Some(Shell::new(program, args.collect()));
                    }
                    break;
                }
                DIRECTORY_OPTION => parsed.directory = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        parsed
    }
}
