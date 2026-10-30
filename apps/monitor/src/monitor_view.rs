use std::collections::{HashMap, HashSet};
use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use unbound_toolkit::ui::{
    Button, Cell, Column as TableColumn, Cursor, Event, HEADER_HEIGHT, Key, Modifiers, Pointer,
    Row as TableRow, SEGMENTED_HEIGHT, SearchField, SegmentedControl, Sort, TEXT_FIELD_HEIGHT,
    Table, WidgetId,
};
use unbound_toolkit::{
    Color, Header, Image, Item, Palette, Point, Radius, Rect, SHEET_INSET, Scene, Shape, TextAlign,
    TextRole, TextStyle, TextSystem,
};

use crate::L10N;
use crate::list::{List, Signal, group_id, sender};
use crate::menu::{MenuCommand, Selected, process_menu};
use crate::model::{Column, Graph, HISTORY, Monitor, Row, RowKey, StopTarget, Tab};
use crate::sampler::Snapshot;
use crate::view::{
    ALERT_ICON, FORCE_QUIT, Icons, Painter, QUIT, Request, Screen, answer_stop, contains,
    is_secondary, opens_menu, stop_alert,
};

pub const TOOLBAR_HEIGHT: f32 = unbound_toolkit::HEADER_HEIGHT as f32;
pub const FOOTER_HEIGHT: f32 = 112.0;
const SIDE: f32 = 12.0;
const LEADING: f32 = 94.0;
const BUTTON_SIZE: f32 = 28.0;
const ITEM_GAP: f32 = 16.0;
const SEGMENT_WIDTH: f32 = 80.0;
const SEARCH_WIDTH: f32 = 200.0;
const GRAPH_WIDTH: f32 = 240.0;
const NAME_WIDTH: f32 = 160.0;
const SUMMARY_WIDTH: f32 = 210.0;
const SUMMARY_ROW: f32 = 22.0;
const SUMMARY_ROWS: usize = 3;
const STOP_SYMBOL: &[u8] = include_bytes!("../data/process-stop-symbolic.svg");
const SYMBOL_PIXELS: u32 = 32;

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

