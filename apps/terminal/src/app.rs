use std::collections::VecDeque;
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::{Action, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::Shell;
use unbound_app_menu::{AppMenu, Standard};
use unbound_spring::{Decay, State as SpringState};
use unbound_term::{
    Axis, BANNER_HEIGHT, Banner, BannerAction, CellPoint, Command, Divider, FULL_SCREEN_LEADING,
    GridSize, HEADER_LEADING, L10N, Layout, Link, MenuCommand, MouseButton, MouseKind, Notice,
    Painter, Pane, STRIP_HEIGHT, SavedWindow, SearchDirection, SelectionKind, Session, Settings,
    TabBar, TabBarLayout, TabTarget, Terminal, TerminalContext, bracketed, dropped_text,
    encode_key, is_openable, is_settings_file, mouse_report, next_font_size, reports_mouse,
    settings_folder, shortcut, tab_menu, terminal_menu, wants_scroll_as_arrows,
};
use unbound_toolkit::ui::{
    Cursor, Event, Key, Menu, Modifiers, Pointer, PointerButton, Scroll, ScrollPhase, ScrollSource,
};
use unbound_toolkit::{
    App, DragAction, DragEvent, FrameRegions, Header, Item, MenuPlacement, Point, Radius, Rect,
    SHEET_INSET, Scene, Shape, Window, WindowEvent,
};
use unbound_watch::{Changes, FolderWatch};

use crate::bus::{Bus, Outgoing};

pub const APP_ID: &str = "org.unbound.Terminal";
const FILES: &str = "unbound-files";
const WINDOW_NODE: NodeId = NodeId(u64::MAX);
const TERMINAL_NODE: NodeId = NodeId(u64::MAX - 1);
const TAB_LIST_NODE: NodeId = NodeId(u64::MAX - 2);
const NEW_TAB_NODE: NodeId = NodeId(u64::MAX - 3);
const TAB_NODES: u64 = u64::MAX - 1024;
const WRITE_RETRY: Duration = Duration::from_millis(5);
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const MIN_MOMENTUM: f64 = 30.0;
const LONG_COMMAND: Duration = Duration::from_secs(10);
const FLASH: Duration = Duration::from_millis(220);
const FLASH_OPACITY: f32 = 0.14;
const MIN_PANE: f32 = 64.0;

struct Momentum {
    velocity: f64,
    since: Instant,
    travelled: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    Idle,
    Selecting,
    Reporting(MouseButton),
    Tab(TabTarget),
    Banner,
    Divider(Divider),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    CloseTab(u32),
    ClosePane(u32),
    CloseOtherTabs(u32),
    Quit,
}

struct Tab {
    id: u32,
    sessions: Vec<Session>,
    layout: Layout,
    focused: u32,
    bell: bool,
}

impl Tab {
    fn new(session: Session) -> Self {
        let id = session.id();
        Self {
            id,
            sessions: vec![session],
            layout: Layout::Pane(id),
            focused: id,
            bell: false,
        }
    }

    fn focused_index(&self) -> usize {
        self.sessions
            .iter()
            .position(|session| session.id() == self.focused)
            .unwrap_or_default()
    }

    fn focused(&self) -> &Session {
        &self.sessions[self.focused_index()]
    }

    fn focused_mut(&mut self) -> &mut Session {
        let index = self.focused_index();
        &mut self.sessions[index]
    }

    fn session(&self, id: u32) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id() == id)
    }

    fn running_program(&self) -> Option<&str> {
        self.sessions.iter().find_map(Session::running_program)
    }
}

struct SettingsWatch {
    _watch: FolderWatch,
    wake: UnixStream,
}

impl SettingsWatch {
    fn start(folder: &Path) -> Option<Self> {
        let started = std::fs::create_dir_all(folder).and_then(|()| {
            let (wake, mut signal) = UnixStream::pair()?;
            wake.set_nonblocking(true)?;
            let watch = FolderWatch::spawn(Changes::Saved, move |paths| {
                if paths.iter().any(|path| is_settings_file(path)) {
                    std::io::Write::write_all(&mut signal, &[1]).ok();
                }
            })?;
            watch.watch(folder)?;
            Ok(Self {
                _watch: watch,
                wake,
            })
        });
        started
            .inspect_err(|error| tracing::warn!("cannot follow the settings: {error}"))
            .ok()
    }

    fn drain(&mut self) {
        let mut buffer = [0u8; 64];
        while matches!(self.wake.read(&mut buffer), Ok(read) if read > 0) {}
    }
}

pub struct TerminalApp {
    tabs: Vec<Tab>,
    active: usize,
    painter: Painter,
    settings: Settings,
    settings_watch: Option<SettingsWatch>,
    home: Option<PathBuf>,
    focused: bool,
    pointer: Point,
    gesture: Gesture,
    hover: Option<TabTarget>,
    reported_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
    title: String,
    size: (u32, u32),
    header: f32,
    banner: Option<Banner>,
    banner_focused: bool,
    droppable: bool,
    pending: Option<Pending>,
    last_query: String,
    link: Option<Link>,
    samples: VecDeque<(Duration, f32)>,
    momentum: Option<Momentum>,
    retry_at: Option<Instant>,
    flash: Option<Instant>,
    badge: u32,
    cursor: Cursor,
    app_menu: Option<AppMenu>,
    bus: Bus,
    saved: Option<(PathBuf, SavedWindow)>,
}

