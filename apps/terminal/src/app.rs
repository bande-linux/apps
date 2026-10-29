use std::collections::VecDeque;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use accesskit::{Action, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::Shell;
use unbound_term::{
    BANNER_HEIGHT, Banner, BannerAction, CellPoint, Command, DEFAULT_FONT_SIZE, GridSize, L10N,
    Link, MouseButton, MouseKind, Notice, Painter, SearchDirection, SelectionKind, Session,
    TAB_BAR_HEIGHT, TabBar, TabBarLayout, TabTarget, Terminal, bracketed, encode_key, is_openable,
    mouse_report, next_font_size, reports_mouse, shortcut, wants_scroll_as_arrows,
};
use unbound_toolkit::ui::{
    Cursor, Event, Key, Modifiers, Pointer, PointerButton, Scroll, ScrollPhase, ScrollSource,
};
use unbound_toolkit::{App, Point, Rect, Scene, Window, WindowEvent};

pub const APP_ID: &str = "org.unbound.Terminal";
const WINDOW_NODE: NodeId = NodeId(u64::MAX);
const TERMINAL_NODE: NodeId = NodeId(u64::MAX - 1);
const WRITE_RETRY: Duration = Duration::from_millis(5);
const FRAME: Duration = Duration::from_millis(8);
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const DECELERATION_PER_MILLISECOND: f32 = 0.998;
const MIN_MOMENTUM: f32 = 30.0;

struct Momentum {
    velocity: f32,
    last: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Idle,
    Selecting,
    Reporting(MouseButton),
    Tab(TabTarget),
    Banner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    CloseTab(u32),
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
    reported_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
    title: String,
    size: (u32, u32),
    banner: Option<Banner>,
    banner_focused: bool,
    pending: Option<Pending>,
    last_query: String,
    link: Option<Link>,
    samples: VecDeque<(Instant, f32)>,
    momentum: Option<Momentum>,
    retry_at: Option<Instant>,
    cursor: Cursor,
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
            reported_cell: None,
            scroll_remainder: 0.0,
            title: String::new(),
            size: window.size(),
            banner: None,
            banner_focused: false,
            pending: None,
            last_query: String::new(),
            link: None,
            samples: VecDeque::new(),
            momentum: None,
            retry_at: None,
            cursor: Cursor::Default,
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

    fn tab_bar_height(&self) -> f32 {
        if self.shows_tab_bar() {
            TAB_BAR_HEIGHT
        } else {
            0.0
        }
    }

    fn content_origin(&self) -> Point {
        let banner = if self.banner.is_some() {
            BANNER_HEIGHT
        } else {
            0.0
        };
        Point::new(0.0, self.tab_bar_height() + banner)
    }

    fn grid(&self) -> GridSize {
        let origin = self.content_origin();
        self.painter
            .metrics
            .grid_for(self.size.0 as f32 - origin.x, self.size.1 as f32 - origin.y)
    }

    fn layout(&mut self) {
        let top = self.tab_bar_height();
        if let Some(banner) = &mut self.banner {
            banner.set_top(top);
        }
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
        if self.pending.is_some() {
            self.close_banner();
        }
        self.focus_terminal(false);
        self.session().terminal.end_find();
        self.active = index;
        self.gesture = Gesture::Idle;
        self.run_find();
        self.focus_terminal(self.focused && !self.banner_focused);
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

    fn request_close_tab(&mut self, window: &mut Window, index: usize) {
        let Some(program) = self.tabs.get(index).and_then(Session::running_program) else {
            return self.close_tab(window, index);
        };
        let message = L10N.tr_args("confirm-close-tab", [("program", quoted(program).into())]);
        let pending = Pending::CloseTab(self.tabs[index].id());
        self.select_tab(window, index);
        self.ask(window, &message, pending);
    }

    fn ask(&mut self, window: &mut Window, message: &str, pending: Pending) {
        self.close_banner();
        let palette = window.appearance().palette();
        self.banner = Some(Banner::confirm(palette, message, &L10N.tr("close")));
        self.pending = Some(pending);
        self.banner_focused = true;
        self.focus_terminal(false);
        self.layout();
        window.request_redraw();
    }

    fn confirm(&mut self, window: &mut Window) {
        let pending = self.pending.take();
        self.close_banner();
        match pending {
            Some(Pending::CloseTab(id)) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id() == id) {
                    self.close_tab(window, index);
                }
            }
            None => {}
        }
    }

    fn close_tab(&mut self, window: &mut Window, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let was_active = index == self.active;
        let closed = self.tabs.remove(index);
        if self.tabs.is_empty() {
            return;
        }
        if index < self.active || self.active >= self.tabs.len() {
            self.active -= 1;
        }
        if self.pending == Some(Pending::CloseTab(closed.id())) {
            self.close_banner();
        }
        if was_active {
            self.run_find();
            self.focus_terminal(self.focused && !self.banner_focused);
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
        self.momentum = None;
        if self.link.take().is_some() {
            window.request_redraw();
        }
        let command = shortcut(key, modifiers);
        let edits_find = self.banner_focused && command == Some(Command::SelectAll);
        if let Some(command) = command.filter(|_| !edits_find) {
            return self.perform(window, command);
        }
        if self.banner_focused
            && let Some(banner) = &mut self.banner
        {
            banner.handle(&Event::Key { key, modifiers });
            return window.request_redraw();
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
            Command::Find => self.open_find(window),
            Command::FindNext => self.find_again(window, SearchDirection::Newer),
            Command::FindPrevious => self.find_again(window, SearchDirection::Older),
        }
        window.request_redraw();
    }

    fn find_banner(&mut self) -> Option<&mut Banner> {
        self.banner.as_mut().filter(|banner| banner.is_find())
    }

    fn open_find(&mut self, window: &Window) {
        match self.find_banner() {
            Some(banner) => banner.take_focus(),
            None => {
                self.close_banner();
                let palette = window.appearance().palette();
                self.banner = Some(Banner::find(palette, &self.last_query));
                self.layout();
                self.run_find();
            }
        }
        self.banner_focused = true;
        self.focus_terminal(false);
    }

    fn find_again(&mut self, window: &Window, direction: SearchDirection) {
        if self.find_banner().is_none() {
            return self.open_find(window);
        }
        let found = self.session().terminal.find_again(direction);
        if let Some(banner) = self.find_banner() {
            banner.set_found(found);
        }
    }

    fn run_find(&mut self) {
        let Some(query) = self.find_banner().map(|banner| banner.query()) else {
            return;
        };
        let found = self.session().terminal.find(&query);
        if let Some(banner) = self.find_banner() {
            banner.set_found(found);
        }
        self.last_query = query;
    }

    fn close_banner(&mut self) {
        if self.banner.take().is_none() {
            return;
        }
        self.session().terminal.end_find();
        self.pending = None;
        self.banner_focused = false;
        self.focus_terminal(self.focused);
        self.layout();
    }

    fn handle_banner_actions(&mut self, window: &mut Window) {
        let Some(actions) = self.banner.as_mut().map(Banner::take_actions) else {
            return;
        };
        for action in actions {
            match action {
                BannerAction::QueryChanged => self.run_find(),
                BannerAction::Older => self.find_again(window, SearchDirection::Older),
                BannerAction::Newer => self.find_again(window, SearchDirection::Newer),
                BannerAction::Dismiss => self.close_banner(),
                BannerAction::Confirm => self.confirm(window),
            }
            if self.tabs.is_empty() {
                return;
            }
        }
        window.request_redraw();
    }

    fn focus_terminal(&mut self, focused: bool) {
        if let Some(report) = self.session().terminal.set_focus(focused) {
            self.send(report);
        }
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

    fn pointer_moved(&mut self, window: &mut Window, pointer: Pointer) {
        let at = pointer.position;
        self.pointer = at;
        let modifiers = pointer.modifiers;
        let hover = self.tab_target(at);
        if hover != self.hover {
            self.hover = hover;
            window.request_redraw();
        }
        self.hover_link(window, modifiers);
        match self.gesture {
            Gesture::Reporting(button) => {
                self.report_mouse(MouseKind::Drag(button), at, modifiers);
            }
            Gesture::Selecting => {
                let cell = self.cell_at(at);
                self.session().terminal.extend_selection(cell);
                window.request_redraw();
            }
            Gesture::Banner => {
                if let Some(banner) = &mut self.banner {
                    banner.handle(&Event::PointerMove(pointer));
                    window.request_redraw();
                }
            }
            Gesture::Tab(_) => {}
            Gesture::Idle if hover.is_none() && !self.in_banner(at) => {
                self.report_mouse(MouseKind::Move, at, modifiers);
            }
            Gesture::Idle => {
                if let Some(banner) = &mut self.banner {
                    banner.handle(&Event::PointerMove(pointer));
                }
            }
        }
        self.update_cursor(window, modifiers);
    }

    fn update_cursor(&mut self, window: &mut Window, modifiers: Modifiers) {
        let at = self.pointer;
        let cursor = if self.link.is_some() {
            Cursor::Pointer
        } else if self.in_banner(at) {
            self.banner.as_ref().map_or(Cursor::Default, Banner::cursor)
        } else if !self.in_content(at) || (reports_mouse(self.mode()) && !modifiers.shift) {
            Cursor::Default
        } else {
            Cursor::Text
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            window.set_cursor(cursor);
        }
    }

    fn in_content(&self, at: Point) -> bool {
        at.y >= self.content_origin().y
    }

    fn hover_link(&mut self, window: &mut Window, modifiers: Modifiers) {
        let at = self.pointer;
        let link = if modifiers.logo && self.gesture == Gesture::Idle && self.in_content(at) {
            let cell = self.cell_at(at);
            self.session().terminal.link_at(cell)
        } else {
            None
        };
        if link != self.link {
            self.link = link;
            window.request_redraw();
        }
    }

    fn open_link(&mut self, window: &mut Window, uri: &str) {
        if !is_openable(uri) {
            return tracing::warn!("not opening a link of an unknown kind: {uri}");
        }
        let token = window
            .activation_token(APP_ID)
            .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
            .ok();
        let command = ["xdg-open".to_owned(), uri.to_owned()];
        if let Err(error) = unbound_xdg::spawn_detached(&command, None, token.as_deref()) {
            tracing::warn!("cannot open {uri}: {error}");
        }
    }

    fn release(&mut self, window: &mut Window, pointer: Pointer) {
        let at = pointer.position;
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Reporting(button) if mouse_button(pointer.button) == Some(button) => {
                self.report_mouse(MouseKind::Release(button), at, pointer.modifiers);
            }
            Gesture::Selecting if pointer.is_primary() => {
                self.session().terminal.finish_selection();
                window.request_redraw();
            }
            Gesture::Tab(target) if self.tab_target(at) == Some(target) => match target {
                TabTarget::Close(index) => self.request_close_tab(window, index),
                TabTarget::NewTab => self.new_tab(window),
                TabTarget::Select(_) => {}
            },
            Gesture::Banner => {
                if let Some(banner) = &mut self.banner {
                    banner.handle(&Event::PointerUp(pointer));
                    window.request_redraw();
                }
            }
            gesture @ (Gesture::Reporting(_) | Gesture::Selecting) => self.gesture = gesture,
            Gesture::Tab(_) | Gesture::Idle => {}
        }
    }

    fn press(&mut self, window: &mut Window, pointer: Pointer) {
        let at = pointer.position;
        self.momentum = None;
        if let Some(target) = self.tab_target(at) {
            if !pointer.is_primary() {
                return;
            }
            if let TabTarget::Select(index) = target {
                self.select_tab(window, index);
            }
            self.gesture = Gesture::Tab(target);
            return;
        }
        if self.in_banner(at) {
            if let Some(banner) = &mut self.banner {
                banner.handle(&Event::PointerDown(pointer));
            }
            self.gesture = Gesture::Banner;
            self.banner_focused = true;
            self.focus_terminal(false);
            return window.request_redraw();
        }
        if self.banner_focused {
            self.banner_focused = false;
            self.focus_terminal(self.focused);
        }
        let modifiers = pointer.modifiers;
        if modifiers.logo && pointer.is_primary() {
            let cell = self.cell_at(at);
            if let Some(link) = self.session().terminal.link_at(cell) {
                self.open_link(window, &link.uri);
            }
            return;
        }
        let Some(button) = mouse_button(pointer.button) else {
            return;
        };
        if !modifiers.shift && reports_mouse(self.mode()) {
            self.session().terminal.clear_selection();
            self.reported_cell = None;
            if self.report_mouse(MouseKind::Press(button), at, modifiers) {
                self.gesture = Gesture::Reporting(button);
            }
            window.request_redraw();
            return;
        }
        if !pointer.is_primary() {
            return;
        }
        let cell = self.cell_at(at);
        self.gesture = Gesture::Selecting;
        let terminal = &mut self.session().terminal;
        if modifiers.shift && terminal.has_selection() {
            terminal.extend_selection(cell);
        } else {
            let kind = SelectionKind::for_clicks(pointer.count, modifiers.alt);
            terminal.start_selection(cell, kind);
        }
        window.request_redraw();
    }

    fn in_banner(&self, at: Point) -> bool {
        self.banner
            .as_ref()
            .is_some_and(|banner| banner.contains(at))
    }

    fn scroll_pages(&mut self, window: &mut Window, pages: i32) {
        self.session().terminal.scroll_pages(pages);
        window.request_redraw();
    }

    fn scroll(&mut self, window: &mut Window, scroll: Scroll) {
        self.pointer = scroll.position;
        let dy = scroll.delta.y;
        let program_scrolls = (!scroll.modifiers.shift && reports_mouse(self.mode()))
            || wants_scroll_as_arrows(self.mode());
        let smooth = scroll.source != ScrollSource::Wheel && !program_scrolls;
        match scroll.phase {
            ScrollPhase::Began => {
                self.momentum = None;
                self.samples.clear();
            }
            ScrollPhase::Changed => {}
            ScrollPhase::Ended => {
                if smooth {
                    self.start_momentum();
                }
                return;
            }
        }
        if smooth {
            let now = Instant::now();
            self.samples.push_back((now, dy));
            while self
                .samples
                .front()
                .is_some_and(|(when, _)| now.duration_since(*when) > VELOCITY_WINDOW)
            {
                self.samples.pop_front();
            }
            self.scroll_smoothly(window, dy);
        } else {
            self.scroll_by_lines(window, dy, scroll.modifiers);
        }
    }

    fn scroll_smoothly(&mut self, window: &mut Window, dy: f32) {
        let line_height = self.painter.metrics.cell_height;
        self.session().terminal.scroll_smoothly(-dy, line_height);
        window.request_redraw();
    }

    fn start_momentum(&mut self) {
        let (Some((first, _)), Some((last, _))) = (self.samples.front(), self.samples.back())
        else {
            return;
        };
        let span = last
            .duration_since(*first)
            .as_secs_f32()
            .max(FRAME.as_secs_f32());
        let distance: f32 = self.samples.iter().map(|(_, delta)| delta).sum();
        let velocity = distance / span;
        self.samples.clear();
        if velocity.abs() >= MIN_MOMENTUM {
            self.momentum = Some(Momentum {
                velocity,
                last: Instant::now(),
            });
        }
    }

    fn glide(&mut self, window: &mut Window, now: Instant) {
        let Some(momentum) = &mut self.momentum else {
            return;
        };
        let elapsed = now.duration_since(momentum.last);
        momentum.last = now;
        let distance = momentum.velocity * elapsed.as_secs_f32();
        momentum.velocity *= DECELERATION_PER_MILLISECOND.powf(elapsed.as_secs_f32() * 1000.0);
        if momentum.velocity.abs() < MIN_MOMENTUM {
            self.momentum = None;
        }
        self.scroll_smoothly(window, distance);
    }

    fn scroll_by_lines(&mut self, window: &mut Window, dy: f32, modifiers: Modifiers) {
        self.scroll_remainder += dy / self.painter.metrics.cell_height;
        let lines = self.scroll_remainder.trunc();
        self.scroll_remainder -= lines;
        let lines = lines as i32;
        if lines == 0 {
            return;
        }
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

    fn schedule_retry(&mut self) {
        let pending = self.tabs.iter().any(Session::has_pending_output);
        self.retry_at = match (pending, self.retry_at) {
            (false, _) => None,
            (true, Some(at)) => Some(at),
            (true, None) => Some(Instant::now() + WRITE_RETRY),
        };
    }

    fn focus(&mut self, window: &mut Window, focused: bool) {
        self.focused = focused;
        self.focus_terminal(focused && !self.banner_focused);
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
        let mut highlights = self.session().terminal.highlights();
        highlights.link = self.link.as_ref().map(|link| link.range.clone());
        items.extend(
            self.painter
                .terminal(&self.tabs[self.active].terminal, origin, &highlights),
        );
        if let Some(banner) = &mut self.banner {
            items.extend(banner.items(size.0, size.1));
        }
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
            WindowEvent::Input(Event::PointerDown(pointer)) => self.press(window, pointer),
            WindowEvent::Input(Event::PointerMove(pointer) | Event::PointerEnter(pointer)) => {
                self.pointer_moved(window, pointer)
            }
            WindowEvent::Input(Event::PointerUp(pointer)) => self.release(window, pointer),
            WindowEvent::Input(Event::PointerLeave) => {
                if self.hover.take().is_some() || self.link.take().is_some() {
                    window.request_redraw();
                }
                if let Some(banner) = &mut self.banner {
                    banner.handle(&Event::PointerLeave);
                }
                self.cursor = Cursor::Default;
            }
            WindowEvent::Input(Event::ModifiersChanged(modifiers)) => {
                self.hover_link(window, modifiers);
                self.update_cursor(window, modifiers);
            }
            WindowEvent::Input(Event::Scroll(scroll)) => self.scroll(window, scroll),
            WindowEvent::Pasted(text) if self.banner_focused => {
                if let Some(banner) = &mut self.banner {
                    banner.handle(&Event::Commit(text.replace(['\r', '\n'], " ")));
                }
            }
            WindowEvent::Input(event @ (Event::Commit(_) | Event::Preedit { .. })) => {
                if self.banner_focused
                    && let Some(banner) = &mut self.banner
                {
                    banner.handle(&event);
                    window.request_redraw();
                }
            }
            WindowEvent::Accessibility(request)
                if request.target_node == TERMINAL_NODE && request.action == Action::Focus =>
            {
                self.banner_focused = false;
                self.focus_terminal(self.focused);
                window.request_redraw();
            }
            WindowEvent::Accessibility(request) => {
                if let Some(banner) = &mut self.banner {
                    banner.perform(&request);
                    window.request_redraw();
                }
            }
            WindowEvent::Pasted(text) => {
                let bytes = bracketed(&text, self.mode());
                self.session().terminal.scroll_to_bottom();
                self.send(&bytes);
            }
            WindowEvent::Focus(focused) => self.focus(window, focused),
            WindowEvent::AppearanceChanged(appearance) => {
                self.painter.set_appearance(appearance);
                if let Some(banner) = &mut self.banner {
                    banner.set_palette(appearance.palette());
                }
            }
            _ => {}
        }
        self.handle_banner_actions(window);
        if self.tabs.is_empty() {
            return window.close();
        }
        let notices = self.session().take_notices();
        self.handle_notices(window, notices);
        self.remove_exited(window);
        if !self.tabs.is_empty() {
            self.sync_title(window);
        }
        self.schedule_retry();
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
        self.schedule_retry();
    }

    fn deadline(&self) -> Option<Instant> {
        let glide = self.momentum.as_ref().map(|momentum| momentum.last + FRAME);
        [glide, self.retry_at].into_iter().flatten().min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.retry_at.is_some_and(|at| at <= now) {
            self.retry_at = None;
            for tab in &mut self.tabs {
                tab.flush();
            }
            self.schedule_retry();
        }
        if self
            .momentum
            .as_ref()
            .is_some_and(|momentum| momentum.last + FRAME <= now)
        {
            self.glide(window, now);
        }
    }

    fn should_close(&mut self, window: &mut Window) -> bool {
        if self.tabs.len() == 1 && self.tabs[0].running_program().is_none() {
            return true;
        }
        self.request_close_tab(window, self.active);
        false
    }

    fn finished(&self) -> bool {
        self.tabs.is_empty()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.banner
            .as_ref()
            .filter(|_| self.banner_focused)
            .and_then(Banner::text_input_area)
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        let mut terminal = Node::new(Role::Terminal);
        terminal.set_label(self.title.clone());
        terminal.set_value(self.terminal().screen_text().join("\n"));
        terminal.add_action(Action::Focus);
        let mut children = vec![TERMINAL_NODE];
        let mut nodes = Vec::new();
        let mut focus = TERMINAL_NODE;
        if let Some(banner) = &self.banner {
            let tree = banner.accessibility();
            children.extend(tree.tree.map(|info| info.root));
            if self.banner_focused {
                focus = tree.focus;
            }
            nodes.extend(tree.nodes);
        }
        let mut root = Node::new(Role::Window);
        root.set_label(self.title.clone());
        root.set_children(children);
        nodes.splice(0..0, [(WINDOW_NODE, root), (TERMINAL_NODE, terminal)]);
        Some(TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(WINDOW_NODE)),
            tree_id: TreeId::ROOT,
            focus,
        })
    }
}

fn mouse_button(button: PointerButton) -> Option<MouseButton> {
    match button {
        PointerButton::Primary => Some(MouseButton::Left),
        PointerButton::Secondary => Some(MouseButton::Right),
        PointerButton::Middle => Some(MouseButton::Middle),
        PointerButton::Back | PointerButton::Forward => None,
    }
}

fn quoted(name: &str) -> String {
    format!("\u{201C}{name}\u{201D}")
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
