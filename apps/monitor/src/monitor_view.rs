use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use accesskit::{ActionRequest, Node, Role, TreeUpdate};
use bande_toolkit::ui::{
    Animated, BUTTON_HEIGHT, Button, Cell, Column as TableColumn, Cursor, Event, Key, Modifiers,
    Pointer, Row as TableRow, SEGMENTED_HEIGHT, SearchField, SegmentedControl, Sort, Spring,
    TEXT_FIELD_HEIGHT, Table, WidgetId,
};
use bande_toolkit::{
    Appearance, Color, Header, Item, Palette, Path, Point, Radius, Rect, SHEET_INSET, Scene,
    Scheme, Shape, TextAlign, TextRole, TextSize, TextStyle, TextSystem, grown_line,
    window_controls_end,
};

use bande_app_menu::Entry;

use crate::L10N;
use crate::list::{List, Signal, group_id, sender};
use crate::menu::{COLUMN_ACTION, MenuCommand, Selected, app_menus, columns_menu, process_menu};
use crate::model::{
    Column, CoreGroup, CoreKind, Filter, Graph, HISTORY, Monitor, Row, RowKey, Series, StopTarget,
    Tab,
};
use crate::sampler::Snapshot;
use crate::signal::Stop;
use crate::system::PressureLevel;
use crate::view::{
    ALERT_ICON, FORCE_QUIT, INSPECT_SYMBOL, Icons, Painter, QUIT, Request, Screen, answer_stop,
    is_secondary, opens_menu, send_signal, stop_alert, stop_button, symbol_button,
};

pub const TOOLBAR_HEIGHT: f32 = bande_toolkit::WINDOW_HEADER_HEIGHT as f32;
const FOOTER_PAD: f32 = 16.0;
const READOUT_SPACE: f32 = 24.0;
const GRID: f32 = 8.0;
const READOUT_GAP: f32 = 32.0;
const READOUT_ROW: f32 = 24.0;
const READOUT_ROWS: usize = 4;
const GRAPH_SMALLEST_WIDTH: f32 = 120.0;
const GRAPH_CAPTION: f32 = 20.0;
const GRAPH_SCALE_PAD: f32 = 6.0;
const CHARGING_BAND: f32 = 3.0;
const SERIES_WIDTH: f32 = 1.5;
const AREA_OPACITY: f32 = 0.3;
const GRAPH_LABEL: f32 = 14.0;
const STRIP_HEIGHT: f32 = 28.0;
const STRIP_GAP: f32 = 4.0;
const STRIP_MIN_WIDTH: f32 = 200.0;
const STRIP_COLUMNS: usize = 4;
const GROUP_LABEL: f32 = 20.0;
const EDGE_GRAB: f32 = 4.0;
const EDGE_SNAP: f32 = 24.0;
const SIDE: f32 = 12.0;
const LEADING: f32 = window_controls_end(true) + 16.0;
const BUTTON_SIZE: f32 = 28.0;
const ITEM_GAP: f32 = 16.0;
const CLEAR_WIDTH: f32 = 120.0;
const EMPTY_MESSAGE_HEIGHT: f32 = 20.0;
const BUTTON_GAP: f32 = 4.0;
const SEGMENT_FONT: f32 = 13.0;
const SEGMENT_PADDING: f32 = 14.0;
const SEGMENT_MIN_PADDING: f32 = 8.0;
const SEGMENT_INSET: f32 = 2.0;
const SEARCH_WIDTH: f32 = 200.0;
const SEARCH_MIN_WIDTH: f32 = 140.0;
const NAME_WIDTH: f32 = 160.0;
const HEADER_PADDING: f32 = 16.0;
const SORT_MARK: f32 = 12.0;

fn column_width(column: Column) -> f32 {
    match column {
        Column::Name => NAME_WIDTH,
        Column::Cpu | Column::Gpu => 72.0,
        Column::Pid | Column::Threads => 80.0,
        Column::CpuTime => 96.0,
        Column::User => 112.0,
        _ => 112.0,
    }
}

fn table_columns(columns: &[Column], text: &mut TextSystem) -> Vec<TableColumn> {
    let header = TextStyle::role(TextRole::Label, Color::BLACK);
    columns
        .iter()
        .map(|column| {
            let title = column.title();
            let needed = text
                .paragraph(&title.to_uppercase(), header, None, TextAlign::Start)
                .width()
                + HEADER_PADDING
                + SORT_MARK;
            let width = (column_width(*column).max(needed) / 4.0).ceil() * 4.0;
            let shown = TableColumn::new(&title, width);
            match (column, column.numeric()) {
                (Column::Name, _) => shown.flexible(),
                (_, true) => shown.numeric(),
                (_, false) => shown,
            }
        })
        .collect()
}

fn table_sort(columns: &[Column], (column, descending): (Column, bool)) -> Option<Sort> {
    let index = columns.iter().position(|shown| *shown == column)?;
    Some(Sort {
        column: index,
        ascending: !descending,
    })
}

fn row_id(key: &RowKey) -> String {
    match key {
        RowKey::Group(key) => group_id(key),
        RowKey::Process { pid, start_ticks } => format!("process:{pid}:{start_ticks}"),
    }
}

fn table_row(
    icons: &mut Icons,
    row: &Row,
    alarm: Color,
    ids: &mut HashMap<String, RowKey>,
) -> TableRow {
    let id = row_id(&row.key);
    ids.insert(id.clone(), row.key.clone());
    let mut cells = row
        .cells
        .iter()
        .map(|text| Cell::new(text.as_str()))
        .collect::<Vec<_>>();
    if let Some(name) = cells.first_mut() {
        name.icon = row.icon.as_deref().and_then(|icon| icons.get(icon));
        if !row.responding {
            name.text = L10N
                .tr_args("not-responding", [("name", row.name.clone().into())])
                .into();
            name.tint = Some(alarm);
        }
    }
    let children = row
        .children
        .iter()
        .map(|child| table_row(icons, child, alarm, ids))
        .collect();
    TableRow::new(id, cells).children(children)
}

struct Toolbar {
    stop: WidgetId,
    inspect: WidgetId,
    tabs: WidgetId,
    search: WidgetId,
    clear: WidgetId,
}

