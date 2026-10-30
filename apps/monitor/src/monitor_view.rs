use std::collections::{HashMap, HashSet};

use accesskit::{ActionRequest, TreeUpdate};
use unbound_toolkit::ui::HEADER_HEIGHT;
use unbound_toolkit::ui::{
    Cell, Column as TableColumn, Cursor, Event, Key, Modifiers, Pointer, Row as TableRow, Sort,
    Table,
};
use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::list::{List, Signal, group_id};
use crate::menu::{MenuCommand, Selected, process_menu};
use crate::model::{Column, Graph, HISTORY, Monitor, Row, RowKey, Tab};
use crate::sampler::Snapshot;
use crate::view::{
    Dialog, DialogButton, Icons, NOT_RESPONDING, Painter, Request, Screen, contains, dialog_click,
    dialog_key, is_secondary, opens_menu, press_dialog_button,
};

pub const TOOLBAR_HEIGHT: f32 = 48.0;
pub const FOOTER_HEIGHT: f32 = 128.0;
const SIDE: f32 = 12.0;
const TEXT_SIZE: f32 = 12.0;
const SMALL_SIZE: f32 = 11.0;
const SEGMENT_WIDTH: f32 = 84.0;
const CONTROL_HEIGHT: f32 = 26.0;
const SEARCH_WIDTH: f32 = 200.0;
const STOP_SIZE: f32 = 28.0;
const GRAPH_WIDTH: f32 = 240.0;
const NAME_WIDTH: f32 = 160.0;
const PROGRAM_ICON: &str = "application-x-executable";
const SYSTEM_COLOR: Color = Color::hex(0xE5484D);
const USER_COLOR: Color = Color::hex(0x3E8EF7);
const MEMORY_COLOR: Color = Color::hex(0x30A46C);

fn column_width(column: Column) -> f32 {
    match column {
        Column::Name => NAME_WIDTH,
        Column::Cpu => 72.0,
        Column::Pid => 64.0,
        Column::Threads => 68.0,
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

fn tab_frames(width: f32) -> Vec<(Tab, Rect)> {
    let total = SEGMENT_WIDTH * Tab::ALL.len() as f32;
    let left = ((width - total) / 2.0).round();
    let y = ((TOOLBAR_HEIGHT - CONTROL_HEIGHT) / 2.0).round();
    Tab::ALL
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            (
                *tab,
                Rect::new(
                    left + index as f32 * SEGMENT_WIDTH,
                    y,
                    SEGMENT_WIDTH,
                    CONTROL_HEIGHT,
                ),
            )
        })
        .collect()
}

fn search_frame(width: f32) -> Rect {
    Rect::new(
        width - SIDE - SEARCH_WIDTH,
        ((TOOLBAR_HEIGHT - CONTROL_HEIGHT) / 2.0).round(),
        SEARCH_WIDTH,
        CONTROL_HEIGHT,
    )
}

fn stop_frame() -> Rect {
    Rect::new(
        SIDE,
        ((TOOLBAR_HEIGHT - STOP_SIZE) / 2.0).round(),
        STOP_SIZE,
        STOP_SIZE,
    )
}

pub struct MonitorView {
    pub monitor: Monitor,
    palette: Palette,
    icons: Icons,
    list: List,
    ids: HashMap<String, RowKey>,
    revealed: HashSet<String>,
    size: (f32, f32),
    searching: bool,
    focused: bool,
    alternate: bool,
    dialog: Option<Dialog>,
    requests: Vec<Request>,
}

impl MonitorView {
    pub fn new(monitor: Monitor, palette: Palette, text: TextSystem, icons: Icons) -> Self {
        let tab = monitor.tab();
        let mut table = Table::new(table_columns(tab));
        table.set_sort(table_sort(tab, monitor.sort()));
        let mut view = Self {
            monitor,
            palette,
            icons,
            list: List::new(table, palette, text),
            ids: HashMap::new(),
            revealed: HashSet::new(),
            size: (0.0, 0.0),
            searching: false,
            focused: true,
            alternate: false,
            dialog: None,
            requests: Vec::new(),
        };
        view.sync();
        view
    }

