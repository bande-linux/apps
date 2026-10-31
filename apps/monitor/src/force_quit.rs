use std::collections::HashMap;
use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, Cell, Column, Cursor, Event, Key, Modifiers, Pointer, Row, Table,
    WidgetId,
};
use bande_toolkit::{
    Appearance, Item, Palette, Point, Radius, Rect, Scene, Shape, TextAlign, TextRole, TextSize,
    TextStyle, TextSystem, grown_line,
};

use crate::L10N;
use crate::apps::GroupKey;
use crate::list::{List, Signal, group_id, sender};
use crate::menu::{MenuCommand, force_quit_menu};
use crate::model::{AppList, StopTarget};
use crate::sampler::Snapshot;
use crate::view::{
    ALERT_ICON, FORCE_QUIT, Icons, Painter, QUIT, Request, Screen, answer_stop, is_secondary,
    opens_menu, stop_alert,
};

const PADDING: f32 = 16.0;
const HINT_HEIGHT: f32 = 44.0;
const HINT_MARGIN: f32 = 12.0;
const LIST_INSET: f32 = 4.0;
const BUTTON_WIDTH: f32 = 112.0;
const FOOTER_HEIGHT: f32 = 60.0;

pub struct ForceQuitView {
    apps: AppList,
    palette: Palette,
    icons: Icons,
    list: List,
    button: WidgetId,
    ids: HashMap<String, GroupKey>,
    size: (f32, f32),
    hint_height: f32,
    text_size: TextSize,
    stopping: Option<StopTarget>,
    requests: Vec<Request>,
    finished: bool,
}

impl ForceQuitView {
    pub fn new(apps: AppList, palette: Palette, text: TextSystem, icons: Icons) -> Self {
        let table = Table::new(vec![Column::new(&L10N.tr("column-name"), 120.0)]).without_header();
        let mut list = List::new(table, palette, text);
        let send = sender(list.signals());
        let mut button =
            Button::new(&L10N.tr("button-force-quit"), move || send(Signal::Stop)).primary();
        button.set_enabled(false);
        let button = list.insert(button);
        Self {
            apps,
            palette,
            icons,
            list,
            button,
            ids: HashMap::new(),
            size: (0.0, 0.0),
            hint_height: HINT_HEIGHT,
            text_size: TextSize::DEFAULT,
            stopping: None,
            requests: Vec::new(),
            finished: false,
        }
    }

    pub fn table(&self) -> Option<&Table> {
        self.list.shown_table()
    }

    fn footer_height(&self) -> f32 {
        FOOTER_HEIGHT + grown_line(self.text_size)
    }

    fn box_frame(&self) -> Rect {
        Rect::new(
            PADDING,
            self.hint_height,
            self.size.0 - 2.0 * PADDING,
            (self.size.1 - self.hint_height - self.footer_height()).max(0.0),
        )
    }

    fn button_frame(&self) -> Rect {
        let width = (BUTTON_WIDTH * self.text_size.scale()).round();
        let height = BUTTON_HEIGHT + grown_line(self.text_size);
        let footer = self.footer_height();
        Rect::new(
            self.size.0 - PADDING - width,
            self.size.1 - footer + ((footer - height) / 2.0).round(),
            width,
            height,
        )
    }

    fn sync(&mut self) {
        let mut ids = HashMap::new();
        let alarm = self.palette.danger;
        let rows = self
            .apps
            .rows()
            .map(|row| {
                let id = group_id(row.key);
                ids.insert(id.clone(), row.key.clone());
                let icon = row.icon.and_then(|icon| self.icons.get(icon));
                let cell = match row.responding {
                    true => Cell::new(row.name),
                    false => Cell::new(
                        L10N.tr_args("not-responding", [("name", row.name.to_owned().into())]),
                    )
                    .tinted(alarm),
                };
                Row::new(id, vec![cell.icon(icon)])
            })
            .collect();
        self.ids = ids;
        if let Some(table) = self.list.table() {
            table.set_rows(rows);
            let selection = table.selection();
            self.select_ids(&selection);
        }
        self.list.take_signals();
    }

    fn select_ids(&mut self, ids: &[String]) {
        let keys = ids
            .iter()
            .filter_map(|id| self.ids.get(id).cloned())
            .collect();
        self.apps.set_selection(keys);
        let selected = !self.apps.selection().is_empty();
        self.list
            .update::<Button>(self.button, |button, _| button.set_enabled(selected));
    }