pub struct MonitorView {
    pub monitor: Monitor,
    palette: Palette,
    icons: Icons,
    list: List,
    toolbar: Toolbar,
    tabs: Vec<Tab>,
    ids: HashMap<String, RowKey>,
    revealed: HashSet<String>,
    size: (f32, f32),
    full_screen: bool,
    focused: bool,
    stopping: Option<StopTarget>,
    requests: Vec<Request>,
    footer: WidgetId,
    readout_widths: Vec<f32>,
    title_width: f32,
    segment_width: f32,
    cores: Animated,
    cores_extent: f32,
    cores_moving: bool,
    edge_drag: Option<f32>,
    edge_hover: bool,
    text_size: TextSize,
}

fn tabs_control(list: &List, tabs: &[Tab], tab: Tab) -> SegmentedControl {
    let send = sender(list.signals());
    let titles = tabs.iter().map(|tab| tab.title()).collect::<Vec<_>>();
    SegmentedControl::new(
        &L10N.tr("tab-label"),
        &titles,
        tabs.iter().position(|shown| *shown == tab),
        move |index| send(Signal::Tab(index)),
    )
    .fill_equally()
}

impl MonitorView {
    pub fn new(monitor: Monitor, palette: Palette, mut text: TextSystem, mut icons: Icons) -> Self {
        let tab = monitor.tab();
        let mut table = Table::new(table_columns(monitor.columns(), &mut text));
        table.set_sort(table_sort(monitor.columns(), monitor.sort()));
        let mut list = List::new(table, palette, text);
        let send = sender(list.signals());
        let stop = list.insert_in_header(stop_button(move || send(Signal::Stop)));
        let send = sender(list.signals());
        let inspect = list.insert_in_header(symbol_button(
            icons.get(INSPECT_SYMBOL),
            &L10N.tr("inspect-label"),
            move || send(Signal::Inspect),
        ));
        let shown_tabs = monitor.tabs();
        let tabs = list.insert_in_header(tabs_control(&list, &shown_tabs, tab));
        let search = list.insert_in_header(search_field(&list, monitor.filter(), ""));
        let send = sender(list.signals());
        let mut clear_button =
            Button::new(&L10N.tr("clear-search"), move || send(Signal::ClearSearch)).quiet();
        clear_button.set_enabled(false);
        let clear = list.insert(clear_button);
        let mut view = Self {
            monitor,
            palette,
            icons,
            list,
            toolbar: Toolbar {
                stop,
                inspect,
                tabs,
                search,
                clear,
            },
            tabs: shown_tabs,
            ids: HashMap::new(),
            revealed: HashSet::new(),
            size: (0.0, 0.0),
            full_screen: false,
            focused: true,
            stopping: None,
            requests: Vec::new(),
            footer: WidgetId::next(),
            readout_widths: Vec::new(),
            title_width: 0.0,
            segment_width: 0.0,
            cores: Animated::points(0.0),
            cores_extent: 0.0,
            cores_moving: false,
            edge_drag: None,
            edge_hover: false,
            text_size: TextSize::DEFAULT,
        };
        view.measure_toolbar();
        view.sync();
        view
    }

    pub fn table(&self) -> Option<&Table> {
        self.list.shown_table()
    }

    fn sync(&mut self) {
        let mut ids = HashMap::new();
        let alarm = self.palette.danger_text;
        let rows = self.monitor.rows();
        let table_rows = rows
            .iter()
            .map(|row| table_row(&mut self.icons, row, alarm, &mut ids))
            .collect();
        let revealed = rows
            .iter()
            .filter(|row| row.revealed)
            .map(|row| row_id(&row.key))
            .collect::<HashSet<_>>();
        let Some(table) = self.list.table() else {
            return;
        };
        table.set_rows(table_rows);
        for id in &revealed {
            if !self.revealed.contains(id) && table.set_row_expanded(id, true) {
                self.revealed.insert(id.clone());
            }
        }
        let hidden = self
            .revealed
            .iter()
            .filter(|id| !revealed.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in hidden {
            table.set_row_expanded(&id, false);
            self.revealed.remove(&id);
        }
        let selection = table.selection();
        self.ids = ids;
        self.drop_selection_signals();
        self.select_ids(&selection);
    }

    fn drop_selection_signals(&mut self) {
        let kept = self
            .list
            .take_signals()
            .into_iter()
            .filter(|signal| !matches!(signal, Signal::Selection(_)))
            .collect::<Vec<_>>();
        self.list.signals().borrow_mut().extend(kept);
    }

    fn select_ids(&mut self, ids: &[String]) {
        let keys = ids
            .iter()
            .filter_map(|id| self.ids.get(id).cloned())
            .collect();
        self.monitor.set_selection(keys);
        let stoppable = self.monitor.stop_target().is_some();
        self.list
            .update::<Button>(self.toolbar.stop, |button, _| button.set_enabled(stoppable));
        let inspectable = self.monitor.selected_pids().len() == 1;
        self.list
            .update::<Button>(self.toolbar.inspect, |button, _| {
                button.set_enabled(inspectable)
            });
    }

    fn drain(&mut self) {
        loop {
            let signals = self.list.take_signals();
            if signals.is_empty() {
                break;
            }
            for signal in signals {
                self.signal(signal);
            }
        }
        let requests = self.list.take_requests();
        self.requests.extend(requests);
    }

    fn signal(&mut self, signal: Signal) {
        match signal {
            Signal::Selection(ids) => self.select_ids(&ids),
            Signal::Activate(_) => self.activate(),
            Signal::Inspect => {
                self.inspect();
            }
            Signal::Sort(sort) => {
                if let Some(column) = self.monitor.columns().get(sort.column) {
                    self.monitor.set_sort(*column, !sort.ascending);
                    self.sync();
                }
            }
            Signal::Tab(index) => {
                if let Some(tab) = self.tabs.get(index) {
                    self.set_tab(*tab);
                }
            }
            Signal::Search(text) => self.set_search(&text),
            Signal::ClearSearch => {
                self.clear_search();
                self.list.focus();
            }
            Signal::Stop => {
                self.open_stop_dialog(false);
            }
            Signal::Choice(choice) => self.chosen(&choice),
        }
    }

    fn chosen(&mut self, choice: &str) {
        let Some(target) = self.stopping.take() else {
            return;
        };
        let icon = self.icons.get(ALERT_ICON);
        let send = sender(self.list.signals());
        let denied = answer_stop(&target, choice, icon, move |choice| {
            send(Signal::Choice(choice.to_owned()))
        });
        if choice == QUIT {
            self.requests
                .extend(target.apps.iter().map(|(app_id, roots)| Request::QuitApp {
                    app_id: app_id.clone(),
                    roots: roots.clone(),
                }));
        }
        if matches!(choice, QUIT | FORCE_QUIT) {
            self.requests.push(Request::Resample);
        }
        if let Some(alert) = denied {
            self.list.present_dialog(alert);
        }
        self.list.focus();
    }

    fn inspect(&mut self) -> bool {
        match self.monitor.selected_pids().as_slice() {
            [pid] => {
                self.requests.push(Request::Inspect(*pid));
                true
            }
            _ => false,
        }
    }

    fn activate(&mut self) {
        let [key] = self.monitor.selection() else {
            return;
        };
        if !matches!(key, RowKey::Group(_)) {
            self.inspect();
            return;
        }
        let id = row_id(key);
        if let Some(table) = self.list.table() {
            let expanded = table.is_row_expanded(&id);
            table.set_row_expanded(&id, !expanded);
        }
    }

    fn inset(&self) -> f32 {
        match self.full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        }
    }