    pub fn table(&self) -> Option<&Table> {
        self.list.shown_table()
    }

    fn table_row(&mut self, row: &Row, ids: &mut HashMap<String, RowKey>) -> TableRow {
        let id = row_id(&row.key);
        ids.insert(id.clone(), row.key.clone());
        let mut cells = row
            .cells
            .iter()
            .map(|text| Cell::new(text.as_str()))
            .collect::<Vec<_>>();
        if let Some(name) = cells.first_mut() {
            let icon = match &row.key {
                RowKey::Interface(_) => None,
                _ => Some(row.icon.as_deref().unwrap_or(PROGRAM_ICON)),
            };
            name.icon = icon.and_then(|icon| self.icons.get(icon));
            if !row.responding {
                name.text = L10N.tr_args("not-responding", [("name", row.name.clone().into())]);
                name.tint = Some(NOT_RESPONDING);
            }
        }
        let children = row
            .children
            .iter()
            .map(|child| self.table_row(child, ids))
            .collect();
        TableRow::new(id, cells).children(children)
    }

    fn sync(&mut self) {
        let mut ids = HashMap::new();
        let rows = self.monitor.rows().to_vec();
        let table_rows = rows
            .iter()
            .map(|row| self.table_row(row, &mut ids))
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
        self.list.take_signals();
        self.select_ids(&selection);
    }

    fn select_ids(&mut self, ids: &[String]) {
        let keys = ids
            .iter()
            .filter_map(|id| self.ids.get(id).cloned())
            .collect();
        self.monitor.set_selection(keys);
    }

