use std::collections::{HashMap, HashSet};
use std::time::Instant;

use accesskit::{ActionRequest, Node, Role, TreeUpdate};
use unbound_toolkit::ui::{
    Animated, Button, Cell, Column as TableColumn, Cursor, Event, HEADER_HEIGHT, Key, Modifiers,
    Pointer, Row as TableRow, SEGMENTED_HEIGHT, SearchField, SegmentedControl, Sort, Spring,
    TEXT_FIELD_HEIGHT, Table, WidgetId,
};
use unbound_toolkit::{
    Color, Header, Item, Palette, Path, Point, Radius, Rect, SHEET_INSET, Scene, Shape, TextAlign,
    TextRole, TextStyle, TextSystem,
};

use crate::L10N;
use crate::list::{List, Signal, group_id, sender};
use crate::menu::{COLUMN_ACTION, MenuCommand, Selected, columns_menu, process_menu};
use crate::model::{
    Column, CoreGroup, CoreKind, Filter, Graph, HISTORY, Monitor, Row, RowKey, StopTarget, Tab,
};
use crate::sampler::Snapshot;
use crate::system::PressureLevel;
use crate::view::{
    ALERT_ICON, FORCE_QUIT, INSPECT_SYMBOL, Icons, Painter, QUIT, Request, Screen, answer_stop,
    contains, is_secondary, opens_menu, stop_alert, stop_button, symbol_button,
};

pub const TOOLBAR_HEIGHT: f32 = unbound_toolkit::HEADER_HEIGHT as f32;
const FOOTER_HEIGHT: f32 = 112.0;
const FOOTER_PAD: f32 = 16.0;
const READOUT_WIDTH: f32 = 176.0;
const READOUT_SPACE: f32 = 24.0;
const GRID: f32 = 8.0;
const READOUT_GAP: f32 = 24.0;
const READOUT_ROW: f32 = 24.0;
const READOUT_ROWS: usize = 3;
const READOUT_COLUMNS: usize = 3;
const GRAPH_MIN_WIDTH: f32 = 160.0;
const GRAPH_SMALLEST_WIDTH: f32 = 96.0;
const GRAPH_PAD: f32 = 6.0;
const GRAPH_LABEL: f32 = 14.0;
const SERIES_WIDTH: f32 = 1.5;
const BAR_SLOT: usize = 2;
const STRIP_HEIGHT: f32 = 28.0;
const STRIP_GAP: f32 = 4.0;
const STRIP_MIN_WIDTH: f32 = 200.0;
const STRIP_COLUMNS: usize = 4;
const GROUP_LABEL: f32 = 20.0;
const BAR_HEIGHT: f32 = 8.0;
const KEY_SIZE: f32 = 8.0;
const KEY_RADIUS: f32 = 2.0;
const KEY_GAP: f32 = 6.0;
const SIDE: f32 = 12.0;
const LEADING: f32 = 94.0;
const BUTTON_SIZE: f32 = 28.0;
const ITEM_GAP: f32 = 16.0;
const BUTTON_GAP: f32 = 4.0;
const SEGMENT_WIDTH: f32 = 80.0;
const SEARCH_WIDTH: f32 = 200.0;
const NAME_WIDTH: f32 = 160.0;

fn column_width(column: Column) -> f32 {
    match column {
        Column::Name => NAME_WIDTH,
        Column::Cpu => 72.0,
        Column::Pid => 64.0,
        Column::Threads => 84.0,
        Column::CpuTime => 84.0,
        Column::User => 96.0,
        _ => 108.0,
    }
}