    fn sheet(&self) -> Rect {
        let inset = self.inset();
        Rect::new(
            inset,
            TOOLBAR_HEIGHT,
            (self.size.0 - 2.0 * inset).max(0.0),
            (self.size.1 - TOOLBAR_HEIGHT - inset).max(0.0),
        )
    }

    fn list_frame(&self) -> Rect {
        let sheet = self.sheet();
        let footer = self.footer_layout();
        Rect::new(
            sheet.x,
            sheet.y,
            sheet.width,
            (footer.top - sheet.y).max(0.0),
        )
    }

    fn toolbar_frames(&self) -> [(WidgetId, Rect); 4] {
        let middle = |height: f32| ((TOOLBAR_HEIGHT - height) / 2.0).round();
        let leading = match self.full_screen {
            true => SIDE,
            false => LEADING + self.title_width + ITEM_GAP,
        };
        let count = self.tabs.len() as f32;
        let tabs_for =
            |padding: f32| (self.segment_width + 2.0 * padding) * count + 2.0 * SEGMENT_INSET;
        let fixed = 2.0 * BUTTON_SIZE + BUTTON_GAP + 2.0 * ITEM_GAP + SIDE;
        let room = self.size.0 - leading - fixed;
        let tabs_width = (room - SEARCH_MIN_WIDTH)
            .clamp(tabs_for(SEGMENT_MIN_PADDING), tabs_for(SEGMENT_PADDING));
        let search_width = (room - tabs_width).clamp(SEARCH_MIN_WIDTH, SEARCH_WIDTH);
        let search = Rect::new(
            self.size.0 - SIDE - search_width,
            middle(TEXT_FIELD_HEIGHT),
            search_width,
            TEXT_FIELD_HEIGHT,
        );
        let tabs = Rect::new(
            search.x - ITEM_GAP - tabs_width,
            middle(SEGMENTED_HEIGHT),
            tabs_width,
            SEGMENTED_HEIGHT,
        );
        let floor = match self.full_screen {
            true => SIDE,
            false => LEADING,
        };
        let inspect_x = (tabs.x - ITEM_GAP - BUTTON_SIZE).max(floor);
        let stop_x = (inspect_x - BUTTON_GAP - BUTTON_SIZE).max(floor);
        [
            (
                self.toolbar.stop,
                Rect::new(stop_x, middle(BUTTON_SIZE), BUTTON_SIZE, BUTTON_SIZE),
            ),
            (
                self.toolbar.inspect,
                Rect::new(inspect_x, middle(BUTTON_SIZE), BUTTON_SIZE, BUTTON_SIZE),
            ),
            (self.toolbar.tabs, tabs),
            (self.toolbar.search, search),
        ]
    }

    fn measure_toolbar(&mut self) {
        let palette = self.palette;
        let text = self.list.text();
        let title = text.paragraph(
            &L10N.tr("window-title"),
            TextStyle::role(TextRole::WindowTitle, palette.ink),
            None,
            TextAlign::Start,
        );
        self.title_width = title.width().ceil();
        let segment = TextStyle::new(SEGMENT_FONT, palette.ink).weight(TextStyle::SEMIBOLD);
        let widest = self
            .tabs
            .iter()
            .map(|tab| {
                text.paragraph(&tab.title(), segment, None, TextAlign::Start)
                    .width()
            })
            .fold(0.0, f32::max);
        self.segment_width = widest.ceil();
    }

    fn grown(&self) -> f32 {
        grown_line(self.text_size)
    }

    fn footer_layout(&self) -> Footer {
        footer_layout(
            self.sheet(),
            &self.readout_widths,
            READOUT_ROW + self.grown(),
            self.cores_extent,
        )
    }

    fn animate_cores(&mut self, first: bool) {
        let width = self.sheet().width - 2.0 * FOOTER_PAD;
        let target = match self.monitor.cores_shown() {
            true => core_slots(&self.monitor.core_groups(), Point::new(0.0, 0.0), width).1,
            false => 0.0,
        };
        let (now, reduce_motion) = self.list.motion();
        if first {
            self.cores.jump_to(target);
        }
        self.cores
            .animate_at(now, reduce_motion, target, Spring::STANDARD);
        (self.cores_extent, self.cores_moving) = self.cores.value_at(now);
    }

    fn on_footer_edge(&self, at: Point) -> bool {
        let sheet = self.sheet();
        self.monitor.tab() == Tab::Cpu
            && at.x >= sheet.x
            && at.x < sheet.x + sheet.width
            && (at.y - self.footer_layout().top).abs() <= EDGE_GRAB
    }

    fn drag_edge(&mut self, from: f32, to: f32) {
        let raised = from - to;
        let shown = self.monitor.cores_shown();
        if (raised >= EDGE_SNAP && !shown) || (raised <= -EDGE_SNAP && shown) {
            self.toggle_cores();
            self.edge_drag = Some(to);
        }
    }

    fn toggle_cores(&mut self) -> bool {
        if self.monitor.tab() != Tab::Cpu {
            self.set_tab(Tab::Cpu);
            self.monitor.set_cores_shown(true);
        } else {
            self.monitor.set_cores_shown(!self.monitor.cores_shown());
        }
        self.requests.push(Request::SaveState(self.monitor.state()));
        true
    }