impl TerminalApp {
    pub fn new(
        window: &mut Window,
        painter: Painter,
        settings: Settings,
        (sessions, active): (Vec<Session>, usize),
        home: Option<PathBuf>,
        saved_file: Option<PathBuf>,
        app_menu: Option<AppMenu>,
    ) -> Self {
        let settings_watch = settings_folder().and_then(|folder| SettingsWatch::start(&folder));
        let mut app = Self {
            active: active.min(sessions.len().saturating_sub(1)),
            tabs: sessions.into_iter().map(Tab::new).collect(),
            painter,
            settings,
            settings_watch,
            home,
            focused: false,
            pointer: Point::default(),
            gesture: Gesture::Idle,
            hover: None,
            reported_cell: None,
            scroll_remainder: 0.0,
            title: String::new(),
            size: window.size(),
            header: window.header_height() as f32,
            banner: None,
            banner_focused: false,
            droppable: false,
            pending: None,
            last_query: String::new(),
            link: None,
            samples: VecDeque::new(),
            momentum: None,
            retry_at: None,
            flash: None,
            badge: 0,
            cursor: Cursor::Default,
            app_menu,
            bus: Bus::start(APP_ID),
            saved: saved_file.map(|file| (file, SavedWindow::default())),
        };
        for tab in &mut app.tabs {
            for session in &mut tab.sessions {
                session.terminal.set_focus(false);
            }
        }
        app.frame_window(window);
        app.size = window.size();
        app.frame_window(window);
        app.layout();
        app
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn session(&mut self) -> &mut Session {
        self.tabs[self.active].focused_mut()
    }

    fn terminal(&self) -> &Terminal {
        &self.tab().focused().terminal
    }

    fn mode(&self) -> TermMode {
        self.terminal().mode()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.session().send(bytes);
    }

    fn has_strip(&self) -> bool {
        self.header > 0.0 || self.tabs.len() > 1
    }

    fn leading(&self) -> f32 {
        if self.header > 0.0 {
            HEADER_LEADING
        } else {
            FULL_SCREEN_LEADING
        }
    }

    fn tab_layout(&self) -> TabBarLayout {
        TabBarLayout::new(self.tabs.len(), self.size.0 as f32, self.leading())
    }

    fn frame_window(&self, window: &mut Window) {
        let toolbar = if self.header == 0.0 && self.tabs.len() > 1 {
            STRIP_HEIGHT
        } else {
            0.0
        };
        window.set_frame_regions(FrameRegions {
            sidebar: 0.0,
            toolbar,
        });
        let layout = self.tab_layout();
        let title_x = if self.tabs.len() > 1 {
            self.size.0 as f32
        } else {
            self.leading()
        };
        let (items, tabs) = if self.header > 0.0 {
            (vec![layout.new_tab], layout.tabs)
        } else {
            (Vec::new(), Vec::new())
        };
        window.set_header(Header {
            title_x,
            items,
            tabs,
        });
    }

    fn sheet(&self) -> Rect {
        let inset = if self.header > 0.0 {
            SHEET_INSET as f32
        } else {
            0.0
        };
        let top = if self.has_strip() { STRIP_HEIGHT } else { 0.0 };
        let (width, height) = (self.size.0 as f32, self.size.1 as f32);
        Rect::new(
            inset,
            top,
            (width - 2.0 * inset).max(0.0),
            (height - top - inset).max(0.0),
        )
    }

    fn pane_area(&self) -> Rect {
        let sheet = self.sheet();
        let banner = if self.banner.is_some() {
            BANNER_HEIGHT
        } else {
            0.0
        };
        Rect::new(
            sheet.x,
            sheet.y + banner,
            sheet.width,
            (sheet.height - banner).max(0.0),
        )
    }

    fn frames(&self) -> Vec<(u32, Rect)> {
        self.tab().layout.frames(self.pane_area())
    }

    fn pane_frame(&self, id: u32) -> Rect {
        self.frames()
            .into_iter()
            .find(|(pane, _)| *pane == id)
            .map_or_else(|| self.pane_area(), |(_, frame)| frame)
    }

    fn focused_frame(&self) -> Rect {
        self.pane_frame(self.tab().focused)
    }

    fn pane_at(&self, at: Point) -> Option<u32> {
        self.frames()
            .into_iter()
            .find(|(_, frame)| contains(*frame, at))
            .map(|(id, _)| id)
    }

    fn divider_at(&self, at: Point) -> Option<Divider> {
        self.tab()
            .layout
            .dividers(self.pane_area())
            .into_iter()
            .find(|divider| contains(divider.reach(), at))
    }

    fn layout(&mut self) {
        let sheet = self.sheet();
        if let Some(banner) = &mut self.banner {
            banner.set_area(Rect::new(sheet.x, sheet.y, sheet.width, BANNER_HEIGHT));
        }
        let area = self.pane_area();
        let metrics = self.painter.metrics;
        let cell_size = metrics.pixel_cell_size();
        let shape = self.settings.cursor.shape();
        for tab in &mut self.tabs {
            for (id, frame) in tab.layout.frames(area) {
                let grid = metrics.grid_for(frame.width, frame.height);
                if let Some(session) = tab.sessions.iter_mut().find(|session| session.id() == id) {
                    session.resize(grid, cell_size);
                    session.terminal.set_cursor_shape(shape);
                }
            }
        }
    }

    fn cell_at(&self, at: Point) -> CellPoint {
        let frame = self.focused_frame();
        self.painter
            .metrics
            .cell_at(Point::new(at.x - frame.x, at.y - frame.y))
    }

    fn tab_target(&self, at: Point) -> Option<TabTarget> {
        if !self.has_strip() || at.y >= STRIP_HEIGHT {
            return None;
        }
        self.tab_layout().hit(at)
    }

    fn tab_title(&self, tab: &Tab) -> String {
        let title = tab.focused().title(self.home.as_deref());
        if title.is_empty() {
            L10N.tr("title")
        } else {
            title
        }
    }

    fn titles(&self) -> Vec<String> {
        self.tabs.iter().map(|tab| self.tab_title(tab)).collect()
    }

    fn sync_title(&mut self, window: &mut Window) {
        let title = self.tab_title(self.tab());
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
        self.save_window();
    }

    fn save_window(&mut self) {
        let Some((file, last)) = &mut self.saved else {
            return;
        };
        let home = self.home.clone();
        let current = SavedWindow {
            tabs: self
                .tabs
                .iter()
                .filter_map(|tab| tab.focused().working_directory().or_else(|| home.clone()))
                .collect(),
            active: self.active,
        };
        if current != *last {
            match current.save(file) {
                Ok(()) => *last = current,
                Err(error) => tracing::warn!("cannot remember the window: {error}"),
            }
        }
    }

    fn forget_window(&mut self) {
        if let Some((file, _)) = self.saved.take()
            && let Err(error) = std::fs::remove_file(&file)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot forget the window: {error}");
        }
    }