    fn drain_signals(&mut self) {
        for signal in self.list.take_signals() {
            match signal {
                Signal::Selection(ids) => self.select_ids(&ids),
                Signal::Activate => self.toggle_selected_group(),
                Signal::Sort(sort) => {
                    if let Some(column) = self.monitor.tab().columns().get(sort.column) {
                        self.monitor.set_sort(*column, !sort.ascending);
                        self.sync();
                    }
                }
            }
        }
        self.requests
            .extend(self.list.take_copy().map(Request::Copy));
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

    fn list_frame(&self) -> Rect {
        Rect::new(
            0.0,
            TOOLBAR_HEIGHT,
            self.size.0,
            (self.size.1 - TOOLBAR_HEIGHT - FOOTER_HEIGHT).max(0.0),
        )
    }

    fn set_tab(&mut self, tab: Tab) {
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
        match self.monitor.stop_target() {
            Some(target) => {
                self.dialog = Some(match force {
                    true => Dialog::ConfirmForce(target),
                    false => Dialog::Confirm(target),
                });
                true
            }
            None => false,
        }
    }

    fn press(&mut self, button: DialogButton) {
        if let Some(dialog) = self.dialog.take() {
            self.dialog = press_dialog_button(dialog, button);
            if button != DialogButton::Cancel {
                self.requests.push(Request::Resample);
            }
        }
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

    fn set_searching(&mut self, searching: bool) {
        self.searching = searching;
        match searching || !self.focused {
            true => self.list.blur(),
            false => self.list.focus(),
        }
    }

    fn set_search(&mut self, search: &str) {
        self.monitor.set_search(search);
        self.sync();
    }

    fn search_key(&mut self, key: Key, modifiers: Modifiers) -> Option<bool> {
        if !self.searching || modifiers.logo || modifiers.control {
            return None;
        }
        let mut search = self.monitor.search().to_owned();
        match key {
            Key::Escape if search.is_empty() => {
                self.set_searching(false);
                return Some(true);
            }
            Key::Escape => search.clear(),
            Key::Backspace => {
                search.pop();
            }
            Key::Enter | Key::Down | Key::Tab => {
                self.set_searching(false);
                if self.monitor.selection().is_empty() {
                    self.select_first();
                }
                return Some(true);
            }
            Key::Character(character) => search.push(character),
            Key::Space => search.push(' '),
            _ => return Some(false),
        }
        self.set_search(&search);
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

    fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if let Some(dialog) = &self.dialog {
            if let Some(button) = dialog_key(dialog, key, modifiers) {
                self.press(button);
            }
            return true;
        }
        if let Some(handled) = self.search_key(key, modifiers) {
            return handled;
        }
        if opens_menu(key, modifiers) {
            return self.menu_from_keyboard();
        }
        match key {
            Key::Character(digit @ '1'..='4') if modifiers.logo => {
                let index = digit as usize - '1' as usize;
                self.set_tab(Tab::ALL[index]);
                true
            }
            Key::Character('f') if modifiers.logo => {
                self.set_searching(true);
                true
            }
            Key::Character('q' | 'Q') if modifiers.logo && modifiers.alt => {
                self.open_stop_dialog(false)
            }
            Key::Character('r' | 'R') if modifiers.logo && modifiers.shift => self.show_in_files(),
            Key::Escape if !self.monitor.search().is_empty() => {
                self.set_search("");
                true
            }
            _ => {
                let handled = self.list.handle(&Event::Key { key, modifiers });
                self.drain_signals();
                handled
            }
        }
    }

    fn commit(&mut self, text: &str) -> bool {
        if !self.searching || self.dialog.is_some() {
            return false;
        }
        let search = format!("{}{text}", self.monitor.search());
        self.set_search(&search);
        true
    }

    fn pointer_down(&mut self, event: &Event, pointer: &Pointer) -> bool {
        let at = pointer.position;
        if let Some(dialog) = &self.dialog {
            return match dialog_click(dialog, self.size, at).filter(|_| pointer.is_primary()) {
                Some(button) => {
                    self.press(button);
                    true
                }
                None => false,
            };
        }
        if at.y < TOOLBAR_HEIGHT {
            return pointer.is_primary() && self.toolbar_click(at);
        }
        if !contains(self.list.frame(), at) {
            return false;
        }
        if self.searching {
            self.set_searching(false);
        }
        let handled = self.list.handle(event);
        self.drain_signals();
        let header = at.y < self.list.frame().y + HEADER_HEIGHT;
        if is_secondary(pointer) && !header && !self.monitor.selection().is_empty() {
            self.request_menu(at);
            return true;
        }
        handled
    }

    fn toolbar_click(&mut self, at: Point) -> bool {
        self.set_searching(contains(search_frame(self.size.0), at));
        if contains(stop_frame(), at) {
            return self.open_stop_dialog(false);
        }
        if let Some((tab, _)) = tab_frames(self.size.0)
            .into_iter()
            .find(|(_, frame)| contains(*frame, at))
        {
            self.set_tab(tab);
        }
        true
    }

    fn pass(&mut self, event: &Event) -> bool {
        if self.dialog.is_some() {
            return false;
        }
        let handled = self.list.handle(event);
        self.drain_signals();
        handled
    }
}

struct Chrome<'a> {
    monitor: &'a Monitor,
    palette: Palette,
    size: (f32, f32),
    searching: bool,
}

impl Chrome<'_> {
    fn paint_toolbar(&self, painter: &mut Painter) {
        let palette = self.palette;
        let stop = stop_frame();
        let enabled = self.monitor.stop_target().is_some();
        let color = if enabled {
            palette.label_primary
        } else {
            palette.label_muted.with_alpha(110)
        };
        painter.items.push(Item::Stroke {
            shape: Shape::rounded(stop.inset(4.0), (STOP_SIZE - 8.0) / 2.0),
            width: 1.5,
            paint: color.into(),
        });
        painter.text_in(
            "\u{2715}",
            TextStyle::new(SMALL_SIZE, color).weight(TextStyle::SEMIBOLD),
            stop,
            TextAlign::Center,
        );
        let frames = tab_frames(self.size.0);
        if let (Some((_, first)), Some((_, last))) = (frames.first(), frames.last()) {
            let track = Rect::new(
                first.x,
                first.y,
                last.x + last.width - first.x,
                first.height,
            );
            painter.fill(
                Shape::rounded(track, 7.0),
                palette.label_muted.with_alpha(36),
            );
        }
        for (tab, frame) in frames {
            let selected = tab == self.monitor.tab();
            if selected {
                let shape = Shape::rounded(frame.inset(2.0), 5.0);
                painter.items.push(Item::Shadow {
                    shape,
                    offset: Point::new(0.0, 0.5),
                    blur_radius: 2.0,
                    color: Color::hex(0x000000).with_alpha(60),
                });
                painter.fill(shape, palette.surface_elevated);
            }
            let weight = if selected {
                TextStyle::SEMIBOLD
            } else {
                TextStyle::MEDIUM
            };
            painter.text_in(
                &tab.title(),
                TextStyle::new(TEXT_SIZE, palette.label_primary).weight(weight),
                frame,
                TextAlign::Center,
            );
        }
        self.paint_search(painter);
        painter.fill(
            Shape::rect(Rect::new(0.0, TOOLBAR_HEIGHT - 1.0, self.size.0, 1.0)),
            palette.separator_color,
        );
    }