    fn measure_readouts(&mut self) {
        let palette = self.palette;
        let summary = self.monitor.summary();
        let text = self.list.text();
        let mut width = |content: &str, style: TextStyle| {
            text.paragraph(content, style, None, TextAlign::Start)
                .width()
        };
        self.readout_widths = summary
            .iter()
            .map(|column| {
                let labels = column
                    .iter()
                    .map(|readout| width(&readout.label, readout_label(palette)))
                    .fold(0.0, f32::max);
                let values = column
                    .iter()
                    .map(|readout| width(&readout.value, readout_value(palette)))
                    .fold(0.0, f32::max);
                ((labels + values + READOUT_SPACE) / GRID).ceil() * GRID
            })
            .collect();
    }

    fn layout(&mut self) {
        self.measure_readouts();
        for (id, frame) in self.toolbar_frames() {
            self.list.place(id, frame);
        }
        self.list.set_frame(self.list_frame());
        let nothing_found = self.nothing_found();
        let message = self.empty_message_frame();
        let clear_width = (CLEAR_WIDTH * self.text_size.scale()).round();
        let clear = match nothing_found {
            true => Rect::new(
                (message.x + (message.width - clear_width) / 2.0).round(),
                message.y + message.height + ITEM_GAP / 2.0,
                clear_width,
                BUTTON_HEIGHT + self.grown(),
            ),
            false => Rect::new(0.0, 0.0, 0.0, 0.0),
        };
        self.list.place(self.toolbar.clear, clear);
        self.list.update::<Button>(self.toolbar.clear, |button, _| {
            button.set_enabled(nothing_found)
        });
    }

    fn nothing_found(&self) -> bool {
        !self.monitor.search().is_empty() && self.monitor.rows().is_empty()
    }

    fn empty_message_frame(&self) -> Rect {
        let list = self.list_frame();
        let body_top = list.y + self.list.header_height();
        let middle = body_top + ((list.y + list.height - body_top) / 2.0).round();
        Rect::new(
            list.x + SIDE,
            middle - EMPTY_MESSAGE_HEIGHT - self.grown() - ITEM_GAP / 2.0,
            (list.width - 2.0 * SIDE).max(0.0),
            EMPTY_MESSAGE_HEIGHT + self.grown(),
        )
    }

    fn set_tab(&mut self, tab: Tab) {
        if let Some(index) = self.tabs.iter().position(|shown| *shown == tab) {
            self.list
                .update::<SegmentedControl>(self.toolbar.tabs, |control, cx| {
                    control.set_selected(cx, index)
                });
        }
        if tab == self.monitor.tab() {
            return;
        }
        let measured = self.measures();
        self.monitor.set_tab(tab);
        if self.measures() != measured {
            self.requests.push(Request::Resample);
        }
        self.show_columns();
    }

    fn measures(&self) -> [bool; 5] {
        [
            self.monitor.measures_network(),
            self.monitor.measures_footprint(),
            self.monitor.measures_gpu(),
            self.monitor.measures_wakeups(),
            self.monitor.measures_disk(),
        ]
    }

    fn show_columns(&mut self) {
        let columns = table_columns(self.monitor.columns(), self.list.text());
        let sort = table_sort(self.monitor.columns(), self.monitor.sort());
        if let Some(table) = self.list.table() {
            table.set_columns(columns);
            table.set_sort(sort);
        }
        self.sync();
    }

    fn toggle_column(&mut self, id: &str) -> bool {
        let Some(column) = Column::parse(id) else {
            return false;
        };
        let measured = self.measures();
        if !self.monitor.toggle_column(column) {
            return false;
        }
        if self.measures() != measured {
            self.requests.push(Request::Resample);
        }
        self.show_columns();
        self.requests.push(Request::SaveState(self.monitor.state()));
        true
    }

    fn open_stop_dialog(&mut self, force: bool) -> bool {
        let Some(target) = self.monitor.stop_target() else {
            return false;
        };
        let icon = self.icons.get(ALERT_ICON);
        let send = sender(self.list.signals());
        let alert = stop_alert(&target, force, icon, move |choice| {
            send(Signal::Choice(choice.to_owned()))
        });
        self.stopping = Some(target);
        self.list.present_dialog(alert);
        true
    }

    fn send_signal(&mut self, how: Stop) -> bool {
        let Some(target) = self.monitor.stop_target() else {
            return false;
        };
        let icon = self.icons.get(ALERT_ICON);
        let send = sender(self.list.signals());
        let denied = send_signal(&target, how, icon, move |choice| {
            send(Signal::Choice(choice.to_owned()))
        });
        self.requests.push(Request::Resample);
        if let Some(alert) = denied {
            self.list.present_dialog(alert);
        }
        true
    }

    fn show_in_files(&mut self) -> bool {
        match self.monitor.location() {
            Some(path) => {
                self.requests.push(Request::ShowInFiles(path));
                true
            }
            None => false,
        }
    }

    fn request_menu(&mut self, at: Point) {
        let selected = Selected {
            stoppable: self.monitor.stop_target().is_some(),
            located: self.monitor.location().is_some(),
            has_pid: !self.monitor.selected_pids().is_empty(),
            inspectable: self.monitor.selected_pids().len() == 1,
        };
        self.requests
            .push(Request::Menu(process_menu(selected), at));
    }

    fn menu_from_keyboard(&mut self) -> bool {
        let bounds = self
            .list
            .shown_table()
            .and_then(|table| table.row_bounds(table.cursor_row()?));
        match bounds {
            Some(row) if !self.monitor.selection().is_empty() => {
                self.request_menu(Point::new(row.x + 40.0, row.y + row.height));
                true
            }
            _ => false,
        }
    }

    fn searching(&self) -> bool {
        self.list.focused() == Some(self.toolbar.search)
    }

    fn set_search(&mut self, search: &str) {
        self.monitor.set_search(search);
        self.sync();
    }

    fn clear_search(&mut self) {
        let field = search_field(&self.list, self.monitor.filter(), "");
        self.toolbar.search = self.list.replace(self.toolbar.search, field);
        self.set_search("");
    }

    fn set_filter(&mut self, filter: Filter) -> bool {
        if filter == Filter::Selected && self.monitor.selection().is_empty() {
            return false;
        }
        self.monitor.set_filter(filter);
        let focused = self.searching();
        let field = search_field(&self.list, filter, self.monitor.search());
        self.toolbar.search = self.list.replace(self.toolbar.search, field);
        if focused {
            self.list.focus_widget(self.toolbar.search);
        }
        self.sync();
        self.requests.push(Request::SaveState(self.monitor.state()));
        true
    }

