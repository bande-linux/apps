use std::cell::Cell;
use std::collections::VecDeque;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use alacritty_terminal::term::TermMode;
use bande_app::{Detached, TabDrop, TabDropTarget, TabHost, View, move_tab};
use bande_app_settings::{Stored, settings_folder};
use bande_spring::{Decay, State as SpringState};
use bande_terminal::ShellCommand;
use bande_terminal::{
    Axis, Banner, BannerAction, CellPoint, CloseAsk, Command, Divider, FULL_SCREEN_LEADING,
    GridSize, L10N, Layout, Link, MenuCommand, MouseButton, MouseKind, NewTabFolder, Notice,
    Painter, Pane, STRIP_HEIGHT, SearchDirection, SelectionKind, Session, Settings, Terminal,
    TerminalContext, bracketed, dropped_text, encode_key, is_openable, look_up_text, mouse_report,
    next_font_size, reports_mouse, shortcut, tab_menu, terminal_menu, wants_scroll_as_arrows,
};
use bande_toolkit::ui::{
    Cursor, Event, Key, Menu, Modifiers, Pointer, PointerButton, Scroll, ScrollExtent, ScrollPhase,
    ScrollSource, Scrollbar, ScrollbarAction, TabAction, TabLabel, TabStrip, TabStripLayout,
    TabsShown,
};
use bande_toolkit::{
    Appearance, DragAction, DragEvent, FrameRegions, HEADER_LEADING, Header, Item, MenuPlacement,
    Point, Radius, Rect, SHEET_INSET, Scene, Shape, Transfer, Window, WindowEvent,
};

use crate::APP_ID;
use crate::bus::Bus;

const BADGE_GAP: Duration = Duration::from_millis(250);
const FILES: &str = "bande-files";
const FILES_APP_ID: &str = "org.bande.Files";
const WINDOW_NODE: NodeId = NodeId(u64::MAX);
const TERMINAL_NODE: NodeId = NodeId(u64::MAX - 1);
const TAB_LIST_NODE: NodeId = NodeId(u64::MAX - 2);
const NEW_TAB_NODE: NodeId = NodeId(u64::MAX - 3);
const TAB_NODES: u64 = u64::MAX - 1024;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const MIN_MOMENTUM: f64 = 30.0;
const LONG_COMMAND: Duration = Duration::from_secs(10);
const FLASH: Duration = Duration::from_millis(220);
const FLASH_OPACITY: f32 = 0.14;
const MIN_PANE: f32 = 64.0;
const BLINK: Duration = Duration::from_millis(530);

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
    Strip,
    Banner,
    Divider(Divider),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    CloseTab(u32),
    ClosePane(u32),
    CloseOtherTabs(u32),
    CloseWindow,
    Quit,
    Paste(u32, String),
    Rename(u32),
}

struct Tab {
    id: u32,
    sessions: Vec<Session>,
    layout: Layout,
    focused: u32,
    bell: bool,
    title: Option<String>,
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
            title: None,
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

pub struct CarriedTab(Box<Tab>);

pub enum Ask {
    NewWindow(Option<PathBuf>),
    Settings,
    Quit,
    Detach(Detached<CarriedTab>),
    TabDrop(TabDrop),
}

pub struct TerminalWindow {
    tabs: Vec<Tab>,
    active: usize,
    painter: Painter,
    settings: Settings,
    home: Option<PathBuf>,
    focused: bool,
    pointer: Point,
    gesture: Gesture,
    strip: TabStrip,
    reported_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
    title: String,
    size: (u32, u32),
    header: f32,
    banner: Option<Banner>,
    banner_focused: bool,
    composing: String,
    droppable: bool,
    pending: Option<Pending>,
    last_query: String,
    link: Option<Link>,
    samples: VecDeque<(Duration, f32)>,
    momentum: Option<Momentum>,
    scrollbar: Scrollbar,
    sliding: bool,
    flash: Option<Instant>,
    badge: u32,
    badged: Option<Instant>,
    cursor: Cursor,
    bus: Rc<Bus>,
    serial: u32,
    remembered: bool,
    asks: Vec<Ask>,
    carrying: Rc<Cell<bool>>,
    drop_target: TabDropTarget,
    blink_from: Instant,
    blink_phase: u32,
    alt_click: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedWindow {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
}

pub struct Opened {
    pub sessions: Vec<Session>,
    pub active: usize,
    pub remembered: bool,
}

impl TerminalWindow {
    pub fn new(
        painter: Painter,
        settings: Settings,
        opened: Opened,
        home: Option<PathBuf>,
        bus: Rc<Bus>,
        carrying: Rc<Cell<bool>>,
        serial: u32,
    ) -> Self {
        let Opened {
            sessions,
            active,
            remembered,
        } = opened;
        let mut app = Self {
            active: active.min(sessions.len().saturating_sub(1)),
            tabs: sessions.into_iter().map(Tab::new).collect(),
            painter,
            settings,
            home,
            focused: false,
            pointer: Point::default(),
            gesture: Gesture::Idle,
            strip: TabStrip::new(),
            reported_cell: None,
            scroll_remainder: 0.0,
            title: String::new(),
            size: (0, 0),
            header: 0.0,
            banner: None,
            banner_focused: false,
            composing: String::new(),
            droppable: false,
            pending: None,
            last_query: String::new(),
            link: None,
            samples: VecDeque::new(),
            momentum: None,
            scrollbar: Scrollbar::default(),
            sliding: false,
            flash: None,
            badge: 0,
            badged: None,
            cursor: Cursor::Default,
            bus,
            serial,
            remembered,
            asks: Vec::new(),
            carrying,
            drop_target: TabDropTarget::default(),
            blink_from: Instant::now(),
            blink_phase: 0,
            alt_click: None,
        };
        for tab in &mut app.tabs {
            for session in &mut tab.sessions {
                session.terminal.set_focus(false);
            }
        }
        app
    }