    fn drain(&mut self) {
        loop {
            let signals = self.list.take_signals();
            if signals.is_empty() {
                break;
            }
            for signal in signals {
                match signal {
                    Signal::Selection(ids) => self.select_ids(&ids),
                    Signal::Activate(_) | Signal::Stop => {
                        self.confirm();
                    }
                    Signal::Choice(choice) => self.chosen(&choice),
                    Signal::Sort(_)
                    | Signal::Tab(_)
                    | Signal::Search(_)
                    | Signal::ClearSearch
                    | Signal::Inspect => {}
                }
            }
        }
        self.list.take_requests();
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

    fn request_menu(&mut self, at: Point) {
        let menu = force_quit_menu(self.apps.selected_entry().is_some());
        self.requests.push(Request::Menu(menu, at));
    }

    fn menu_from_keyboard(&mut self) -> bool {
        let bounds = self
            .list
            .shown_table()
            .and_then(|table| table.row_bounds(table.cursor_row()?));
        match bounds {
            Some(row) if !self.apps.selection().is_empty() => {
                self.request_menu(Point::new(row.x + 32.0, row.y + row.height));
                true
            }
            _ => false,
        }
    }

    fn confirm(&mut self) -> bool {
        let Some(target) = self.apps.stop_target() else {
            return false;
        };
        let icon = self.icons.get(ALERT_ICON);
        let send = sender(self.list.signals());
        let alert = stop_alert(&target, true, icon, move |choice| {
            send(Signal::Choice(choice.to_owned()))
        });
        self.stopping = Some(target);
        self.list.present_dialog(alert);
        true
    }

    fn key(&mut self, event: &Event, key: Key, modifiers: Modifiers) -> bool {
        if self.list.has_dialog() {
            return self.pass(event);
        }
        if opens_menu(key, modifiers) {
            return self.menu_from_keyboard();
        }
        match key {
            Key::Escape => self.finished = true,
            Key::Character('w') if modifiers.logo => self.finished = true,
            Key::Enter => return self.confirm(),
            _ => return self.pass(event),
        }
        true
    }

    fn pointer_down(&mut self, event: &Event, pointer: &Pointer) -> bool {
        let at = pointer.position;
        let handled = self.pass(event);
        if self.list.has_dialog() || !self.list.frame().contains(at) {
            return handled;
        }
        if is_secondary(pointer) && !self.apps.selection().is_empty() {
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

impl Screen for ForceQuitView {
    fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        let palette = self.palette;
        let hint_style = TextStyle::role(TextRole::Callout, palette.ink2);
        let hint_width = width as f32 - 2.0 * PADDING;
        let hint = self.list.text().paragraph(
            &L10N.tr("force-quit-hint"),
            hint_style,
            Some(hint_width),
            TextAlign::Start,
        );
        self.hint_height = (hint.height() + 2.0 * HINT_MARGIN).max(HINT_HEIGHT).ceil();
        let frame = self.box_frame();
        self.list.set_frame(frame.inset(LIST_INSET));
        self.list.place(self.button, self.button_frame());
        let (controls, dialog) = self.list.paint(width, height);
        let empty = self.apps.is_empty();
        let mut painter = Painter {
            text: self.list.text(),
            items: Vec::new(),
        };
        painter.text_in(
            &L10N.tr("force-quit-hint"),
            hint_style,
            Rect::new(PADDING, 0.0, hint_width, self.hint_height),
            TextAlign::Start,
        );
        let shape = Shape::rounded(frame, Radius::M);
        painter.fill(shape, palette.overlay);
        painter.items.push(Item::Stroke {
            shape: Shape::rounded(frame.inset(0.5), Radius::M - 0.5),
            width: 1.0,
            paint: palette.hairline.into(),
        });
        if empty {
            painter.text_in(
                &L10N.tr("force-quit-empty"),
                TextStyle::role(TextRole::Body, palette.ink2),
                frame,
                TextAlign::Center,
            );
        }
        painter.items.extend(controls);
        painter.items.extend(dialog);
        Scene {
            width,
            height,
            background: palette.sheet,
            items: painter.items,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.text_size = appearance.text_size;
        self.list.set_appearance(appearance);
        self.sync();
    }

    fn set_focused(&mut self, focused: bool) {
        match focused {
            true => self.list.focus(),
            false => self.list.blur(),
        }
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } => self.key(event, *key, *modifiers),
            Event::PointerDown(pointer) => self.pointer_down(event, pointer),
            _ => self.pass(event),
        }
    }

    fn update(&mut self, snapshot: Snapshot) {
        self.apps.update(snapshot);
        self.sync();
    }

    fn measures_proportional_memory(&self) -> bool {
        false
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
            Some(MenuCommand::ForceQuit) => self.confirm(),
            Some(MenuCommand::ShowInFiles) => {
                self.requests
                    .extend(self.apps.selected_entry().map(Request::ShowInFiles));
                false
            }
            _ => false,
        }
    }

    fn menu_closed(&mut self) -> bool {
        self.list.menu_closed()
    }

    fn pasted(&mut self, _text: &str) -> bool {
        false
    }

    fn set_responding(&mut self, pid: u32, app_id: &str, responding: bool) -> bool {
        let changed = self.apps.set_responding(pid, app_id, responding);
        if changed {
            self.sync();
        }
        changed
    }

    fn finished(&self) -> bool {
        self.finished
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
}