    fn search_key(&mut self, key: Key, modifiers: Modifiers) -> Option<bool> {
        if !self.searching() || modifiers.logo || modifiers.control {
            return None;
        }
        match key {
            Key::Escape if self.monitor.search().is_empty() => self.list.focus(),
            Key::Escape => {
                self.clear_search();
                self.list.focus_widget(self.toolbar.search);
            }
            Key::Enter | Key::Down => {
                self.list.focus();
                if self.monitor.selection().is_empty() {
                    self.select_first();
                }
            }
            _ => return None,
        }
        Some(true)
    }

    fn select_first(&mut self) {
        let Some(first) = self.monitor.rows().first().map(|row| row_id(&row.key)) else {
            return;
        };
        if let Some(table) = self.list.table() {
            table.select(std::slice::from_ref(&first));
            table.scroll_to_row(0);
        }
        self.select_ids(&[first]);
    }

    fn key(&mut self, event: &Event, key: Key, modifiers: Modifiers) -> bool {
        if self.list.has_dialog() {
            return self.pass(event);
        }
        if let Some(handled) = self.search_key(key, modifiers) {
            return handled;
        }
        if opens_menu(key, modifiers) {
            return self.menu_from_keyboard();
        }
        match key {
            Key::Character(digit @ '1'..='9') if modifiers.control => {
                let index = digit as usize - '1' as usize;
                match self.tabs.get(index).copied() {
                    Some(tab) => {
                        self.set_tab(tab);
                        true
                    }
                    None => false,
                }
            }
            Key::Character('f') if modifiers.control => {
                self.list.focus_widget(self.toolbar.search);
                true
            }
            Key::Character('e') if modifiers.control => self.open_stop_dialog(false),
            Key::Character('r' | 'R') if modifiers.control && modifiers.shift => {
                self.show_in_files()
            }
            Key::Character('i') if modifiers.control => self.inspect(),
            Key::Escape if !self.monitor.search().is_empty() => {
                self.clear_search();
                true
            }
            Key::Escape if self.monitor.cores_shown() => self.toggle_cores(),
            _ => self.pass(event),
        }
    }

    fn pointer_down(&mut self, event: &Event, pointer: &Pointer) -> bool {
        let at = pointer.position;
        if !self.list.has_dialog() && self.on_footer_edge(at) {
            self.edge_drag = Some(at.y);
            return true;
        }
        let on_graph = self
            .footer_layout()
            .graph
            .is_some_and(|graph| graph.contains(at));
        if on_graph && !self.list.has_dialog() && self.monitor.tab() == Tab::Cpu {
            return self.toggle_cores();
        }
        let handled = self.pass(event);
        if self.list.has_dialog() || !self.list.frame().contains(at) {
            return handled;
        }
        let header = at.y < self.list.frame().y + self.list.header_height();
        if is_secondary(pointer) && header && self.monitor.tab().has_column_choice() {
            let menu = columns_menu(self.monitor.columns());
            self.requests.push(Request::Menu(menu, at));
            return true;
        }
        if is_secondary(pointer) && !header && !self.monitor.selection().is_empty() {
            self.request_menu(at);
            return true;
        }
        handled
    }

    fn pass(&mut self, event: &Event) -> bool {
        let handled = self.list.handle(event);
        self.drain();
        handled
    }
}

fn series_points(values: &[f32], plot: Rect) -> Vec<Point> {
    let step = plot.width / (HISTORY - 1) as f32;
    let right = plot.x + plot.width;
    let bottom = plot.y + plot.height;
    let first = values.len().saturating_sub(HISTORY);
    let shown = &values[first..];
    shown
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Point::new(
                right - (shown.len() - 1 - index) as f32 * step,
                bottom - value.clamp(0.0, 1.0) * plot.height,
            )
        })
        .collect()
}

fn polyline(points: &[Point]) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    Some(
        rest.iter()
            .fold(Path::new().move_to(*first), |path, point| {
                path.line_to(*point)
            }),
    )
}

fn series_color(series: Series, palette: Palette) -> Color {
    let (lightness, chroma) = match palette.scheme {
        Scheme::Light => (0.56, 0.18),
        Scheme::Dark => (0.72, 0.14),
    };
    match series {
        Series::Primary => Color::oklch(lightness, chroma, 255.0),
        Series::Secondary => Color::oklch(lightness, chroma + 0.02, 25.0),
    }
}

fn paint_area(painter: &mut Painter, points: &[Point], bottom: f32, color: Color) {
    let (Some(path), Some(first), Some(last)) = (polyline(points), points.first(), points.last())
    else {
        return;
    };
    painter.items.push(Item::FillPath {
        path: path
            .clone()
            .line_to(Point::new(last.x, bottom))
            .line_to(Point::new(first.x, bottom))
            .close(),
        paint: color.with_opacity(AREA_OPACITY).into(),
    });
    painter.items.push(Item::StrokePath {
        path,
        width: SERIES_WIDTH,
        paint: color.into(),
    });
}

fn paint_series(painter: &mut Painter, values: &[f32], plot: Rect, color: Color) {
    let points = series_points(values, plot);
    paint_area(painter, &points, plot.y + plot.height, color);
}

fn paint_battery(
    painter: &mut Painter,
    charge: &[(f32, f32)],
    charging: &[(f32, f32)],
    plot: Rect,
    palette: Palette,
) {
    let bottom = plot.y + plot.height;
    let points: Vec<Point> = charge
        .iter()
        .map(|(along, level)| {
            Point::new(
                plot.x + along * plot.width,
                bottom - level.clamp(0.0, 1.0) * plot.height,
            )
        })
        .collect();
    paint_area(
        painter,
        &points,
        bottom,
        series_color(Series::Primary, palette),
    );
    for (from, to) in charging {
        painter.fill(
            Shape::rect(Rect::new(
                plot.x + from * plot.width,
                bottom - CHARGING_BAND,
                (to - from) * plot.width,
                CHARGING_BAND,
            )),
            palette.success,
        );
    }
}

fn pressure_color(level: PressureLevel, palette: Palette) -> Color {
    match level {
        PressureLevel::Normal => palette.success,
        PressureLevel::Elevated => palette.warning,
        PressureLevel::Critical => palette.danger_text,
    }
}