    fn paint_search(&self, painter: &mut Painter) {
        let palette = self.palette;
        let search = search_frame(self.size.0);
        let shape = Shape::rounded(search, 7.0);
        painter.fill(shape, palette.label_muted.with_alpha(36));
        if self.searching {
            painter.items.push(Item::Stroke {
                shape: Shape::rounded(search.inset(-1.5), 8.5),
                width: 3.0,
                paint: palette.accent_color.with_alpha(120).into(),
            });
        }
        let text_frame = Rect::new(
            search.x + 26.0,
            search.y,
            search.width - 34.0,
            search.height,
        );
        painter.text_in(
            "\u{2315}",
            TextStyle::new(TEXT_SIZE + 2.0, palette.label_muted),
            Rect::new(search.x + 6.0, search.y, 18.0, search.height),
            TextAlign::Center,
        );
        let query = self.monitor.search();
        if query.is_empty() && !self.searching {
            painter.text_in(
                &L10N.tr("search-placeholder"),
                TextStyle::new(TEXT_SIZE, palette.label_muted),
                text_frame,
                TextAlign::Start,
            );
            return;
        }
        let paragraph = painter.paragraph(
            query,
            TextStyle::new(TEXT_SIZE, palette.label_primary),
            None,
            TextAlign::Start,
        );
        let width = paragraph.width().min(text_frame.width - 2.0);
        let y = text_frame.y + ((text_frame.height - paragraph.height()) / 2.0).round();
        painter.items.push(Item::Clip {
            shape: Shape::rect(text_frame),
            items: vec![Item::Text {
                origin: Point::new(text_frame.x, y),
                paragraph,
            }],
        });
        if self.searching {
            painter.fill(
                Shape::rect(Rect::new(
                    text_frame.x + width + 1.0,
                    text_frame.y + 6.0,
                    1.5,
                    text_frame.height - 12.0,
                )),
                palette.accent_color,
            );
        }
    }