fn table_columns(tab: Tab) -> Vec<TableColumn> {
    tab.columns()
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

fn table_sort(tab: Tab, (column, descending): (Column, bool)) -> Option<Sort> {
    let index = tab.columns().iter().position(|shown| *shown == column)?;
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
    tabs: WidgetId,
    search: WidgetId,
}

pub struct MonitorView {
    pub monitor: Monitor,
    palette: Palette,
    icons: Icons,
    list: List,
    toolbar: Toolbar,
    ids: HashMap<String, RowKey>,
    revealed: HashSet<String>,
    size: (f32, f32),
    full_screen: bool,
    focused: bool,
    stopping: Option<StopTarget>,
    requests: Vec<Request>,
}

fn tab_titles() -> Vec<String> {
    Tab::ALL.iter().map(|tab| tab.title()).collect()
}

impl MonitorView {
    pub fn new(monitor: Monitor, palette: Palette, text: TextSystem, icons: Icons) -> Self {
        let tab = monitor.tab();
        let mut table = Table::new(table_columns(tab));
        table.set_sort(table_sort(tab, monitor.sort()));
        let mut list = List::new(table, palette, text);
        let send = sender(list.signals());
        let stop_label = L10N.tr("stop-label");
        let stop = match Image::from_svg(STOP_SYMBOL, SYMBOL_PIXELS, SYMBOL_PIXELS) {
            Some(symbol) => list.insert(Button::symbol(symbol, &stop_label, move || {
                send(Signal::Stop)
            })),
            None => list.insert(Button::new(&stop_label, move || send(Signal::Stop))),
        };
        let send = sender(list.signals());
        let selected = Tab::ALL.iter().position(|shown| *shown == tab);
        let tabs = list.insert(SegmentedControl::new(
            &L10N.tr("tab-label"),
            &tab_titles(),
            selected,
            move |index| send(Signal::Tab(index)),
        ));
        let search = list.insert(search_field(&list));
        let mut view = Self {
            monitor,
            palette,
            icons,
            list,
            toolbar: Toolbar { stop, tabs, search },
            ids: HashMap::new(),
            revealed: HashSet::new(),
            size: (0.0, 0.0),
            full_screen: false,
            focused: true,
            stopping: None,
            requests: Vec::new(),
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
            Signal::Activate => self.toggle_selected_group(),
            Signal::Sort(sort) => {
                if let Some(column) = self.monitor.tab().columns().get(sort.column) {
                    self.monitor.set_sort(*column, !sort.ascending);
                    self.sync();
                }
            }
            Signal::Tab(index) => {
                if let Some(tab) = Tab::ALL.get(index) {
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
        if matches!(choice, QUIT | FORCE_QUIT) {
            self.requests.push(Request::Resample);
        }
        if let Some(alert) = denied {
            self.list.present_dialog(alert);
        }
        self.list.focus();
    }

    fn toggle_selected_group(&mut self) {
        let [key @ RowKey::Group(_)] = self.monitor.selection() else {
            return;
        };
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
        Rect::new(
            sheet.x,
            sheet.y,
            sheet.width,
            (sheet.height - FOOTER_HEIGHT).max(0.0),
        )
    }

    fn toolbar_frames(&self) -> [(WidgetId, Rect); 3] {
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
        let tabs_width = SEGMENT_WIDTH * Tab::ALL.len() as f32;
        let tabs = Rect::new(
            search.x - ITEM_GAP - tabs_width,
            middle(SEGMENTED_HEIGHT),
            tabs_width,
            SEGMENTED_HEIGHT,
        );
        let stop_x = (tabs.x - ITEM_GAP - BUTTON_SIZE).max(leading);
        [
            (
                self.toolbar.stop,
                Rect::new(stop_x, middle(BUTTON_SIZE), BUTTON_SIZE, BUTTON_SIZE),
            ),
            (self.toolbar.tabs, tabs),
            (self.toolbar.search, search),
        ]
    }

    fn layout(&mut self) {
        for (id, frame) in self.toolbar_frames() {
            self.list.place(id, frame);
        }
        self.list.set_frame(self.list_frame());
    }

    fn set_tab(&mut self, tab: Tab) {
        if let Some(index) = Tab::ALL.iter().position(|shown| *shown == tab) {
            self.list
                .update::<SegmentedControl>(self.toolbar.tabs, |control, cx| {
                    control.set_selected(cx, index)
                });
        }
        if tab == self.monitor.tab() {
            return;
        }
        let measured = self.monitor.measures_proportional_memory();
        self.monitor.set_tab(tab);
        if self.monitor.measures_proportional_memory() != measured {
            self.requests.push(Request::Resample);
        }
        let sort = table_sort(tab, self.monitor.sort());
        if let Some(table) = self.list.table() {
            table.set_columns(table_columns(tab));
            table.set_sort(sort);
        }
        self.sync();
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
        let field = search_field(&self.list);
        self.toolbar.search = self.list.replace(self.toolbar.search, field);
        self.set_search("");
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
                match Tab::ALL.get(index) {
                    Some(tab) => {
                        self.set_tab(*tab);
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
            Key::Escape if !self.monitor.search().is_empty() => {
                self.clear_search();
                true
            }
            _ => self.pass(event),
        }
    }

    fn pointer_down(&mut self, event: &Event, pointer: &Pointer) -> bool {
        let at = pointer.position;
        let handled = self.pass(event);
        if self.list.has_dialog() || !contains(self.list.frame(), at) {
            return handled;
        }
        let header = at.y < self.list.frame().y + HEADER_HEIGHT;
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

fn paint_graph(monitor: &Monitor, palette: Palette, painter: &mut Painter, frame: Rect) {
    painter.fill(Shape::rounded(frame, Radius::S), palette.well);
    let inner = frame.inset(6.0);
    let bar = inner.width / HISTORY as f32;
    let bars = |painter: &mut Painter, values: &[f32], color: Color, baseline: f32, reach: f32| {
        let start = inner.x + inner.width - values.len() as f32 * bar;
        for (index, value) in values.iter().enumerate() {
            let height = value.clamp(0.0, 1.0) * reach;
            if height.abs() < 0.5 {
                continue;
            }
            painter.fill(
                Shape::rect(Rect::new(
                    start + index as f32 * bar,
                    baseline.min(baseline - height),
                    (bar - 1.0).max(1.0),
                    height.abs(),
                )),
                color,
            );
        }
    };
    let bottom = inner.y + inner.height;
    match monitor.graph() {
        Graph::Stacked { lower, upper } => {
            bars(painter, &upper, palette.accent, bottom, inner.height);
            bars(painter, &lower, palette.danger, bottom, inner.height);
        }
        Graph::Filled(values) => bars(painter, &values, palette.accent, bottom, inner.height),
        Graph::Mirrored { up, down } => {
            let middle = (inner.y + inner.height / 2.0).round();
            bars(painter, &up, palette.accent, middle, inner.height / 2.0);
            bars(painter, &down, palette.danger, middle, -inner.height / 2.0);
            painter.fill(
                Shape::rect(Rect::new(inner.x, middle, inner.width, 1.0)),
                palette.hairline,
            );
        }
    }
}

fn paint_footer(monitor: &Monitor, palette: Palette, sheet: Rect, painter: &mut Painter) {
    let top = sheet.y + sheet.height - FOOTER_HEIGHT;
    painter.fill(
        Shape::rect(Rect::new(sheet.x, top, sheet.width, 1.0)),
        palette.hairline,
    );
    for (index, (label, value)) in monitor.summary().iter().enumerate() {
        let x = sheet.x + SIDE + (index / SUMMARY_ROWS) as f32 * (SUMMARY_WIDTH + 2.0 * SIDE);
        let y = top + 16.0 + (index % SUMMARY_ROWS) as f32 * SUMMARY_ROW;
        let row = Rect::new(x, y, SUMMARY_WIDTH, SUMMARY_ROW);
        painter.text_in(
            label,
            TextStyle::role(TextRole::Callout, palette.ink2),
            row,
            TextAlign::Start,
        );
        painter.text_in(
            value,
            TextStyle::role(TextRole::Callout, palette.ink)
                .weight(TextStyle::MEDIUM)
                .tabular(),
            row,
            TextAlign::End,
        );
    }
    let graph = Rect::new(
        sheet.x + sheet.width - SIDE - GRAPH_WIDTH,
        top + SIDE,
        GRAPH_WIDTH,
        FOOTER_HEIGHT - 2.0 * SIDE,
    );
    if graph.x > sheet.x + SIDE + 2.0 * (SUMMARY_WIDTH + 2.0 * SIDE) {
        paint_graph(monitor, palette, painter, graph);
    }
}

fn search_field(list: &List) -> SearchField {
    let send = sender(list.signals());
    SearchField::new(&L10N.tr("search-placeholder"), "", move |text| {
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
        match MenuCommand::parse(action) {
            Some(MenuCommand::Quit) => self.open_stop_dialog(false),
            Some(MenuCommand::ForceQuit) => self.open_stop_dialog(true),
            Some(MenuCommand::ShowInFiles) => self.show_in_files(),
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
        self.size = (width as f32, height as f32);
        self.layout();
        let (controls, dialog) = self.list.paint(width, height);
        let sheet = self.sheet();
        let radius = Radius::concentric(Radius::XL, self.inset());
        let (monitor, palette) = (&self.monitor, self.palette);
        let mut painter = Painter {
            text: self.list.text(),
            items: Vec::new(),
        };
        paint_footer(monitor, palette, sheet, &mut painter);
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
        self.sync();
    }

    fn measures_proportional_memory(&self) -> bool {
        self.monitor.measures_proportional_memory()
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
        Some(self.list.accessibility())
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
        self.list.animating()
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
        })
    }

    fn set_full_screen(&mut self, full_screen: bool) {
        self.full_screen = full_screen;
    }
}