fn paint_pressure(
    painter: &mut Painter,
    samples: &[(f32, PressureLevel)],
    plot: Rect,
    palette: Palette,
) {
    let values = samples.iter().map(|(used, _)| *used).collect::<Vec<_>>();
    let points = series_points(&values, plot);
    let first = samples.len().saturating_sub(HISTORY);
    let levels = &samples[first..];
    let bottom = plot.y + plot.height;
    let mut start = 0;
    while start < levels.len() {
        let level = levels[start].1;
        let end = levels[start..]
            .iter()
            .position(|(_, other)| *other != level)
            .map_or(levels.len(), |offset| start + offset);
        let run = &points[start..(end + 1).min(points.len())];
        let run = match run.len() {
            1 => vec![Point::new(run[0].x - 1.0, run[0].y), run[0]],
            _ => run.to_vec(),
        };
        paint_area(painter, &run, bottom, pressure_color(level, palette));
        start = end;
    }
}

fn graph_title(graph: &Graph, tab: Tab) -> String {
    match graph {
        Graph::Pressure(_) => L10N.tr("graph-pressure"),
        Graph::Battery { .. } => L10N.tr("graph-battery"),
        Graph::Stacked { .. } | Graph::Lines { .. } => L10N.tr(match tab {
            Tab::Cpu => "graph-cpu-title",
            Tab::Energy => "graph-energy-title",
            Tab::Disk => "graph-disk-title",
            Tab::Network => "graph-network-title",
            Tab::Memory | Tab::Gpu => "graph-gpu-title",
        }),
    }
}

fn paint_well(painter: &mut Painter, frame: Rect, palette: Palette) -> Shape {
    let well = Shape::rounded(frame, Radius::S);
    let background = match palette.scheme {
        Scheme::Light => palette.well.mix(palette.sheet, 0.55),
        Scheme::Dark => palette.well.mix(palette.chassis, 0.5),
    };
    painter.fill(well.clone(), background);
    for quarter in 1..4 {
        let y = (frame.y + frame.height * quarter as f32 / 4.0).round();
        painter.fill(
            Shape::rect(Rect::new(frame.x, y, frame.width, 1.0)),
            palette.hairline.with_opacity(0.6),
        );
    }
    well
}

fn paint_graph(monitor: &Monitor, palette: Palette, painter: &mut Painter, area: Rect) {
    let graph_kind = monitor.graph();
    let caption = Rect::new(area.x, area.y, area.width, GRAPH_CAPTION);
    let label = readout_label(palette);
    painter.text_in(
        &graph_title(&graph_kind, monitor.tab()),
        label,
        caption,
        TextAlign::Center,
    );
    let span = match graph_kind {
        Graph::Battery { .. } => L10N.tr("graph-battery-span"),
        _ => monitor.span(),
    };
    painter.text_in(&span, label, caption, TextAlign::End);
    let frame = Rect::new(
        area.x,
        area.y + GRAPH_CAPTION,
        area.width,
        (area.height - GRAPH_CAPTION).max(1.0),
    );
    let mut graph = Painter {
        text: &mut *painter.text,
        items: Vec::new(),
    };
    let well = paint_well(&mut graph, frame, palette);
    let plot = Rect::new(
        frame.x,
        frame.y + SERIES_WIDTH,
        frame.width,
        (frame.height - SERIES_WIDTH).max(1.0),
    );
    let scale = match graph_kind {
        Graph::Stacked { lower, upper } => {
            paint_series(
                &mut graph,
                &upper,
                plot,
                series_color(Series::Primary, palette),
            );
            paint_series(
                &mut graph,
                &lower,
                plot,
                series_color(Series::Secondary, palette),
            );
            None
        }
        Graph::Lines {
            primary,
            secondary,
            scale,
        } => {
            paint_series(
                &mut graph,
                &primary,
                plot,
                series_color(Series::Primary, palette),
            );
            if let Some(secondary) = &secondary {
                paint_series(
                    &mut graph,
                    secondary,
                    plot,
                    series_color(Series::Secondary, palette),
                );
            }
            Some(scale)
        }
        Graph::Pressure(samples) => {
            paint_pressure(&mut graph, &samples, plot, palette);
            None
        }
        Graph::Battery { charge, charging } => {
            paint_battery(&mut graph, &charge, &charging, plot, palette);
            None
        }
    };
    if let Some(scale) = scale {
        graph.text_in(
            &scale,
            TextStyle {
                caps: false,
                ..label
            }
            .tabular(),
            Rect::new(
                frame.x + GRAPH_SCALE_PAD,
                frame.y + GRAPH_SCALE_PAD / 2.0,
                frame.width - 2.0 * GRAPH_SCALE_PAD,
                GRAPH_CAPTION - GRAPH_SCALE_PAD,
            ),
            TextAlign::Start,
        );
    }
    let items = std::mem::take(&mut graph.items);
    painter.items.push(Item::Clip { shape: well, items });
    painter.items.push(Item::Stroke {
        shape: Shape::rounded(frame.inset(0.5), Radius::S - 0.5),
        width: 1.0,
        paint: palette.hairline.into(),
    });
}

#[derive(Debug, Clone, PartialEq)]
struct Footer {
    sheet: Rect,
    top: f32,
    row: f32,
    columns: Vec<Rect>,
    graph: Option<Rect>,
    cores: Rect,
}

fn readout_label(palette: Palette) -> TextStyle {
    TextStyle::role(TextRole::Label, palette.ink3)
}

fn readout_value(palette: Palette) -> TextStyle {
    TextStyle::role(TextRole::BodyStrong, palette.ink).tabular()
}

fn footer_layout(sheet: Rect, widths: &[f32], row: f32, cores: f32) -> Footer {
    let available = sheet.width - 2.0 * FOOTER_PAD;
    let height = READOUT_ROWS as f32 * row + 2.0 * FOOTER_PAD;
    let top = sheet.y + sheet.height - height - cores;
    let content = Rect::new(
        sheet.x + FOOTER_PAD,
        top + FOOTER_PAD,
        available,
        height - 2.0 * FOOTER_PAD,
    );
    let mut used = 0.0;
    let fitting = widths
        .iter()
        .take_while(|width| {
            used += *width + READOUT_GAP;
            used - READOUT_GAP <= available
        })
        .count();
    let shown = &widths[..fitting];
    let trailing = shown
        .iter()
        .skip(1)
        .map(|width| width + READOUT_GAP)
        .sum::<f32>();
    let leading = shown.first().map_or(0.0, |width| width + READOUT_GAP);
    let graph_width = available - leading - trailing;
    let graph = (graph_width >= GRAPH_SMALLEST_WIDTH)
        .then(|| Rect::new(content.x + leading, content.y, graph_width, content.height));
    let mut x = content.x;
    let mut columns = Vec::new();
    for (index, width) in shown.iter().enumerate() {
        if index == 1 && graph.is_some() {
            x = content.x + available - trailing + READOUT_GAP;
        }
        columns.push(Rect::new(x, content.y, *width, content.height));
        x += width + READOUT_GAP;
    }
    Footer {
        sheet,
        top,
        row,
        columns,
        graph,
        cores: Rect::new(
            sheet.x + FOOTER_PAD,
            top + height,
            sheet.width - 2.0 * FOOTER_PAD,
            cores,
        ),
    }
}

