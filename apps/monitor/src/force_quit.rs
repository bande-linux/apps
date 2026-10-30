use std::collections::HashMap;

use accesskit::{ActionRequest, TreeUpdate};
use unbound_toolkit::ui::{Cell, Column, Cursor, Event, Key, Modifiers, Pointer, Row, Table};
use unbound_toolkit::{Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem};

use crate::L10N;
use crate::apps::GroupKey;
use crate::list::{List, Signal, group_id};
use crate::menu::{MenuCommand, force_quit_menu};
use crate::model::AppList;
use crate::sampler::Snapshot;
use crate::view::{
    Dialog, DialogButton, Icons, NOT_RESPONDING, Painter, Request, Screen, contains, dialog_click,
    dialog_key, is_secondary, opens_menu, press_dialog_button,
};

const PADDING: f32 = 16.0;
const HINT_HEIGHT: f32 = 44.0;
const LIST_INSET: f32 = 4.0;
const BUTTON_WIDTH: f32 = 104.0;
const BUTTON_HEIGHT: f32 = 26.0;
const FOOTER_HEIGHT: f32 = 58.0;
const TEXT_SIZE: f32 = 13.0;

pub struct ForceQuitView {
    apps: AppList,
    palette: Palette,
    icons: Icons,
    list: List,
    ids: HashMap<String, GroupKey>,
    size: (f32, f32),

    dialog: Option<Dialog>,
    requests: Vec<Request>,
    finished: bool,
}

impl ForceQuitView {
    pub fn new(apps: AppList, palette: Palette, text: TextSystem, icons: Icons) -> Self {
        let table = Table::new(vec![Column::new(&L10N.tr("column-name"), 120.0)]).without_header();
        Self {
            apps,
            palette,
            icons,
            list: List::new(table, palette, text),
            ids: HashMap::new(),
            size: (0.0, 0.0),

            dialog: None,
            requests: Vec::new(),
            finished: false,
        }
    }

    pub fn table(&self) -> Option<&Table> {
        self.list.shown_table()
    }

    fn box_frame(&self) -> Rect {
        Rect::new(
            PADDING,
            HINT_HEIGHT,
            self.size.0 - 2.0 * PADDING,
            (self.size.1 - HINT_HEIGHT - FOOTER_HEIGHT).max(0.0),
        )
    }

    fn button_frame(&self) -> Rect {
        Rect::new(
            self.size.0 - PADDING - BUTTON_WIDTH,
            self.size.1 - FOOTER_HEIGHT + ((FOOTER_HEIGHT - BUTTON_HEIGHT) / 2.0).round(),
            BUTTON_WIDTH,
            BUTTON_HEIGHT,
        )
    }

    fn sync(&mut self) {
        let mut ids = HashMap::new();
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
                    .tinted(NOT_RESPONDING),
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
    }

    fn drain_signals(&mut self) {
        for signal in self.list.take_signals() {
            match signal {
                Signal::Selection(ids) => self.select_ids(&ids),
                Signal::Activate => {
                    self.confirm();
                }
                Signal::Sort(_) => {}
            }
        }
        self.list.take_copy();
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
        match self.apps.stop_target() {
            Some(target) => {
                self.dialog = Some(Dialog::ConfirmForce(target));
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

    fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if let Some(dialog) = &self.dialog {
            if let Some(button) = dialog_key(dialog, key, modifiers) {
                self.press(button);
            }
            return true;
        }
        if opens_menu(key, modifiers) {
            return self.menu_from_keyboard();
        }
        match key {
            Key::Escape => self.finished = true,
            Key::Character('w') if modifiers.logo => self.finished = true,
            Key::Enter => return self.confirm(),
            _ => {
                let handled = self.list.handle(&Event::Key { key, modifiers });
                self.drain_signals();
                return handled;
            }
        }
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
        if pointer.is_primary() && contains(self.button_frame(), at) {
            return self.confirm();
        }
        if !contains(self.list.frame(), at) {
            return false;
        }
        let handled = self.list.handle(event);
        self.drain_signals();
        if is_secondary(pointer) && !self.apps.selection().is_empty() {
            self.request_menu(at);
            return true;
        }
        handled
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

impl Screen for ForceQuitView {
    fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        let frame = self.box_frame();
        self.list.set_frame(frame.inset(LIST_INSET));
        let list = self.list.paint(width, height);
        let palette = self.palette;
        let button = self.button_frame();
        let selected = !self.apps.selection().is_empty();
        let empty = self.apps.is_empty();
        let mut painter = Painter {
            text: self.list.text(),
            palette,
            items: Vec::new(),
        };
        painter.text_in(
            &L10N.tr("force-quit-hint"),
            TextStyle::new(TEXT_SIZE - 1.0, palette.ink2),
            Rect::new(PADDING, 0.0, width as f32 - 2.0 * PADDING, HINT_HEIGHT),
            TextAlign::Start,
        );
        let shape = Shape::rounded(frame, 8.0);
        painter.fill(shape, palette.overlay);
        painter.items.push(Item::Stroke {
            shape,
            width: 1.0,
            paint: palette.hairline.into(),
        });
        if empty {
            painter.text_in(
                &L10N.tr("force-quit-empty"),
                TextStyle::new(TEXT_SIZE, palette.ink2),
                frame,
                TextAlign::Center,
            );
        }
        painter.items.extend(list);
        painter.button(button, &L10N.tr("button-force-quit"), selected);
        if let Some(dialog) = &self.dialog {
            painter.dialog(self.size, dialog, false);
        }
        Scene {
            width,
            height,
            background: palette.sheet,
            items: painter.items,
        }
    }

    fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.list.set_palette(palette);
    }

    fn set_focused(&mut self, focused: bool) {
        match focused {
            true => self.list.focus(),
            false => self.list.blur(),
        }
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
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