    fn handle_notices(&mut self, window: &mut Window, tab: usize, notices: Vec<Notice>) {
        let background = !self.focused || tab != self.active;
        for notice in notices {
            match notice {
                Notice::Copy(text) => window.copy(&text),
                Notice::Bell => self.bell(window, tab),
                Notice::Finished {
                    command,
                    elapsed,
                    code,
                } if background
                    && self.settings.notify_finished
                    && elapsed >= LONG_COMMAND
                    && !command.is_empty() =>
                {
                    let summary = match code {
                        Some(code) if code != 0 => L10N.tr_args(
                            "command-failed",
                            [("command", command.into()), ("code", code.into())],
                        ),
                        _ => L10N.tr_args("command-finished", [("command", command.into())]),
                    };
                    let body = self
                        .tabs
                        .get(tab)
                        .map(|tab| self.tab_title(tab))
                        .unwrap_or_default();
                    self.bus.send(Outgoing::Notify { summary, body });
                }
                Notice::Finished { .. } => {}
                Notice::Notify { title, body } if background => {
                    self.bus.send(Outgoing::Notify {
                        summary: title,
                        body,
                    });
                }
                Notice::Notify { .. } => {}
            }
        }
    }

    fn bell(&mut self, window: &mut Window, tab: usize) {
        if tab != self.active
            && let Some(tab) = self.tabs.get_mut(tab)
        {
            tab.bell = true;
            window.request_redraw();
        }
        if !self.focused && self.settings.badge_on_bell {
            self.badge += 1;
            self.bus.send(Outgoing::Badge(self.badge));
        }
        if self.settings.flash_on_bell && tab == self.active {
            self.flash = Some(Instant::now());
            window.request_redraw();
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
        self.tabs[index].bell = false;
        self.gesture = Gesture::Idle;
        self.run_find();
        self.focus_terminal(self.focused && !self.banner_focused);
        window.request_redraw();
    }

    fn spawn_session(&mut self, size: GridSize) -> Option<Session> {
        let directory = self
            .session()
            .working_directory()
            .or_else(|| self.home.clone());
        let mut terminal = Terminal::new(size);
        terminal.resize(size, self.painter.metrics.pixel_cell_size());
        Session::spawn(terminal, None, directory)
            .inspect_err(|error| tracing::warn!("cannot start a shell: {error}"))
            .ok()
    }

    fn new_tab(&mut self, window: &mut Window) {
        let Some(session) = self.spawn_session(self.terminal().size()) else {
            return;
        };
        self.tabs.push(Tab::new(session));
        self.after_tabs_changed(window);
        self.select_tab(window, self.tabs.len() - 1);
    }

    fn after_tabs_changed(&mut self, window: &mut Window) {
        self.frame_window(window);
        self.size = window.size();
        self.layout();
        window.request_redraw();
    }

    fn split(&mut self, window: &mut Window, axis: Axis) {
        let frame = self.focused_frame();
        let small = match axis {
            Axis::SideBySide => frame.width / 2.0 < MIN_PANE,
            Axis::Stacked => frame.height / 2.0 < MIN_PANE,
        };
        if small {
            return;
        }
        let Some(session) = self.spawn_session(self.terminal().size()) else {
            return;
        };
        let id = session.id();
        let tab = &mut self.tabs[self.active];
        let focused = tab.focused;
        tab.sessions.push(session);
        tab.layout.split(focused, axis, id);
        self.layout();
        self.focus_pane(window, id);
    }

    fn focus_pane(&mut self, window: &mut Window, id: u32) {
        if self.tab().focused == id || self.tab().session(id).is_none() {
            return;
        }
        if self.find_banner().is_some() {
            self.session().terminal.end_find();
        }
        self.focus_terminal(false);
        self.tabs[self.active].focused = id;
        self.run_find();
        self.focus_terminal(self.focused && !self.banner_focused);
        self.sync_title(window);
        window.request_redraw();
    }

    fn move_focus(&mut self, window: &mut Window, forward: bool) {
        let tab = self.tab();
        let next = tab.layout.neighbour(tab.focused, forward);
        self.focus_pane(window, next);
    }

    fn new_window(&mut self, window: &mut Window) {
        let directory = self
            .session()
            .working_directory()
            .or_else(|| self.home.clone());
        let mut command = vec![];
        if let Some(directory) = directory {
            command.push(DIRECTORY_OPTION.to_owned());
            command.push(directory.to_string_lossy().into_owned());
        }
        spawn_self(window, command);
    }

    fn request_close_tab(&mut self, window: &mut Window, index: usize) {
        let Some(program) = self.tabs.get(index).and_then(Tab::running_program) else {
            return self.close_tab(window, index);
        };
        let message = L10N.tr_args("confirm-close-tab", [("program", quoted(program).into())]);
        let pending = Pending::CloseTab(self.tabs[index].id);
        self.select_tab(window, index);
        self.ask(window, &message, pending);
    }

    fn request_close(&mut self, window: &mut Window) {
        if self.tab().sessions.len() == 1 {
            return self.request_close_tab(window, self.active);
        }
        let id = self.tab().focused;
        match self.tab().focused().running_program() {
            Some(program) => {
                let message =
                    L10N.tr_args("confirm-close-pane", [("program", quoted(program).into())]);
                self.ask(window, &message, Pending::ClosePane(id));
            }
            None => self.close_pane(window, self.active, id),
        }
    }

    fn ask(&mut self, window: &mut Window, message: &str, pending: Pending) {
        self.close_banner();
        let palette = window.appearance().palette();
        let button = match pending {
            Pending::Quit => L10N.tr("quit"),
            Pending::CloseTab(_) | Pending::ClosePane(_) | Pending::CloseOtherTabs(_) => {
                L10N.tr("close")
            }
        };
        let text = self.painter.text_system().sharing_fonts();
        self.banner = Some(Banner::confirm(palette, text, message, &button));
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
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.close_tab(window, index);
                }
            }
            Some(Pending::ClosePane(id)) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.session(id).is_some()) {
                    self.close_pane(window, index, id);
                }
            }
            Some(Pending::CloseOtherTabs(keep)) => self.remove_other_tabs(window, keep),
            Some(Pending::Quit) => window.close(),
            None => {}
        }
    }

    fn close_other_tabs(&mut self, window: &mut Window, keep: u32) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == keep) else {
            return;
        };
        let busy = self
            .tabs
            .iter()
            .filter(|tab| tab.id != keep && tab.running_program().is_some())
            .count();
        if busy == 0 {
            return self.remove_other_tabs(window, keep);
        }
        self.select_tab(window, index);
        let message = L10N.tr_args("confirm-close-other-tabs", [("count", busy.into())]);
        self.ask(window, &message, Pending::CloseOtherTabs(keep));
    }

    fn may_quit(&mut self, window: &mut Window) -> bool {
        let busy = self
            .tabs
            .iter()
            .filter(|tab| tab.running_program().is_some())
            .count();
        if busy > 0 {
            let message = L10N.tr_args("confirm-quit", [("count", busy.into())]);
            self.ask(window, &message, Pending::Quit);
        }
        busy == 0
    }

    fn remove_other_tabs(&mut self, window: &mut Window, keep: u32) {
        if !self.tabs.iter().any(|tab| tab.id == keep) {
            return;
        }
        while let Some(index) = self.tabs.iter().position(|tab| tab.id != keep) {
            self.close_tab(window, index);
        }
    }

    fn close_pane(&mut self, window: &mut Window, tab: usize, id: u32) {
        let Some(entry) = self.tabs.get_mut(tab) else {
            return;
        };
        if entry.sessions.len() == 1 {
            return self.close_tab(window, tab);
        }
        let was_focused = entry.focused == id;
        if was_focused && tab == self.active {
            self.focus_terminal(false);
            self.session().terminal.end_find();
        }
        let entry = &mut self.tabs[tab];
        let next = entry.layout.neighbour(id, false);
        entry.layout.remove(id);
        entry.sessions.retain(|session| session.id() != id);
        if was_focused {
            entry.focused = next;
        }
        if self.pending == Some(Pending::ClosePane(id)) {
            self.close_banner();
        }
        if was_focused && tab == self.active {
            self.run_find();
            self.focus_terminal(self.focused && !self.banner_focused);
        }
        self.gesture = Gesture::Idle;
        self.layout();
        window.request_redraw();
    }

    fn close_tab(&mut self, window: &mut Window, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let was_active = index == self.active;
        let closed = self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.forget_window();
            return;
        }
        if index < self.active || self.active >= self.tabs.len() {
            self.active -= 1;
        }
        let stale = match self.pending {
            Some(Pending::CloseTab(id) | Pending::CloseOtherTabs(id)) => id == closed.id,
            Some(Pending::ClosePane(id)) => closed.session(id).is_some(),
            Some(Pending::Quit) | None => false,
        };
        if stale {
            self.close_banner();
        }
        if was_active {
            self.tabs[self.active].bell = false;
            self.run_find();
            self.focus_terminal(self.focused && !self.banner_focused);
        }
        self.gesture = Gesture::Idle;
        self.hover = None;
        self.after_tabs_changed(window);
    }

    fn remove_exited(&mut self, window: &mut Window) {
        while let Some((tab, id)) = self.tabs.iter().enumerate().find_map(|(index, tab)| {
            tab.sessions
                .iter()
                .find(|session| session.exited())
                .map(|session| (index, session.id()))
        }) {
            self.close_pane(window, tab, id);
        }
    }

    fn key(&mut self, window: &mut Window, key: Key, modifiers: Modifiers) {
        self.momentum = None;
        if self.link.take().is_some() {
            window.request_redraw();
        }
        if opens_menu(key, modifiers) && !self.banner_focused {
            return self.menu_from_keyboard(window);
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
            Command::SelectAll => {
                self.session().terminal.select_all();
                self.offer_primary(window);
            }
            Command::ClearScrollback => self.session().terminal.clear_scrollback(),
            Command::BiggerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), true))
            }
            Command::SmallerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), false))
            }
            Command::DefaultTextSize => self.set_font_size(window, self.settings.font_size),
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
            Command::SplitSideBySide => self.split(window, Axis::SideBySide),
            Command::SplitStacked => self.split(window, Axis::Stacked),
            Command::NextPane => self.move_focus(window, true),
            Command::PreviousPane => self.move_focus(window, false),
            Command::Settings => spawn_self(window, vec![SETTINGS_OPTION.to_owned()]),
        }
        window.request_redraw();
    }

    fn menu_action(&mut self, window: &mut Window, action: &str) {
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit) => {
                if self.may_quit(window) {
                    window.close();
                }
                Ok(())
            }
            Some(Standard::CloseWindow) => {
                self.request_close(window);
                Ok(())
            }
            None => {
                if let Some(command) = MenuCommand::parse(action) {
                    self.perform_menu(window, command);
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
    }

    fn perform_menu(&mut self, window: &mut Window, command: MenuCommand) {
        match command {
            MenuCommand::Run(command) => self.perform(window, command),
            MenuCommand::OpenLink(uri) => self.open_link(window, &uri),
            MenuCommand::CopyLink(uri) => window.copy(&uri),
            MenuCommand::ShowInFiles => self.show_in_files(window),
            MenuCommand::CloseTab(id) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.request_close_tab(window, index);
                }
            }
            MenuCommand::CloseOtherTabs(id) => self.close_other_tabs(window, id),
        }
    }

    fn show_in_files(&mut self, window: &mut Window) {
        let Some(directory) = self.session().working_directory() else {
            return;
        };
        let token = window
            .activation_token(APP_ID)
            .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
            .ok();
        let command = [FILES.to_owned(), directory.to_string_lossy().into_owned()];
        if let Err(error) = unbound_xdg::spawn_detached(&command, None, token.as_deref()) {
            tracing::warn!("cannot show {} in Files: {error}", directory.display());
        }
    }

    fn show_menu(&mut self, window: &mut Window, menu: Menu, at: Point) {
        self.momentum = None;
        if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
            tracing::warn!("cannot show the menu: {error}");
        }
    }

    fn terminal_menu(&mut self, window: &mut Window, at: Point, link: Option<Link>) {
        let context = TerminalContext {
            link: link.map(|link| link.uri),
            has_selection: self.terminal().selected_text().is_some(),
            knows_directory: self.session().working_directory().is_some(),
            tab: self.tab().id,
        };
        self.show_menu(window, terminal_menu(&context), at);
    }

    fn menu_from_keyboard(&mut self, window: &mut Window) {
        let frame = self.focused_frame();
        let below = match self.terminal().cursor_cell() {
            Some((column, line)) => self.painter.metrics.below_cell(column, line),
            None => Point::new(frame.width / 2.0, frame.height / 2.0),
        };
        let at = Point::new(frame.x + below.x, frame.y + below.y);
        self.terminal_menu(window, at, None);
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
                let text = self.painter.text_system().sharing_fonts();
                self.banner = Some(Banner::find(palette, text, &self.last_query));
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

    fn paste(&mut self, text: &str) {
        let bytes = bracketed(text, self.mode());
        self.session().terminal.scroll_to_bottom();
        self.send(&bytes);
    }

    fn offer_primary(&mut self, window: &mut Window) {
        if let Some(text) = self.terminal().selected_text() {
            window.select_primary(&text);
        }
    }

    fn drag(&mut self, window: &mut Window, drag: DragEvent) {
        match drag {
            DragEvent::Entered {
                position,
                uris,
                text,
            } => {
                self.droppable = text || !uris.is_empty();
                self.drag_over(window, position);
            }
            DragEvent::Moved(position) => self.drag_over(window, position),
            DragEvent::Left => self.droppable = false,
            DragEvent::Dropped {
                position, transfer, ..
            } => {
                self.droppable = false;
                let Some(pane) = self.pane_at(position) else {
                    return;
                };
                self.focus_pane(window, pane);
                if self.banner_focused {
                    self.banner_focused = false;
                    self.focus_terminal(self.focused);
                }
                self.paste(&dropped_text(&transfer.uris, &transfer.text));
                window.request_redraw();
            }
            DragEvent::Finished(_) => {}
        }
    }

    fn drag_over(&mut self, window: &mut Window, position: Point) {
        let accepts = self.droppable && self.pane_at(position).is_some();
        window.accept_drag_only(accepts.then_some(DragAction::Copy));
    }

    fn handle_banner_actions(&mut self, window: &mut Window) {
        if let Some(request) = self
            .banner
            .as_mut()
            .and_then(Banner::take_clipboard_request)
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot use the clipboard: {error}");
        }
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

    fn reload_settings(&mut self, window: &mut Window) {
        let Some(folder) = settings_folder() else {
            return;
        };
        let settings = Settings::load(&folder);
        if settings == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, settings);
        self.painter.set_profile(settings.profile);
        if old.font_size != settings.font_size {
            self.painter.set_metrics(settings.font_size, window.scale());
        }
        self.layout();
        window.request_redraw();
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
        match self.gesture.clone() {
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
            Gesture::Divider(divider) => {
                let ratio = divider.ratio_at(at);
                self.tabs[self.active]
                    .layout
                    .set_ratio(&divider.path, ratio);
                self.layout();
                window.request_redraw();
            }
            Gesture::Tab(_) => {}
            Gesture::Idle if hover.is_none() && self.in_content(at) => {
                if self.pane_at(at) == Some(self.tab().focused) {
                    self.report_mouse(MouseKind::Move, at, modifiers);
                }
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
        let divider = match &self.gesture {
            Gesture::Divider(divider) => Some(divider.axis),
            _ => self.divider_at(at).map(|divider| divider.axis),
        };
        let cursor = if let Some(axis) = divider {
            match axis {
                Axis::SideBySide => Cursor::ColumnResize,
                Axis::Stacked => Cursor::RowResize,
            }
        } else if self.link.is_some() {
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
        contains(self.pane_area(), at)
    }

    fn hover_link(&mut self, window: &mut Window, modifiers: Modifiers) {
        let at = self.pointer;
        let over_focused = self.pane_at(at) == Some(self.tab().focused);
        let link = if modifiers.logo && self.gesture == Gesture::Idle && over_focused {
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
                self.offer_primary(window);
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
            Gesture::Divider(_) | Gesture::Tab(_) | Gesture::Idle => {}
        }
        self.update_cursor(window, pointer.modifiers);
    }

    fn press(&mut self, window: &mut Window, pointer: Pointer) {
        let at = pointer.position;
        self.momentum = None;
        if let Some(target) = self.tab_target(at) {
            if asks_for_menu(&pointer) {
                let tab = match target {
                    TabTarget::Select(index) | TabTarget::Close(index) => {
                        self.tabs.get(index).map(|tab| tab.id)
                    }
                    TabTarget::NewTab => None,
                };
                return self.show_menu(window, tab_menu(tab, self.tabs.len()), at);
            }
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
        if pointer.is_primary()
            && let Some(divider) = self.divider_at(at)
        {
            self.gesture = Gesture::Divider(divider);
            return;
        }
        if !self.in_content(at) {
            return;
        }
        if self.banner_focused {
            self.banner_focused = false;
            self.focus_terminal(self.focused);
        }
        if let Some(pane) = self.pane_at(at) {
            self.focus_pane(window, pane);
        }
        let modifiers = pointer.modifiers;
        let program_takes_mouse = !modifiers.shift && reports_mouse(self.mode());
        if asks_for_menu(&pointer) && !program_takes_mouse {
            let cell = self.cell_at(at);
            let link = self.session().terminal.link_at(cell);
            return self.terminal_menu(window, at, link);
        }
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
        if program_takes_mouse {
            self.session().terminal.clear_selection();
            self.reported_cell = None;
            if self.report_mouse(MouseKind::Press(button), at, modifiers) {
                self.gesture = Gesture::Reporting(button);
            }
            window.request_redraw();
            return;
        }
        if pointer.button == PointerButton::Middle {
            if let Err(error) = window.paste_primary() {
                tracing::warn!("cannot paste the selection: {error}");
            }
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
        if let Some(pane) = self.pane_at(scroll.position)
            && pane != self.tab().focused
        {
            return;
        }
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
                    self.forget_samples_before(scroll.time);
                    self.start_momentum(window.frame_time().on_screen());
                    window.request_redraw();
                }
                return;
            }
        }
        if smooth {
            self.samples.push_back((scroll.time, dy));
            self.forget_samples_before(scroll.time);
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

    fn forget_samples_before(&mut self, time: Duration) {
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| time.saturating_sub(*at) > VELOCITY_WINDOW)
        {
            self.samples.pop_front();
        }
    }

    fn start_momentum(&mut self, since: Instant) {
        let samples = std::mem::take(&mut self.samples);
        let (Some((first, _)), Some((last, _))) = (samples.front(), samples.back()) else {
            return;
        };
        let span = last.saturating_sub(*first).as_secs_f64();
        if span <= 0.0 {
            return;
        }
        let distance: f32 = samples.iter().skip(1).map(|(_, delta)| delta).sum();
        let velocity = f64::from(distance) / span;
        if velocity.abs() >= MIN_MOMENTUM {
            self.momentum = Some(Momentum {
                velocity,
                since,
                travelled: 0.0,
            });
        }
    }

    fn glide(&mut self, window: &mut Window, now: Instant) {
        let Some(momentum) = &mut self.momentum else {
            return;
        };
        let state = Decay::NORMAL.state_at(
            SpringState {
                position: 0.0,
                velocity: momentum.velocity,
            },
            now.saturating_duration_since(momentum.since).as_secs_f64(),
        );
        let step = state.position - momentum.travelled;
        momentum.travelled = state.position;
        if state.velocity.abs() < MIN_MOMENTUM {
            self.momentum = None;
        }
        self.scroll_smoothly(window, step as f32);
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
        let pending = self
            .tabs
            .iter()
            .flat_map(|tab| &tab.sessions)
            .any(Session::has_pending_output);
        self.retry_at = match (pending, self.retry_at) {
            (false, _) => None,
            (true, Some(at)) => Some(at),
            (true, None) => Some(Instant::now() + WRITE_RETRY),
        };
    }

    fn focus(&mut self, window: &mut Window, focused: bool) {
        self.focused = focused;
        if focused {
            if let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                tracing::warn!("{error}");
            }
            if self.badge > 0 {
                self.badge = 0;
                self.bus.send(Outgoing::Badge(0));
            }
        }
        self.focus_terminal(focused && !self.banner_focused);
        window.request_redraw();
    }

    fn flash_opacity(&self, now: Instant) -> Option<f32> {
        let elapsed = now.duration_since(self.flash?);
        (elapsed < FLASH)
            .then(|| FLASH_OPACITY * (1.0 - elapsed.as_secs_f32() / FLASH.as_secs_f32()))
    }

    fn sheet_items(&mut self) -> Vec<Item> {
        let mut highlights = self.session().terminal.highlights();
        highlights.link = self.link.as_ref().map(|link| link.range.clone());
        let area = self.pane_area();
        let tab = &self.tabs[self.active];
        let panes: Vec<Pane<'_>> = tab
            .layout
            .frames(area)
            .into_iter()
            .filter_map(|(id, frame)| {
                tab.session(id).map(|session| Pane {
                    frame,
                    terminal: &session.terminal,
                    focused: id == tab.focused,
                })
            })
            .collect();
        let dividers = tab.layout.dividers(area);
        self.painter.panes(&panes, &dividers, &highlights)
    }
}

impl App for TerminalApp {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let header = window.header_height() as f32;
        if header != self.header {
            self.header = header;
            self.frame_window(window);
        }
        let size = window.size();
        let rescaled = self
            .painter
            .set_metrics(self.painter.font_size(), window.scale());
        if rescaled || size != self.size {
            self.size = size;
            self.frame_window(window);
            self.layout();
        }
        self.glide(window, window.frame_time().presentation);
        let mut items = Vec::new();
        if self.has_strip() {
            let titles = self.titles();
            let bells: Vec<bool> = self.tabs.iter().map(|tab| tab.bell).collect();
            let bar = TabBar {
                titles: &titles,
                bells: &bells,
                active: self.active,
                hover: self.hover,
                focused: self.focused,
                leading: self.leading(),
            };
            items.extend(self.painter.tab_bar(&bar, size.0 as f32));
        }
        let sheet = self.sheet();
        let mut inside = self.sheet_items();
        if let Some(banner) = &mut self.banner {
            inside.extend(banner.items(size.0, size.1));
        }
        if let Some(opacity) = self.flash_opacity(Instant::now()) {
            inside.push(Item::Fill {
                shape: Shape::rect(sheet),
                paint: self.painter.theme.foreground.with_opacity(opacity).into(),
            });
        }
        let radius = if self.header > 0.0 {
            Radius::concentric(Radius::XL, SHEET_INSET as f32)
        } else {
            0.0
        };
        items.push(Item::Clip {
            shape: Shape::rounded(sheet, radius),
            items: inside,
        });
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
            WindowEvent::Accessibility(request)
                if request.target_node == NEW_TAB_NODE && request.action == Action::Click =>
            {
                self.new_tab(window);
            }
            WindowEvent::Accessibility(request)
                if request.target_node.0 >= TAB_NODES
                    && request.target_node.0 < TAB_NODES + self.tabs.len() as u64 =>
            {
                let index = (request.target_node.0 - TAB_NODES) as usize;
                self.select_tab(window, index);
            }
            WindowEvent::Accessibility(request) => {
                if let Some(banner) = &mut self.banner {
                    banner.perform(&request);
                    window.request_redraw();
                }
            }
            WindowEvent::Pasted(text) => self.paste(&text),
            WindowEvent::Drag(drag) => self.drag(window, drag),
            WindowEvent::MenuAction(action) => self.menu_action(window, &action),
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
        self.handle_notices(window, self.active, notices);
        self.remove_exited(window);
        if !self.tabs.is_empty() {
            self.sync_title(window);
        }
        self.schedule_retry();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = self
            .tabs
            .iter()
            .flat_map(|tab| tab.sessions.iter().map(Session::fd))
            .collect::<Vec<_>>();
        sources.extend(self.app_menu.as_ref().map(AppMenu::fd));
        sources.extend(self.settings_watch.as_ref().map(|watch| watch.wake.as_fd()));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let sessions: usize = self.tabs.iter().map(|tab| tab.sessions.len()).sum();
        let menu_index = self.app_menu.as_ref().map(|_| sessions);
        if Some(index) == menu_index {
            let actions = self
                .app_menu
                .as_mut()
                .map(AppMenu::take_actions)
                .unwrap_or_default();
            for action in actions {
                if self.tabs.is_empty() {
                    return;
                }
                self.menu_action(window, &action);
            }
            self.handle_banner_actions(window);
            return window.request_redraw();
        }
        if index >= sessions {
            if let Some(watch) = &mut self.settings_watch {
                watch.drain();
            }
            return self.reload_settings(window);
        }
        let mut remaining = index;
        let Some(tab) = self.tabs.iter().position(|tab| {
            let inside = remaining < tab.sessions.len();
            if !inside {
                remaining -= tab.sessions.len();
            }
            inside
        }) else {
            return;
        };
        let notices = self.tabs[tab].sessions[remaining].read();
        self.handle_notices(window, tab, notices);
        if tab == self.active || self.has_strip() {
            window.request_redraw();
        }
        self.remove_exited(window);
        if !self.tabs.is_empty() {
            self.sync_title(window);
        }
        self.schedule_retry();
    }

    fn deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) {
        if self.retry_at.is_some_and(|at| at <= now) {
            self.retry_at = None;
            for session in self.tabs.iter_mut().flat_map(|tab| &mut tab.sessions) {
                session.flush();
            }
            self.schedule_retry();
        }
    }

    fn animating(&self) -> bool {
        self.momentum.is_some() || self.flash_opacity(Instant::now()).is_some()
    }

    fn should_close(&mut self, window: &mut Window) -> bool {
        let by_button = window.close_requested_by_button();
        let last = self.tabs.len() == 1 && (by_button || self.tabs[0].sessions.len() == 1);
        if last && self.tabs[0].running_program().is_none() {
            self.forget_window();
            return true;
        }
        if by_button {
            self.request_close_tab(window, self.active);
        } else {
            self.request_close(window);
        }
        false
    }

    fn should_quit(&mut self, window: &mut Window) -> bool {
        self.may_quit(window)
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
        let mut children = Vec::new();
        let mut nodes = Vec::new();
        if self.has_strip() {
            let layout = self.tab_layout();
            let mut list = Node::new(Role::TabList);
            list.set_label(L10N.tr("tabs"));
            let titles = self.titles();
            let mut tabs = Vec::new();
            for (index, (rect, title)) in layout.tabs.iter().zip(&titles).enumerate() {
                let id = NodeId(TAB_NODES + index as u64);
                let mut tab = Node::new(Role::Tab);
                tab.set_label(title.clone());
                tab.set_bounds(bounds(*rect));
                tab.set_selected(index == self.active);
                tab.add_action(Action::Click);
                tabs.push(id);
                nodes.push((id, tab));
            }
            let mut new_tab = Node::new(Role::Button);
            new_tab.set_label(L10N.tr("new-tab"));
            new_tab.set_bounds(bounds(layout.new_tab));
            new_tab.add_action(Action::Click);
            nodes.push((NEW_TAB_NODE, new_tab));
            tabs.push(NEW_TAB_NODE);
            list.set_children(tabs);
            nodes.push((TAB_LIST_NODE, list));
            children.push(TAB_LIST_NODE);
        }
        children.push(TERMINAL_NODE);
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

fn bounds(rect: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: f64::from(rect.x),
        y0: f64::from(rect.y),
        x1: f64::from(rect.x + rect.width),
        y1: f64::from(rect.y + rect.height),
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

pub fn spawn_self(window: &mut Window, arguments: Vec<String>) {
    let program = match std::env::current_exe() {
        Ok(program) => program.to_string_lossy().into_owned(),
        Err(error) => return tracing::warn!("cannot find the terminal program: {error}"),
    };
    let token = window
        .activation_token(APP_ID)
        .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
        .ok();
    let mut command = vec![program];
    command.extend(arguments);
    if let Err(error) = unbound_xdg::spawn_detached(&command, None, token.as_deref()) {
        tracing::warn!("cannot open a window: {error}");
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

fn asks_for_menu(pointer: &Pointer) -> bool {
    pointer.button == PointerButton::Secondary
        || (pointer.is_primary() && pointer.modifiers.control)
}

fn opens_menu(key: Key, modifiers: Modifiers) -> bool {
    let shift_only = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    key == Key::Menu || (key == Key::Function(10) && modifiers == shift_only)
}

fn quoted(name: &str) -> String {
    format!("\u{201C}{name}\u{201D}")
}

pub const DIRECTORY_OPTION: &str = "--working-directory";
pub const SETTINGS_OPTION: &str = "--settings";
pub const RESTORE_OPTION: &str = "--restore";

#[derive(Debug, Default)]
pub struct Arguments {
    pub command: Option<Shell>,
    pub directory: Option<PathBuf>,
    pub settings: bool,
    pub restore: Option<PathBuf>,
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
                SETTINGS_OPTION => parsed.settings = true,
                RESTORE_OPTION => parsed.restore = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        parsed
    }

    pub fn is_plain(&self) -> bool {
        self.command.is_none() && self.directory.is_none() && !self.settings
    }
}