fn paint_footer(monitor: &Monitor, palette: Palette, footer: &Footer, painter: &mut Painter) {
    let sheet = footer.sheet;
    painter.fill(
        Shape::rect(Rect::new(sheet.x, footer.top, sheet.width, 1.0)),
        palette.hairline,
    );
    for (column, frame) in monitor.summary().iter().zip(&footer.columns) {
        for (index, readout) in column.iter().enumerate() {
            let row = Rect::new(
                frame.x,
                frame.y + index as f32 * footer.row,
                frame.width,
                footer.row,
            );
            let value = match readout.series {
                Some(series) => TextStyle {
                    color: series_color(series, palette),
                    ..readout_value(palette)
                },
                None => readout_value(palette),
            };
            painter.text_in(
                &readout.label,
                readout_label(palette),
                row,
                TextAlign::Start,
            );
            painter.text_in(&readout.value, value, row, TextAlign::End);
        }
    }
    if let Some(graph) = footer.graph {
        paint_graph(monitor, palette, painter, graph);
    }
    if footer.cores.height > 0.0 {
        paint_cores(monitor, palette, footer.cores, painter);
    }
}

enum CoreSlot {
    Label(String, Rect),
    Strip(usize, Vec<f32>, Rect),
}

fn core_slots(groups: &[CoreGroup], origin: Point, width: f32) -> (Vec<CoreSlot>, f32) {
    let largest = groups
        .iter()
        .map(|group| group.cores.len())
        .max()
        .unwrap_or(1);
    let columns = (((width + STRIP_GAP) / (STRIP_MIN_WIDTH + STRIP_GAP)) as usize)
        .clamp(1, STRIP_COLUMNS)
        .min(largest.max(1));
    let strip_width = (width - (columns - 1) as f32 * STRIP_GAP) / columns as f32;
    let mut slots = Vec::new();
    let mut y = origin.y;
    for group in groups {
        if let Some(kind) = group.kind {
            let title = match kind {
                CoreKind::Performance => L10N.tr("cores-performance"),
                CoreKind::Efficiency => L10N.tr("cores-efficiency"),
            };
            slots.push(CoreSlot::Label(
                title,
                Rect::new(origin.x, y, width, GROUP_LABEL),
            ));
            y += GROUP_LABEL;
        }
        for (index, (core, history)) in group.cores.iter().enumerate() {
            let column = index % columns;
            let row = index / columns;
            slots.push(CoreSlot::Strip(
                *core,
                history.clone(),
                Rect::new(
                    origin.x + column as f32 * (strip_width + STRIP_GAP),
                    y + row as f32 * (STRIP_HEIGHT + STRIP_GAP),
                    strip_width,
                    STRIP_HEIGHT,
                ),
            ));
        }
        y += group.cores.len().div_ceil(columns) as f32 * (STRIP_HEIGHT + STRIP_GAP);
    }
    (slots, y - origin.y + FOOTER_PAD - STRIP_GAP)
}

fn paint_cores(monitor: &Monitor, palette: Palette, area: Rect, painter: &mut Painter) {
    let (slots, _) = core_slots(
        &monitor.core_groups(),
        Point::new(area.x, area.y),
        area.width,
    );
    for slot in slots {
        match slot {
            CoreSlot::Label(title, frame) => {
                painter.text_in(&title, readout_label(palette), frame, TextAlign::Start)
            }
            CoreSlot::Strip(core, history, frame) => {
                let mut strip = Painter {
                    text: &mut *painter.text,
                    items: Vec::new(),
                };
                let well = paint_well(&mut strip, frame, palette);
                paint_series(
                    &mut strip,
                    &history,
                    frame,
                    series_color(Series::Primary, palette),
                );
                strip.text_in(
                    &L10N.tr_args("core-label", [("number", (core + 1).into())]),
                    readout_label(palette),
                    Rect::new(
                        frame.x + GRAPH_SCALE_PAD,
                        frame.y + 2.0,
                        frame.width - 2.0 * GRAPH_SCALE_PAD,
                        GRAPH_LABEL,
                    ),
                    TextAlign::Start,
                );
                let items = std::mem::take(&mut strip.items);
                painter.items.push(Item::Clip { shape: well, items });
            }
        }
    }
}

fn search_field(list: &List, filter: Filter, text: &str) -> SearchField {
    let send = sender(list.signals());
    let placeholder = match filter {
        Filter::All | Filter::Hierarchy => L10N.tr("search-placeholder"),
        filter => L10N.tr_args("search-filtered", [("filter", filter.title().into())]),
    };
    SearchField::new(&placeholder, text, move |text| {
        send(Signal::Search(text.to_owned()))
    })
}

impl Screen for MonitorView {
    fn app_menus(&self) -> Option<Vec<Entry>> {
        Some(app_menus(
            &self.monitor.tabs(),
            self.monitor.filter(),
            self.monitor.frequency(),
        ))
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.text_size = appearance.text_size;
        self.list.set_appearance(appearance);
        self.measure_toolbar();
        self.show_columns();
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        match focused {
            true if self.list.focused().is_none() => self.list.focus(),
            true => {}
            false => self.list.blur(),
        }
    }

    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    fn menu_action(&mut self, action: &str) -> bool {
        if self.list.menu_action(action) {
            self.drain();
            return true;
        }
        if let Some(id) = action.strip_prefix(COLUMN_ACTION) {
            return self.toggle_column(id);
        }
        match MenuCommand::parse(action) {
            Some(MenuCommand::Quit) => self.open_stop_dialog(false),
            Some(MenuCommand::ForceQuit) => self.open_stop_dialog(true),
            Some(MenuCommand::ShowInFiles) => self.show_in_files(),
            Some(MenuCommand::Inspect) => self.inspect(),
            Some(MenuCommand::CopyName) => {
                let names = self.monitor.selected_names();
                if !names.is_empty() {
                    self.requests.push(Request::Copy(names.join("\n")));
                }
                false
            }
            Some(MenuCommand::CopyPid) => {
                let pids = self.monitor.selected_pids();
                if !pids.is_empty() {
                    let pids = pids.iter().map(u32::to_string).collect::<Vec<_>>();
                    self.requests.push(Request::Copy(pids.join("\n")));
                }
                false
            }
            Some(MenuCommand::Find) => {
                self.list.focus_widget(self.toolbar.search);
                true
            }
            Some(MenuCommand::Show(tab)) => {
                self.set_tab(tab);
                true
            }
            Some(MenuCommand::ShowCores) => self.toggle_cores(),
            Some(MenuCommand::Filter(filter)) => self.set_filter(filter),
            Some(MenuCommand::Signal(how)) => self.send_signal(how),
            Some(MenuCommand::Frequency(frequency)) => {
                self.monitor.set_frequency(frequency);
                self.requests.push(Request::SaveState(self.monitor.state()));
                self.requests.push(Request::Resample);
                true
            }
            None => false,
        }
    }