fn table_columns(tab: Tab, columns: &[Column]) -> Vec<TableColumn> {
    columns
        .iter()
        .map(|column| {
            let shown = TableColumn::new(&column.title(tab), column_width(*column));
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
        RowKey::Interface(name) => format!("interface:{name}"),
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
            name.text = L10N.tr_args("not-responding", [("name", row.name.clone().into())]);
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
    readout_width: f32,
    cores: Animated,
    cores_extent: f32,
    cores_moving: bool,
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
}

impl MonitorView {
    pub fn new(monitor: Monitor, palette: Palette, text: TextSystem, mut icons: Icons) -> Self {
        let tab = monitor.tab();
        let mut table = Table::new(table_columns(tab, monitor.columns()));
        table.set_sort(table_sort(monitor.columns(), monitor.sort()));
        let mut list = List::new(table, palette, text);
        let send = sender(list.signals());
        let stop = list.insert(stop_button(move || send(Signal::Stop)));
        let send = sender(list.signals());
        let inspect = list.insert(symbol_button(
            icons.get(INSPECT_SYMBOL),
            &L10N.tr("inspect-label"),
            move || send(Signal::Inspect),
        ));
        let shown_tabs = monitor.tabs();
        let tabs = list.insert(tabs_control(&list, &shown_tabs, tab));
        let search = list.insert(search_field(&list, monitor.filter(), ""));
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
            readout_width: READOUT_WIDTH,
            cores: Animated::points(0.0),
            cores_extent: 0.0,
            cores_moving: false,
        };
        view.sync();
        view
    }

    pub fn table(&self) -> Option<&Table> {
        self.list.shown_table()
    }

    fn sync(&mut self) {
        let mut ids = HashMap::new();
        let alarm = self.palette.danger;
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
            false => LEADING,
        };
        let search = Rect::new(
            self.size.0 - SIDE - SEARCH_WIDTH,
            middle(TEXT_FIELD_HEIGHT),
            SEARCH_WIDTH,
            TEXT_FIELD_HEIGHT,
        );
        let tabs_width = SEGMENT_WIDTH * self.tabs.len() as f32;
        let tabs = Rect::new(
            search.x - ITEM_GAP - tabs_width,
            middle(SEGMENTED_HEIGHT),
            tabs_width,
            SEGMENTED_HEIGHT,
        );
        let inspect_x = (tabs.x - ITEM_GAP - BUTTON_SIZE).max(leading);
        let stop_x = (inspect_x - BUTTON_GAP - BUTTON_SIZE).max(leading);
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

    fn footer_layout(&self) -> Footer {
        footer_layout(
            self.sheet(),
            slot_count(&self.monitor),
            self.readout_width,
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
        let widest = summary
            .iter()
            .map(|(label, value)| {
                width(label, readout_label(palette)) + width(value, readout_value(palette))
            })
            .fold(0.0, f32::max);
        let key = match self.monitor.memory_bar() {
            Some(_) => KEY_SIZE + KEY_GAP,
            None => 0.0,
        };
        self.readout_width = ((widest + key + READOUT_SPACE) / GRID).ceil() * GRID;
    }

    fn layout(&mut self) {
        self.measure_readouts();
        for (id, frame) in self.toolbar_frames() {
            self.list.place(id, frame);
        }
        self.list.set_frame(self.list_frame());
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

    fn measures(&self) -> [bool; 4] {
        [
            self.monitor.measures_proportional_memory(),
            self.monitor.measures_gpu(),
            self.monitor.measures_wakeups(),
            self.monitor.measures_disk(),
        ]
    }

    fn show_columns(&mut self) {
        let tab = self.monitor.tab();
        let columns = table_columns(tab, self.monitor.columns());
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
            Key::Character(digit @ '1'..='9') if modifiers.logo => {
                let index = digit as usize - '1' as usize;
                match self.tabs.get(index).copied() {
                    Some(tab) => {
                        self.set_tab(tab);
                        true
                    }
                    None => false,
                }
            }
            Key::Character('f') if modifiers.logo => {
                self.list.focus_widget(self.toolbar.search);
                true
            }
            Key::Character('q' | 'Q') if modifiers.logo && modifiers.alt => {
                self.open_stop_dialog(false)
            }
            Key::Character('r' | 'R') if modifiers.logo && modifiers.shift => self.show_in_files(),
            Key::Character('i') if modifiers.logo => self.inspect(),
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
        let on_graph = self
            .footer_layout()
            .graph
            .is_some_and(|graph| contains(graph, at));
        if on_graph && !self.list.has_dialog() && self.monitor.tab() == Tab::Cpu {
            return self.toggle_cores();
        }
        let handled = self.pass(event);
        if self.list.has_dialog() || !contains(self.list.frame(), at) {
            return handled;
        }
        let header = at.y < self.list.frame().y + HEADER_HEIGHT;
        if is_secondary(pointer) && header && self.monitor.tab().has_column_choice() {
            let menu = columns_menu(self.monitor.tab(), self.monitor.columns());
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

fn paint_series(
    painter: &mut Painter,
    values: &[f32],
    plot: Rect,
    line: Color,
    area: Option<Color>,
) {
    let points = series_points(values, plot);
    let (Some(path), Some(first), Some(last)) = (polyline(&points), points.first(), points.last())
    else {
        return;
    };
    let bottom = plot.y + plot.height;
    if let Some(area) = area {
        let closed = path
            .clone()
            .line_to(Point::new(last.x, bottom))
            .line_to(Point::new(first.x, bottom))
            .close();
        painter.items.push(Item::FillPath {
            path: closed,
            paint: area.into(),
        });
    }
    painter.items.push(Item::StrokePath {
        path,
        width: SERIES_WIDTH,
        paint: line.into(),
    });
}

fn pressure_color(level: PressureLevel, palette: Palette) -> Color {
    match level {
        PressureLevel::Normal => palette.success,
        PressureLevel::Elevated => palette.warning,
        PressureLevel::Critical => palette.danger,
    }
}

fn paint_pressure(
    painter: &mut Painter,
    samples: &[(f32, PressureLevel)],
    plot: Rect,
    palette: Palette,
) {
    let step = plot.width / HISTORY as f32;
    let right = plot.x + plot.width;
    let bottom = plot.y + plot.height;
    let first = samples.len().saturating_sub(HISTORY);
    let shown = &samples[first..];
    for (index, (used, level)) in shown.iter().enumerate() {
        let height = used.clamp(0.0, 1.0) * plot.height;
        let x = right - (shown.len() - index) as f32 * step;
        painter.fill(
            Shape::rect(Rect::new(x, bottom - height, (step - 1.0).max(1.0), height)),
            pressure_color(*level, palette),
        );
    }
}

fn paint_graph(monitor: &Monitor, palette: Palette, painter: &mut Painter, frame: Rect) {
    let well = Shape::rounded(frame, Radius::S);
    let mut graph = Painter {
        text: &mut *painter.text,
        items: Vec::new(),
    };
    graph.fill(well, palette.well);
    for quarter in 1..4 {
        let y = (frame.y + frame.height * quarter as f32 / 4.0).round();
        graph.fill(
            Shape::rect(Rect::new(frame.x, y, frame.width, 1.0)),
            palette.hairline,
        );
    }
    let plot = Rect::new(frame.x, frame.y + 1.0, frame.width, frame.height - 1.0);
    let graph_kind = monitor.graph();
    let is_scale = matches!(graph_kind, Graph::Lines { .. });
    let title = match graph_kind {
        Graph::Lines {
            primary,
            secondary,
            scale,
        } => {
            paint_series(
                &mut graph,
                &primary,
                plot,
                palette.accent,
                Some(palette.accent_soft),
            );
            if let Some(secondary) = &secondary {
                paint_series(&mut graph, secondary, plot, palette.ink2, None);
            }
            scale
        }
        Graph::Pressure(samples) => {
            paint_pressure(&mut graph, &samples, plot, palette);
            L10N.tr("graph-pressure")
        }
    };
    let label = readout_label(palette);
    let corner = |y: f32| {
        Rect::new(
            frame.x + GRAPH_PAD,
            y,
            frame.width - 2.0 * GRAPH_PAD,
            GRAPH_LABEL,
        )
    };
    let title_style = match is_scale {
        true => TextStyle {
            caps: false,
            ..label
        }
        .tabular(),
        false => label,
    };
    graph.text_in(
        &title,
        title_style,
        corner(frame.y + GRAPH_PAD),
        TextAlign::Start,
    );
    graph.text_in(
        &L10N.tr("graph-span"),
        label,
        corner(frame.y + frame.height - GRAPH_PAD - GRAPH_LABEL),
        TextAlign::Start,
    );
    let items = std::mem::take(&mut graph.items);
    painter.items.push(Item::Clip { shape: well, items });
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Footer {
    sheet: Rect,
    top: f32,
    rows: usize,
    readout_width: f32,
    graph: Option<Rect>,
    cores: Rect,
}

fn readout_label(palette: Palette) -> TextStyle {
    TextStyle::role(TextRole::Label, palette.ink3)
}

fn readout_value(palette: Palette) -> TextStyle {
    TextStyle::role(TextRole::BodyStrong, palette.ink).tabular()
}

fn footer_layout(sheet: Rect, readouts: usize, readout_width: f32, cores: f32) -> Footer {
    let available = sheet.width - 2.0 * FOOTER_PAD;
    let readouts_width = |columns: usize| {
        columns as f32 * readout_width + columns.saturating_sub(1) as f32 * READOUT_GAP
    };
    let fitting = (1..=READOUT_COLUMNS)
        .rev()
        .find(|columns| available - readouts_width(*columns) - READOUT_GAP >= GRAPH_MIN_WIDTH)
        .unwrap_or(1);
    let columns = fitting.min(readouts.div_ceil(READOUT_ROWS)).max(1);
    let rows = readouts.div_ceil(columns).max(1);
    let height = (rows as f32 * READOUT_ROW + 2.0 * FOOTER_PAD).max(FOOTER_HEIGHT);
    let top = sheet.y + sheet.height - height - cores;
    let graph_x = sheet.x + FOOTER_PAD + readouts_width(columns) + READOUT_GAP;
    let graph_width = sheet.x + sheet.width - FOOTER_PAD - graph_x;
    let graph = (graph_width >= GRAPH_SMALLEST_WIDTH).then(|| {
        Rect::new(
            graph_x,
            top + FOOTER_PAD,
            graph_width,
            height - 2.0 * FOOTER_PAD,
        )
    });
    Footer {
        sheet,
        top,
        rows,
        readout_width,
        graph,
        cores: Rect::new(
            sheet.x + FOOTER_PAD,
            top + height,
            sheet.width - 2.0 * FOOTER_PAD,
            cores,
        ),
    }
}

fn paint_footer(monitor: &Monitor, palette: Palette, footer: Footer, painter: &mut Painter) {
    let (sheet, readout_width) = (footer.sheet, footer.readout_width);
    let summary = monitor.summary();
    let bar = monitor.memory_bar();
    painter.fill(
        Shape::rect(Rect::new(sheet.x, footer.top, sheet.width, 1.0)),
        palette.hairline,
    );
    let slot = |index: usize| {
        let x = sheet.x + FOOTER_PAD + (index / footer.rows) as f32 * (readout_width + READOUT_GAP);
        let y = footer.top + FOOTER_PAD + (index % footer.rows) as f32 * READOUT_ROW;
        Rect::new(x, y, readout_width, READOUT_ROW)
    };
    let colors = segment_colors(palette);
    if let Some(bar) = &bar {
        paint_memory_bar(painter, bar, slot(BAR_SLOT), palette);
    }
    for (index, (label, value)) in summary.iter().enumerate() {
        let shift = usize::from(bar.is_some() && index >= BAR_SLOT);
        let mut row = slot(index + shift);
        let key = bar
            .iter()
            .flatten()
            .position(|(readout, _)| *readout == index)
            .and_then(|segment| colors.get(segment));
        if let Some(color) = key {
            let middle = row.y + ((READOUT_ROW - KEY_SIZE) / 2.0).round();
            painter.fill(
                Shape::rounded(Rect::new(row.x, middle, KEY_SIZE, KEY_SIZE), KEY_RADIUS),
                *color,
            );
            row = Rect::new(
                row.x + KEY_SIZE + KEY_GAP,
                row.y,
                row.width - KEY_SIZE - KEY_GAP,
                row.height,
            );
        }
        painter.text_in(label, readout_label(palette), row, TextAlign::Start);
        painter.text_in(value, readout_value(palette), row, TextAlign::End);
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
                let well = Shape::rounded(frame, Radius::S);
                let mut strip = Painter {
                    text: &mut *painter.text,
                    items: Vec::new(),
                };
                strip.fill(well, palette.well);
                paint_series(
                    &mut strip,
                    &history,
                    frame,
                    palette.accent,
                    Some(palette.accent_soft),
                );
                strip.text_in(
                    &L10N.tr_args("core-label", [("number", (core + 1).into())]),
                    readout_label(palette),
                    Rect::new(
                        frame.x + GRAPH_PAD,
                        frame.y + 2.0,
                        frame.width - 2.0 * GRAPH_PAD,
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

fn segment_colors(palette: Palette) -> [Color; 4] {
    [
        palette.accent,
        palette.ink2,
        palette.ink3,
        palette.accent_soft,
    ]
}

fn slot_count(monitor: &Monitor) -> usize {
    monitor.summary().len() + usize::from(monitor.memory_bar().is_some())
}

fn paint_memory_bar(painter: &mut Painter, bar: &[(usize, f32)], slot: Rect, palette: Palette) {
    let track = Rect::new(
        slot.x,
        slot.y + ((slot.height - BAR_HEIGHT) / 2.0).round(),
        slot.width,
        BAR_HEIGHT,
    );
    let shape = Shape::rounded(track, BAR_HEIGHT / 2.0);
    let mut items = vec![Item::Fill {
        shape: Shape::rect(track),
        paint: palette.well.into(),
    }];
    let mut x = track.x;
    for ((_, share), color) in bar.iter().zip(segment_colors(palette)) {
        let width = (share.clamp(0.0, 1.0) * track.width).round();
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(x, track.y, width, track.height)),
            paint: color.into(),
        });
        x += width;
    }
    painter.items.push(Item::Clip { shape, items });
}

fn search_field(list: &List, filter: Filter, text: &str) -> SearchField {
    let send = sender(list.signals());
    let placeholder = match filter {
        Filter::All => L10N.tr("search-placeholder"),
        filter => L10N.tr_args("search-filtered", [("filter", filter.title().into())]),
    };
    SearchField::new(&placeholder, text, move |text| {
        send(Signal::Search(text.to_owned()))
    })
}

impl Screen for MonitorView {
    fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.list.set_palette(palette);
        self.sync();
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
        let (monitor, palette) = (&self.monitor, self.palette);
        let mut painter = Painter {
            text: self.list.text(),
            items: Vec::new(),
        };
        paint_footer(monitor, palette, footer, &mut painter);
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
        }
        self.sync();
    }

    fn measures_gpu(&self) -> bool {
        self.monitor.measures_gpu()
    }

    fn measures_disk(&self) -> bool {
        self.monitor.measures_disk()
    }

    fn measures_proportional_memory(&self) -> bool {
        self.monitor.measures_proportional_memory()
    }

    fn measures_wakeups(&self) -> bool {
        self.monitor.measures_wakeups()
    }

    fn measures_sleep_blockers(&self) -> bool {
        self.monitor.measures_sleep_blockers()
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
        for (index, (label, value)) in self.monitor.summary().into_iter().enumerate() {
            let mut readout = Node::new(Role::Label);
            readout.set_label(label);
            readout.set_value(value);
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
        self.list.cursor()
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