    fn paint_graph(&self, painter: &mut Painter, frame: Rect) {
        let palette = self.palette;
        painter.fill(Shape::rounded(frame, 6.0), palette.surface_elevated);
        let inner = frame.inset(6.0);
        let bar = inner.width / HISTORY as f32;
        let bars =
            |painter: &mut Painter, values: &[f32], color: Color, baseline: f32, reach: f32| {
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
        match self.monitor.graph() {
            Graph::Stacked { lower, upper } => {
                bars(painter, &upper, USER_COLOR, bottom, inner.height);
                bars(painter, &lower, SYSTEM_COLOR, bottom, inner.height);
            }
            Graph::Filled(values) => bars(painter, &values, MEMORY_COLOR, bottom, inner.height),
            Graph::Mirrored { up, down } => {
                let middle = (inner.y + inner.height / 2.0).round();
                bars(painter, &up, USER_COLOR, middle, inner.height / 2.0);
                bars(painter, &down, SYSTEM_COLOR, middle, -inner.height / 2.0);
                painter.fill(
                    Shape::rect(Rect::new(inner.x, middle, inner.width, 1.0)),
                    palette.separator_color,
                );
            }
        }
    }

    fn paint_footer(&self, painter: &mut Painter) {
        let palette = self.palette;
        let top = self.size.1 - FOOTER_HEIGHT;
        painter.fill(
            Shape::rect(Rect::new(0.0, top, self.size.0, FOOTER_HEIGHT)),
            palette.label_muted.with_alpha(16),
        );
        painter.fill(
            Shape::rect(Rect::new(0.0, top, self.size.0, 1.0)),
            palette.separator_color,
        );
        let rows_per_column = 3;
        let column_width = 210.0;
        for (index, (label, value)) in self.monitor.summary().iter().enumerate() {
            let x = SIDE + (index / rows_per_column) as f32 * (column_width + SIDE);
            let y = top + 16.0 + (index % rows_per_column) as f32 * 22.0;
            painter.text_in(
                label,
                TextStyle::new(TEXT_SIZE, palette.label_muted),
                Rect::new(x, y, column_width, 20.0),
                TextAlign::Start,
            );
            painter.text_in(
                value,
                TextStyle::new(TEXT_SIZE, palette.label_primary)
                    .weight(TextStyle::MEDIUM)
                    .tabular(),
                Rect::new(x, y, column_width, 20.0),
                TextAlign::End,
            );
        }
        let graph = Rect::new(
            self.size.0 - SIDE - GRAPH_WIDTH,
            top + 12.0,
            GRAPH_WIDTH,
            FOOTER_HEIGHT - 24.0,
        );
        if graph.x > SIDE + 2.0 * column_width {
            self.paint_graph(painter, graph);
        }
    }
}

impl Screen for MonitorView {
    fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.list.set_palette(palette);
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.set_searching(self.searching);
    }

    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    fn menu_action(&mut self, action: &str) -> bool {
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
                self.set_searching(true);
                true
            }
            Some(MenuCommand::Show(tab)) => {
                self.set_tab(tab);
                true
            }
            None => false,
        }
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::Commit(text) => self.commit(text),
            Event::PointerDown(pointer) => self.pointer_down(event, pointer),
            Event::ModifiersChanged(modifiers) => {
                let changed = self.alternate != modifiers.alt;
                self.alternate = modifiers.alt;
                (changed && self.dialog.is_some()) | self.pass(event)
            }
            _ => self.pass(event),
        }
    }

    fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.list.set_frame(self.list_frame());
        let list = self.list.paint(width, height);
        let chrome = Chrome {
            monitor: &self.monitor,
            palette: self.palette,
            size: self.size,
            searching: self.searching,
        };
        let mut painter = Painter {
            text: self.list.text(),
            palette: self.palette,
            items: Vec::new(),
        };
        chrome.paint_toolbar(&mut painter);
        painter.items.extend(list);
        chrome.paint_footer(&mut painter);
        if let Some(dialog) = &self.dialog {
            painter.dialog(self.size, dialog, self.alternate);
        }
        Scene {
            width,
            height,
            background: self.palette.surface_background,
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
        self.drain_signals();
        performed
    }

    fn cursor(&self) -> Cursor {
        self.list.cursor()
    }

    fn animating(&self) -> bool {
        self.list.animating()
    }
}