    fn menu_closed(&mut self) -> bool {
        self.list.menu_closed()
    }

    fn pasted(&mut self, text: &str) -> bool {
        let pasted = self.list.pasted(text);
        self.drain();
        pasted
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } => self.key(event, *key, *modifiers),
            Event::PointerDown(pointer) => self.pointer_down(event, pointer),
            Event::PointerMove(pointer) => match self.edge_drag {
                Some(from) => {
                    self.drag_edge(from, pointer.position.y);
                    true
                }
                None => {
                    self.edge_hover = self.on_footer_edge(pointer.position);
                    self.pass(event)
                }
            },
            Event::PointerUp(_) if self.edge_drag.take().is_some() => true,
            Event::PointerLeave => {
                let dragged = self.edge_drag.take().is_some();
                self.edge_hover = false;
                self.pass(event) | dragged
            }
            _ => self.pass(event),
        }
    }

    fn scene(&mut self, width: u32, height: u32) -> Scene {
        let first = self.size == (0.0, 0.0);
        self.size = (width as f32, height as f32);
        self.animate_cores(first);
        self.layout();
        let (controls, dialog) = self.list.paint(width, height);
        let sheet = self.sheet();
        let radius = Radius::concentric(Radius::XL, self.inset());
        let footer = self.footer_layout();
        let nothing_found = self.nothing_found();
        let empty_message = self.empty_message_frame();
        let (monitor, palette) = (&self.monitor, self.palette);
        let mut painter = Painter {
            text: self.list.text(),
            items: Vec::new(),
        };
        paint_footer(monitor, palette, &footer, &mut painter);
        if nothing_found {
            painter.text_in(
                &L10N.tr_args("no-match", [("text", monitor.search().to_owned().into())]),
                TextStyle::role(TextRole::Body, palette.ink3),
                empty_message,
                TextAlign::Center,
            );
        }
        painter.items = vec![Item::Clip {
            shape: Shape::rounded(sheet, radius),
            items: std::mem::take(&mut painter.items),
        }];
        painter.items.extend(controls);
        painter.items.extend(dialog);
        Scene {
            width,
            height,
            background: self.palette.sheet,
            items: painter.items,
        }
    }

    fn update(&mut self, snapshot: Snapshot) {
        self.monitor.update(snapshot);
        let tabs = self.monitor.tabs();
        if tabs != self.tabs {
            let control = tabs_control(&self.list, &tabs, self.monitor.tab());
            self.toolbar.tabs = self.list.replace(self.toolbar.tabs, control);
            self.tabs = tabs;
            self.measure_toolbar();
        }
        self.sync();
    }

    fn measures_gpu(&self) -> bool {
        self.monitor.measures_gpu()
    }

    fn interval(&self) -> Option<Duration> {
        Some(self.monitor.frequency().interval())
    }

    fn measures_disk(&self) -> bool {
        self.monitor.measures_disk()
    }

    fn measures_network(&self) -> bool {
        self.monitor.measures_network()
    }

    fn measures_footprint(&self) -> bool {
        self.monitor.measures_footprint()
    }

    fn measures_wakeups(&self) -> bool {
        self.monitor.measures_wakeups()
    }

    fn measures_sleep_blockers(&self) -> bool {
        self.monitor.measures_sleep_blockers()
    }

    fn measures_battery(&self) -> bool {
        self.monitor.measures_battery()
    }

    fn finished(&self) -> bool {
        false
    }

    fn set_responding(&mut self, pid: u32, app_id: &str, responding: bool) -> bool {
        let changed = self.monitor.set_responding(pid, app_id, responding);
        if changed {
            self.sync();
        }
        changed
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        let mut tree = self.list.accessibility();
        let group = self.footer.part(0);
        let mut footer = Node::new(Role::Group);
        footer.set_label(self.monitor.tab().title());
        let mut nodes = Vec::new();
        let readouts = self.monitor.summary().into_iter().flatten();
        for (index, shown) in readouts.enumerate() {
            let mut readout = Node::new(Role::Label);
            readout.set_label(shown.label);
            readout.set_value(shown.value);
            nodes.push((self.footer.part(index as u64 + 2), readout));
        }
        let mut graph = Node::new(Role::Image);
        graph.set_label(self.monitor.graph_description());
        nodes.push((self.footer.part(1), graph));
        footer.set_children(nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        if let Some((_, root)) = tree.nodes.first_mut() {
            root.push_child(group);
        }
        tree.nodes.push((group, footer));
        tree.nodes.extend(nodes);
        Some(tree)
    }

    fn perform(&mut self, request: &ActionRequest) -> bool {
        let performed = self.list.perform(request);
        self.drain();
        performed
    }

    fn cursor(&self) -> Cursor {
        match self.edge_drag.is_some() || self.edge_hover {
            true => Cursor::RowResize,
            false => self.list.cursor(),
        }
    }

    fn animating(&self) -> bool {
        self.list.animating() || self.cores_moving
    }

    fn deadline(&self) -> Option<Instant> {
        self.list.deadline()
    }

    fn header(&self) -> Option<Header> {
        let items = self
            .toolbar_frames()
            .into_iter()
            .map(|(_, frame)| frame)
            .collect();
        Some(Header {
            title_x: LEADING,
            items,
            tabs: Vec::new(),
        })
    }

    fn set_full_screen(&mut self, full_screen: bool) {
        self.full_screen = full_screen;
    }
}