    pub fn attach(&mut self, window: &mut Window) {
        self.header = window.header_height() as f32;
        self.painter
            .set_metrics(self.painter.font_size(), window.scale());
        self.frame_window(window);
        self.size = window.size();
        self.frame_window(window);
        self.layout();
    }

    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    pub fn busy_tabs(&self) -> usize {
        self.tabs
            .iter()
            .filter(|tab| tab.running_program().is_some())
            .count()
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
        if !self.session().send(bytes) {
            self.bus.ring();
        }
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

    fn tab_layout(&self) -> TabStripLayout {
        TabStripLayout::new(
            self.tabs.len(),
            self.size.0 as f32,
            self.leading(),
            STRIP_HEIGHT,
        )
    }

    fn sync_strip(&mut self) -> TabStripLayout {
        let ids: Vec<u64> = self.tabs.iter().map(|tab| u64::from(tab.id)).collect();
        self.strip.sync(&ids);
        self.tab_layout()
    }

    fn in_strip(&self, at: Point) -> bool {
        self.has_strip() && at.y < STRIP_HEIGHT
    }

    fn strip_action(&mut self, window: &mut Window, action: TabAction) {
        match action {
            TabAction::Select(index) => self.select_tab(window, index),
            TabAction::Close(index) => self.request_close_tab(window, index),
            TabAction::NewTab => self.new_tab(window),
            TabAction::Menu { tab, at } => {
                let tab = tab.and_then(|index| self.tabs.get(index)).map(|tab| tab.id);
                self.show_menu(window, tab_menu(tab, self.tabs.len()), at);
            }
            TabAction::Moved { from, to } => self.move_tab(from, to),
            TabAction::Detach { index, grab } => self.detach_tab(window, index, grab),
        }
        window.request_redraw();
    }

    fn move_tab(&mut self, from: usize, to: usize) {
        if move_tab(&mut self.tabs, &mut self.active, from, to) {}
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
        window.set_detachable_tabs(self.tabs.len() > 1);
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
        let banner = self.banner.as_ref().map_or(0.0, Banner::height);
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
            .find(|(_, frame)| frame.contains(at))
            .map(|(id, _)| id)
    }

    fn divider_at(&self, at: Point) -> Option<Divider> {
        self.tab()
            .layout
            .dividers(self.pane_area())
            .into_iter()
            .find(|divider| divider.reach().contains(at))
    }

    fn layout(&mut self) {
        let sheet = self.sheet();
        if let Some(banner) = &mut self.banner {
            let height = banner.height();
            banner.set_area(Rect::new(sheet.x, sheet.y, sheet.width, height));
        }
        let area = self.pane_area();
        let metrics = self.painter.metrics;
        let cell_size = metrics.pixel_cell_size();
        let shape = self.settings.cursor.shape();
        let scrollback = self.settings.scrollback;
        for tab in &mut self.tabs {
            for (id, frame) in tab.layout.frames(area) {
                let grid = metrics.grid_for(frame.width, frame.height);
                if let Some(session) = tab.sessions.iter_mut().find(|session| session.id() == id) {
                    session.resize(grid, cell_size);
                    session.terminal.set_options(shape, scrollback);
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

    fn tab_title(&self, tab: &Tab) -> String {
        if let Some(title) = &tab.title {
            return title.clone();
        }
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
    }

    pub fn saved_window(&self) -> Option<SavedWindow> {
        self.remembered.then(|| SavedWindow {
            tabs: self
                .tabs
                .iter()
                .filter_map(|tab| {
                    tab.focused()
                        .working_directory()
                        .or_else(|| self.home.clone())
                })
                .collect(),
            active: self.active,
        })
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
                    self.bus.notify(summary, body);
                }
                Notice::Finished { .. } => {}
                Notice::Notify { title, body } if background => {
                    self.bus.notify(title, body);
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
        let now = Instant::now();
        let due = self
            .badged
            .is_none_or(|badged| now.duration_since(badged) >= BADGE_GAP);
        if !self.focused && self.settings.badge_on_bell && due {
            self.badged = Some(now);
            self.badge += 1;
            self.bus.set_badge(self.serial, self.badge);
        }
        if self.settings.audible_bell {
            self.bus.ring();
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

    fn new_tab_folder(&mut self) -> Option<PathBuf> {
        match self.settings.new_tab_folder {
            NewTabFolder::Same => self
                .session()
                .working_directory()
                .or_else(|| self.home.clone()),
            NewTabFolder::Home => self.home.clone(),
        }
    }

    fn spawn_session(&mut self, size: GridSize) -> Option<Session> {
        let directory = self.new_tab_folder();
        let mut terminal = Terminal::new(size);
        terminal.resize(size, self.painter.metrics.pixel_cell_size());
        terminal.set_options(self.settings.cursor.shape(), self.settings.scrollback);
        Session::spawn(terminal, self.settings.shell(), directory)
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

    fn new_window(&mut self) {
        let directory = self.new_tab_folder();
        self.asks.push(Ask::NewWindow(directory));
    }

    fn asks_before_closing(&self, running: bool) -> bool {
        match self.settings.close_ask {
            CloseAsk::Always => true,
            CloseAsk::WhenRunning => running,
            CloseAsk::Never => false,
        }
    }

    fn request_close_tab(&mut self, window: &mut Window, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let program = tab.running_program();
        if !self.asks_before_closing(program.is_some()) {
            return self.close_tab(window, index);
        }
        let message = match program {
            Some(program) => {
                L10N.tr_args("confirm-close-tab", [("program", quoted(program).into())])
            }
            None => L10N.tr("confirm-close-idle-tab"),
        };
        let pending = Pending::CloseTab(tab.id);
        self.select_tab(window, index);
        self.ask(window, &message, pending);
    }

    fn request_close(&mut self, window: &mut Window) {
        if self.tab().sessions.len() == 1 {
            return self.request_close_tab(window, self.active);
        }
        let id = self.tab().focused;
        let program = self.tab().focused().running_program();
        if !self.asks_before_closing(program.is_some()) {
            return self.close_pane(window, self.active, id);
        }
        let message = match program {
            Some(program) => {
                L10N.tr_args("confirm-close-pane", [("program", quoted(program).into())])
            }
            None => L10N.tr("confirm-close-idle-pane"),
        };
        self.ask(window, &message, Pending::ClosePane(id));
    }

    fn ask(&mut self, window: &mut Window, message: &str, pending: Pending) {
        self.close_banner();
        let appearance = window.appearance();
        let button = match pending {
            Pending::Quit => L10N.tr("quit"),
            Pending::Paste(..) => L10N.tr("paste"),
            Pending::Rename(_) => L10N.tr("rename"),
            Pending::CloseTab(_)
            | Pending::ClosePane(_)
            | Pending::CloseOtherTabs(_)
            | Pending::CloseWindow => L10N.tr("close"),
        };
        let text = self.painter.text_system().sharing_fonts();
        self.banner = Some(Banner::confirm(appearance, text, message, &button));
        self.pending = Some(pending);
        self.banner_focused = true;
        self.focus_terminal(false);
        self.layout();
        window.request_redraw();
    }

    fn confirm(&mut self, window: &mut Window) {
        let pending = self.pending.take();
        let query = self.banner.as_ref().map(Banner::query);
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
            Some(Pending::CloseWindow) => {
                while !self.tabs.is_empty() {
                    self.close_tab(window, 0);
                }
            }
            Some(Pending::Quit) => self.asks.push(Ask::Quit),
            Some(Pending::Paste(id, text)) => {
                if self.tab().focused().id() == id {
                    self.paste_now(&text);
                }
            }
            Some(Pending::Rename(id)) => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.title = query
                        .map(|title| title.trim().to_owned())
                        .filter(|title| !title.is_empty());
                }
                self.sync_title(window);
            }
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
        if self.tabs.len() == 1 || !self.asks_before_closing(busy > 0) {
            return self.remove_other_tabs(window, keep);
        }
        self.select_tab(window, index);
        let message = match busy {
            0 => L10N.tr("confirm-close-idle-other-tabs"),
            busy => L10N.tr_args("confirm-close-other-tabs", [("count", busy.into())]),
        };
        self.ask(window, &message, Pending::CloseOtherTabs(keep));
    }

    pub fn ask_to_quit(&mut self, window: &mut Window, busy: usize) {
        let message = match busy {
            0 => L10N.tr("confirm-quit-idle"),
            busy => L10N.tr_args("confirm-quit", [("count", busy.into())]),
        };
        self.ask(window, &message, Pending::Quit);
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
        drop(self.take_tab(window, index));
    }

    fn take_tab(&mut self, window: &mut Window, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let was_active = index == self.active;
        if was_active {
            self.focus_terminal(false);
        }
        let closed = self.tabs.remove(index);
        if self.tabs.is_empty() {
            return Some(closed);
        }
        if index < self.active || self.active >= self.tabs.len() {
            self.active -= 1;
        }
        let stale = match &self.pending {
            Some(Pending::CloseTab(id) | Pending::CloseOtherTabs(id)) => *id == closed.id,
            Some(Pending::Paste(..)) => was_active,
            Some(Pending::Rename(id)) => *id == closed.id,
            Some(Pending::ClosePane(id)) => closed.session(*id).is_some(),
            Some(Pending::CloseWindow | Pending::Quit) | None => false,
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
        let layout = self.sync_strip();
        self.strip.hover(&layout, None);
        self.after_tabs_changed(window);
        Some(closed)
    }

    fn detach_tab(&mut self, window: &mut Window, index: usize, grab: Point) {
        self.gesture = Gesture::Idle;
        let Some((title, id)) = self
            .tabs
            .get(index)
            .map(|tab| (self.tab_title(tab), u64::from(tab.id)))
        else {
            return;
        };
        let drag = match window.start_window_drag(Transfer::text(&title)) {
            Ok(drag) => drag,
            Err(error) => {
                tracing::warn!("cannot drag the tab out: {error}");
                return;
            }
        };
        let sheet = self.sheet();
        let size = (sheet.width.round() as u32, sheet.height.round() as u32);
        let Some(tab) = self.take_tab(window, index) else {
            return;
        };
        self.carrying.set(true);
        self.asks.push(Ask::Detach(Detached {
            tab: CarriedTab(Box::new(tab)),
            id,
            index,
            drag,
            grab,
            size,
        }));
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
        if key == Key::Menu && !self.banner_focused {
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
        let shift_only = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        if modifiers == shift_only && !in_alternate_screen {
            match key {
                Key::PageUp => return self.scroll_pages(window, 1),
                Key::PageDown => return self.scroll_pages(window, -1),
                _ => {}
            }
        }
        let modifiers = match key {
            Key::Character(_) if !self.settings.alt_as_meta => Modifiers {
                alt: false,
                ..modifiers
            },
            _ => modifiers,
        };
        self.blink_from = Instant::now();
        if let Some(bytes) = encode_key(key, modifiers, mode) {
            self.session().terminal.scroll_to_bottom();
            if key == Key::Enter && modifiers == Modifiers::default() && !in_alternate_screen {
                self.session().terminal.mark_prompt();
            }
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
            Command::SelectOutput => {
                if self.session().terminal.select_command_output() {
                    self.offer_primary(window);
                }
            }
            Command::BiggerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), true))
            }
            Command::SmallerText => {
                self.set_font_size(window, next_font_size(self.painter.font_size(), false))
            }
            Command::DefaultTextSize => self.set_font_size(window, self.settings.font_size),
            Command::NewTab => self.new_tab(window),
            Command::NewWindow => self.new_window(),
            Command::NextTab => self.select_tab(window, (self.active + 1) % self.tabs.len()),
            Command::PreviousTab => {
                let count = self.tabs.len();
                self.select_tab(window, (self.active + count - 1) % count);
            }
            Command::SelectTab(index) => self.select_tab(window, index),
            Command::LastTab => self.select_tab(window, self.tabs.len() - 1),
            Command::Find => self.open_find(window),
            Command::EditTitle => self.edit_title(window),
            Command::FindNext => self.find_again(window, SearchDirection::Newer),
            Command::FindPrevious => self.find_again(window, SearchDirection::Older),
            Command::SplitSideBySide => self.split(window, Axis::SideBySide),
            Command::SplitStacked => self.split(window, Axis::Stacked),
            Command::NextPane => self.move_focus(window, true),
            Command::PreviousPane => self.move_focus(window, false),
            Command::PreviousPrompt => {
                self.session().terminal.jump_to_prompt(true);
            }
            Command::NextPrompt => {
                self.session().terminal.jump_to_prompt(false);
            }
            Command::Settings => self.asks.push(Ask::Settings),
        }
        window.request_redraw();
    }

    pub fn printable_text(&self) -> (String, String) {
        let title = self.titles().into_iter().next().unwrap_or_default();
        (title, self.terminal().all_text().trim_end().to_owned())
    }

    pub fn menu_action(&mut self, window: &mut Window, action: &str) -> bool {
        let Some(command) = MenuCommand::parse(action) else {
            return false;
        };
        self.perform_menu(window, command);
        self.settle(window);
        true
    }

    pub fn close_command(&mut self, window: &mut Window) {
        self.request_close(window);
        self.settle(window);
    }

    pub fn close_by_button(&mut self, window: &mut Window) -> bool {
        let busy = self.busy_tabs();
        if !self.asks_before_closing(busy > 0) {
            return true;
        }
        let message = match busy {
            0 => L10N.tr("confirm-close-idle-window"),
            busy => L10N.tr_args("confirm-close-window", [("count", busy.into())]),
        };
        self.ask(window, &message, Pending::CloseWindow);
        self.settle(window);
        false
    }

    pub fn finished(&self) -> bool {
        self.tabs.is_empty()
    }

    fn settle(&mut self, window: &mut Window) {
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
    }

    fn perform_menu(&mut self, window: &mut Window, command: MenuCommand) {
        match command {
            MenuCommand::Run(command) => self.perform(window, command),
            MenuCommand::OpenLink(uri) => self.open_link(window, &uri),
            MenuCommand::CopyLink(uri) => window.copy(&uri),
            MenuCommand::ShowInFiles => {
                if let Some(directory) = self.session().working_directory() {
                    self.show_in_files(window, &directory);
                }
            }
            MenuCommand::Reveal(path) => self.show_in_files(window, &path),
            MenuCommand::LookUp => {
                if let Some(text) = look_up_text(self.terminal().selected_text()) {
                    window.look_up(text);
                }
            }
            MenuCommand::CloseTab(id) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.request_close_tab(window, index);
                }
            }
            MenuCommand::CloseOtherTabs(id) => self.close_other_tabs(window, id),
        }
    }

    fn show_in_files(&mut self, window: &mut Window, path: &Path) {
        let token = window
            .activation_token(APP_ID)
            .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
            .ok();
        let command = [FILES.to_owned(), path.to_string_lossy().into_owned()];
        if let Err(error) =
            bande_xdg::spawn_detached(FILES_APP_ID, &command, None, token.as_deref())
        {
            tracing::warn!("cannot show {} in Files: {error}", path.display());
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
            look_up: look_up_text(self.terminal().selected_text()),
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

    fn edit_title(&mut self, window: &mut Window) {
        self.close_banner();
        let title = self.tab_title(self.tab());
        let text = self.painter.text_system().sharing_fonts();
        self.banner = Some(Banner::rename(window.appearance(), text, &title));
        self.pending = Some(Pending::Rename(self.tab().id));
        self.banner_focused = true;
        self.focus_terminal(false);
        self.layout();
        window.request_redraw();
    }

    fn open_find(&mut self, window: &Window) {
        match self.find_banner() {
            Some(banner) => banner.take_focus(),
            None => {
                self.close_banner();
                let text = self.painter.text_system().sharing_fonts();
                self.banner = Some(Banner::find(window.appearance(), text, &self.last_query));
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

    fn paste(&mut self, window: &mut Window, text: &str) {
        let lines = text
            .replace("\r\n", "\n")
            .trim_end_matches(['\r', '\n'])
            .split(['\r', '\n'])
            .count();
        let guarded = self.mode().contains(TermMode::BRACKETED_PASTE);
        if self.settings.paste_warning && lines > 1 && !guarded {
            let message = L10N.tr_args("confirm-paste", [("count", lines.into())]);
            let id = self.tab().focused().id();
            return self.ask(window, &message, Pending::Paste(id, text.to_owned()));
        }
        self.paste_now(text);
    }

    fn paste_now(&mut self, text: &str) {
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
        if self.carrying.get() {
            return self.tab_drag(window, drag);
        }
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
                self.paste(window, &dropped_text(&transfer.uris, &transfer.text));
                window.request_redraw();
            }
            DragEvent::Finished(_) => {}
        }
    }

    fn tab_drag(&mut self, window: &mut Window, drag: DragEvent) {
        let strip = self.has_strip().then_some(STRIP_HEIGHT);
        if let Some(drop) = self.drop_target.event(window, strip, drag) {
            self.asks.push(Ask::TabDrop(drop));
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

    pub fn reload_settings(&mut self, window: &mut Window) {
        let Some(folder) = settings_folder() else {
            return;
        };
        let settings = Settings::load(&folder);
        if settings == self.settings {
            return;
        }
        self.painter.set_profile(settings.profile);
        if self.settings.font != settings.font {
            self.painter.set_font(&settings.font);
        }
        if self.settings.font_size != settings.font_size {
            self.painter.set_metrics(settings.font_size, window.scale());
        }
        self.settings = settings;
        self.blink_from = Instant::now();
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
        if self.scrollbar_pointer(window, &Event::PointerMove(pointer)) {
            return self.update_cursor(window, modifiers);
        }
        let layout = self.sync_strip();
        let over_strip = self.in_strip(at);
        if self.strip.hover(&layout, over_strip.then_some(at)) {
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
            Gesture::Strip => {
                let now = window.frame_time().on_screen();
                let reduce_motion = window.appearance().reduce_motion;
                if let Some(action) = self.strip.moved(&layout, at, now, reduce_motion) {
                    self.strip_action(window, action);
                }
                if self.strip.is_engaged() && matches!(self.gesture, Gesture::Idle) {
                    self.gesture = Gesture::Strip;
                }
                window.request_redraw();
            }
            Gesture::Idle if !over_strip && self.in_content(at) => {
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
        } else if self.over_scrollbar(at)
            || !self.in_content(at)
            || (reports_mouse(self.mode()) && !modifiers.shift)
        {
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
        self.pane_area().contains(at)
    }

    fn link_at(&mut self, at: Point) -> Option<Link> {
        let cell = self.cell_at(at);
        let directory = self.session().working_directory();
        let terminal = &mut self.session().terminal;
        terminal.link_at(cell).or_else(|| {
            let home = std::env::home_dir()?;
            terminal.path_at(cell, directory.as_deref().unwrap_or(&home), &home)
        })
    }

    fn hover_link(&mut self, window: &mut Window, modifiers: Modifiers) {
        let at = self.pointer;
        let over_focused = self.pane_at(at) == Some(self.tab().focused);
        let link = if modifiers.control && self.gesture == Gesture::Idle && over_focused {
            self.link_at(at)
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
        let uri = uri.to_owned();
        let spawned = std::thread::Builder::new()
            .name("open link".to_owned())
            .spawn(move || {
                if let Err(error) = bande_xdg::open(&uri, token.as_deref()) {
                    tracing::warn!("cannot open {uri}: {error}");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot open a link: {error}");
        }
    }

    fn end_gesture(&mut self, window: &mut Window) {
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Reporting(button) => {
                self.report_mouse(
                    MouseKind::Release(button),
                    self.pointer,
                    Modifiers::default(),
                );
            }
            Gesture::Selecting => {
                self.session().terminal.finish_selection();
                self.offer_primary(window);
                window.request_redraw();
            }
            Gesture::Strip => {
                let layout = self.tab_layout();
                self.strip.cancel(
                    &layout,
                    window.frame_time().on_screen(),
                    window.appearance().reduce_motion,
                );
                window.request_redraw();
            }
            Gesture::Banner | Gesture::Divider(_) | Gesture::Idle => {}
        }
    }

    fn release(&mut self, window: &mut Window, pointer: Pointer) {
        let at = pointer.position;
        if self.scrollbar_pointer(window, &Event::PointerUp(pointer)) {
            return self.update_cursor(window, pointer.modifiers);
        }
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Reporting(button) if mouse_button(pointer.button) == Some(button) => {
                self.report_mouse(MouseKind::Release(button), at, pointer.modifiers);
            }
            Gesture::Selecting if pointer.is_primary() => {
                let cell = self.cell_at(at);
                if self.alt_click.take() == Some((cell.column, cell.line)) {
                    self.session().terminal.clear_selection();
                    self.move_cursor_to(cell.column, cell.line);
                } else {
                    self.session().terminal.finish_selection();
                    self.offer_primary(window);
                }
                window.request_redraw();
            }
            Gesture::Strip => {
                let layout = self.sync_strip();
                let now = window.frame_time().on_screen();
                let reduce_motion = window.appearance().reduce_motion;
                if let Some(action) = self.strip.released(&layout, at, now, reduce_motion) {
                    self.strip_action(window, action);
                }
                window.request_redraw();
            }
            Gesture::Banner => {
                if let Some(banner) = &mut self.banner {
                    banner.handle(&Event::PointerUp(pointer));
                    window.request_redraw();
                }
            }
            gesture @ (Gesture::Reporting(_) | Gesture::Selecting) => self.gesture = gesture,
            Gesture::Divider(_) | Gesture::Idle => {}
        }
        self.update_cursor(window, pointer.modifiers);
    }

    fn press(&mut self, window: &mut Window, pointer: Pointer) {
        let at = pointer.position;
        self.momentum = None;
        if self.in_strip(at) {
            let layout = self.sync_strip();
            if let Some(action) = self.strip.press(&layout, &pointer) {
                self.strip_action(window, action);
            }
            if self.strip.is_engaged() {
                self.gesture = Gesture::Strip;
            }
            if layout.hit(at).is_some() {
                return;
            }
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
        if self.scrollbar_pointer(window, &Event::PointerDown(pointer)) || !self.in_content(at) {
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
            let link = self.link_at(at);
            return self.terminal_menu(window, at, link);
        }
        if modifiers.control && pointer.is_primary() {
            if let Some(link) = self.link_at(at) {
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
        self.alt_click = (modifiers.alt && pointer.count == 1).then_some((cell.column, cell.line));
        let terminal = &mut self.session().terminal;
        if modifiers.shift && terminal.has_selection() {
            terminal.extend_selection(cell);
        } else {
            let kind = SelectionKind::for_clicks(pointer.count, modifiers.alt);
            terminal.start_selection(cell, kind);
        }
        window.request_redraw();
    }

    fn move_cursor_to(&mut self, column: usize, line: usize) {
        let mode = self.mode();
        if mode.contains(TermMode::ALT_SCREEN) {
            return;
        }
        let Some((from, on)) = self.session().terminal.cursor_cell() else {
            return;
        };
        if on != line || from == column {
            return;
        }
        let key = if column > from { Key::Right } else { Key::Left };
        let Some(step) = encode_key(key, Modifiers::default(), mode) else {
            return;
        };
        self.send(&step.repeat(column.abs_diff(from)));
    }

    fn in_banner(&self, at: Point) -> bool {
        self.banner
            .as_ref()
            .is_some_and(|banner| banner.contains(at))
    }

    fn scroll_pages(&mut self, window: &mut Window, pages: i32) {
        self.session().terminal.scroll_pages(pages);
        self.scrollbar.reveal(Instant::now());
        window.request_redraw();
    }

    fn scroll_extent(&self) -> Option<(Rect, ScrollExtent)> {
        let extent = self
            .terminal()
            .scroll_extent(self.painter.metrics.cell_height)?;
        Some((self.focused_frame(), extent))
    }

    fn over_scrollbar(&self, at: Point) -> bool {
        self.scroll_extent()
            .is_some_and(|(frame, extent)| self.scrollbar.claims(frame, &extent, at))
    }

    fn scrollbar_pointer(&mut self, window: &mut Window, event: &Event) -> bool {
        let (frame, extent) = self.scroll_extent().unwrap_or_default();
        let reduce_motion = self.painter.reduce_motion();
        let action = self
            .scrollbar
            .pointer(Instant::now(), reduce_motion, frame, &extent, event);
        match action {
            None => false,
            Some(ScrollbarAction::Redraw) => {
                window.request_redraw();
                matches!(event, Event::PointerDown(_) | Event::PointerUp(_))
            }
            Some(ScrollbarAction::Page(direction)) => {
                self.momentum = None;
                self.scroll_pages(window, -direction as i32);
                true
            }
            Some(ScrollbarAction::Seek(offset)) => {
                let line_height = self.painter.metrics.cell_height;
                self.session().terminal.seek(offset, line_height);
                window.request_redraw();
                true
            }
        }
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
        self.scrollbar.reveal(Instant::now());
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
        self.scrollbar.reveal(Instant::now());
        window.request_redraw();
    }

    fn focus(&mut self, focused: bool) {
        self.focused = focused;
        self.blink_from = Instant::now();
        if focused && self.badge > 0 {
            self.badge = 0;
            self.bus.set_badge(self.serial, 0);
        }
        self.focus_terminal(focused && !self.banner_focused);
    }

    fn blinking(&self) -> bool {
        self.settings.blink && self.focused && !self.banner_focused
    }

    fn blink_phase(&self, now: Instant) -> u32 {
        let phase = now.duration_since(self.blink_from).as_millis() / BLINK.as_millis();
        u32::try_from(phase).unwrap_or(u32::MAX)
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
        let dividers: Vec<Rect> = tab
            .layout
            .dividers(area)
            .iter()
            .map(|divider| divider.line)
            .collect();
        self.blink_phase = self.blink_phase(Instant::now());
        self.painter.cursor_hidden = self.blinking() && self.blink_phase % 2 == 1;
        self.painter
            .panes(&panes, &dividers, &highlights, &self.composing)
    }
}

impl TabHost for TerminalWindow {
    type Tab = CarriedTab;

    fn release_tab(&mut self, window: &mut Window, id: u64) -> Option<CarriedTab> {
        let index = self.tabs.iter().position(|tab| u64::from(tab.id) == id)?;
        self.take_tab(window, index)
            .map(|tab| CarriedTab(Box::new(tab)))
    }

    fn adopt_tab(&mut self, window: &mut Window, tab: CarriedTab, index: usize) {
        let index = index.min(self.tabs.len());
        let mut tab = *tab.0;
        tab.bell = false;
        if !self.tabs.is_empty() {
            self.focus_terminal(false);
        }
        self.tabs.insert(index, tab);
        self.active = index;
        self.gesture = Gesture::Idle;
        let layout = self.sync_strip();
        self.strip.hover(&layout, None);
        self.after_tabs_changed(window);
        self.run_find();
        self.focus_terminal(self.focused && !self.banner_focused);
        self.sync_title(window);
    }

    fn slide_tab_to(&mut self, window: &mut Window, id: u64, x: f32) {
        let Some(index) = self.tabs.iter().position(|tab| u64::from(tab.id) == id) else {
            return;
        };
        let layout = self.sync_strip();
        let now = window.frame_time().on_screen();
        let reduce_motion = window.appearance().reduce_motion;
        if let Some(action) = self.strip.slide_to(&layout, index, x, now, reduce_motion) {
            self.strip_action(window, action);
        }
        self.gesture = Gesture::Idle;
    }

    fn insertion_index(&self, x: f32) -> usize {
        self.tab_layout().insertion_index(x)
    }

    fn grab_point(&self, grab: Point) -> Point {
        self.tab_layout().grab_point(grab)
    }
}

impl View for TerminalWindow {
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
        let now = window.frame_time().presentation;
        self.glide(window, now);
        let mut items = Vec::new();
        self.sliding = false;
        if self.has_strip() {
            let layout = self.sync_strip();
            let titles = self.titles();
            let labels: Vec<TabLabel<'_>> = titles
                .iter()
                .zip(&self.tabs)
                .map(|(title, tab)| TabLabel {
                    title,
                    badge: tab.bell,
                })
                .collect();
            let chrome = self.painter.theme.chrome;
            let shown = TabsShown {
                labels: &labels,
                active: self.active,
                focused: self.focused,
                palette: &chrome,
            };
            items.extend(
                self.strip
                    .paint(&layout, &shown, self.painter.text_system(), now),
            );
            self.sliding = self.strip.animating(now);
        }
        let sheet = self.sheet();
        let mut inside = self.sheet_items();
        if let Some((frame, extent)) = self.scroll_extent() {
            let knob = self.painter.theme.palette.ink3;
            self.scrollbar.paint(now, frame, &extent, knob, &mut inside);
        }
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

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match event {
            WindowEvent::Input(Event::Key { key, modifiers }) => self.key(window, key, modifiers),
            WindowEvent::Input(Event::PointerDown(pointer)) => self.press(window, pointer),
            WindowEvent::Input(Event::PointerMove(pointer) | Event::PointerEnter(pointer)) => {
                self.pointer_moved(window, pointer)
            }
            WindowEvent::Input(Event::PointerUp(pointer)) => self.release(window, pointer),
            WindowEvent::Input(Event::PointerLeave) => {
                self.scrollbar_pointer(window, &Event::PointerLeave);
                self.end_gesture(window);
                let layout = self.tab_layout();
                let unhovered = self.strip.hover(&layout, None);
                if self.link.take().is_some() || unhovered {
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
            WindowEvent::Input(Event::Preedit { text, .. }) if !self.banner_focused => {
                self.composing = text;
                window.request_redraw();
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
            WindowEvent::Pasted(text) => self.paste(window, &text),
            WindowEvent::Drag(drag) => self.drag(window, drag),
            WindowEvent::MenuAction(action) => {
                self.menu_action(window, &action);
                return true;
            }
            _ => {}
        }
        self.settle(window);
        true
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.painter.set_appearance(appearance);
        if let Some(banner) = &mut self.banner {
            banner.set_appearance(appearance);
            self.layout();
        }
    }

    fn set_active(&mut self, active: bool) {
        self.focus(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.tabs
            .iter()
            .flat_map(|tab| tab.sessions.iter().map(Session::fd))
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) -> bool {
        let mut remaining = index;
        let Some(tab) = self.tabs.iter().position(|tab| {
            let inside = remaining < tab.sessions.len();
            if !inside {
                remaining -= tab.sessions.len();
            }
            inside
        }) else {
            return false;
        };
        let notices = self.tabs[tab].sessions[remaining].read();
        self.handle_notices(window, tab, notices);
        self.remove_exited(window);
        if self.tabs.is_empty() {
            window.close();
            return true;
        }
        self.sync_title(window);
        tab == self.active || self.has_strip()
    }

    fn animating(&self) -> bool {
        let now = Instant::now();
        self.momentum.is_some()
            || self.sliding
            || self.flash_opacity(now).is_some()
            || self.scrollbar.animating(now)
    }

    fn deadline(&self) -> Option<Instant> {
        let blink = self
            .blinking()
            .then(|| self.blink_from + BLINK * self.blink_phase.saturating_add(1));
        blink.into_iter().chain(self.scrollbar.deadline()).min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) -> bool {
        self.blink_phase = self.blink_phase(now);
        self.scrollbar.tick(now);
        window.request_redraw();
        true
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
}

fn quoted(name: &str) -> String {
    format!("\u{201C}{name}\u{201D}")
}

pub const DIRECTORY_OPTION: &str = "--working-directory";
pub const NEW_WINDOW_OPTION: &str = "--new-window";

#[derive(Debug, Default)]
pub struct Arguments {
    pub command: Option<ShellCommand>,
    pub directory: Option<PathBuf>,
    pub new_window: bool,
}

impl Arguments {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Self {
        let mut parsed = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-e" => {
                    if let Some(program) = args.next() {
                        parsed.command = Some((program, args.collect()));
                    }
                    break;
                }
                DIRECTORY_OPTION => parsed.directory = args.next().map(PathBuf::from),
                NEW_WINDOW_OPTION => parsed.new_window = true,
                _ => {}
            }
        }
        parsed
    }
}

impl Drop for TerminalWindow {
    fn drop(&mut self) {
        self.bus.set_badge(self.serial, 0);
    }
}
