use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer};
use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::menu::{MenuCommand, force_quit_menu};
use crate::model::AppList;
use crate::sampler::Snapshot;
use crate::view::{
    Dialog, DialogButton, Icons, Painter, Request, Screen, Scroller, contains, dialog_click,
    dialog_key, is_secondary, opens_menu, press_dialog_button,
};

const PADDING: f32 = 16.0;
const HINT_HEIGHT: f32 = 44.0;
const ROW_HEIGHT: f32 = 32.0;
const ICON_SIZE: f32 = 22.0;
const BUTTON_WIDTH: f32 = 104.0;
const BUTTON_HEIGHT: f32 = 26.0;
const FOOTER_HEIGHT: f32 = 58.0;
const TEXT_SIZE: f32 = 13.0;
const UNRESPONSIVE: Color = Color::hex(0xE5484D);

pub struct ForceQuitView {
    apps: AppList,
    palette: Palette,
    text: TextSystem,
    icons: Icons,
    size: (f32, f32),
    scroller: Scroller,
    focused: bool,
    dialog: Option<Dialog>,
    requests: Vec<Request>,
    finished: bool,
}

impl ForceQuitView {
    pub fn new(apps: AppList, palette: Palette, text: TextSystem, icons: Icons) -> Self {
        Self {
            apps,
            palette,
            text,
            icons,
            size: (0.0, 0.0),
            scroller: Scroller::default(),
            focused: true,
            dialog: None,
            requests: Vec::new(),
            finished: false,
        }
    }

    fn list_frame(&self) -> Rect {
        Rect::new(
            PADDING,
            HINT_HEIGHT,
            self.size.0 - 2.0 * PADDING,
            self.size.1 - HINT_HEIGHT - FOOTER_HEIGHT,
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

    fn visible_rows(&self) -> usize {
        ((self.list_frame().height / ROW_HEIGHT).floor() as usize).max(1)
    }

    fn reveal_selection(&mut self) {
        let rows = self.apps.len();
        let height = self.list_frame().height;
        match self.apps.selected_index() {
            Some(index) => self.scroller.reveal(index, rows, ROW_HEIGHT, height),
            None => self.scroller.clamp(rows, ROW_HEIGHT, height),
        }
    }

    fn row_at(&self, at: Point) -> Option<usize> {
        let list = self.list_frame();
        if !contains(list, at) {
            return None;
        }
        let index = self.scroller.index_at(at.y - list.y, ROW_HEIGHT);
        (index < self.apps.len()).then_some(index)
    }

    fn request_menu(&mut self, at: Point) {
        let menu = force_quit_menu(self.apps.selected_entry().is_some());
        self.requests.push(Request::Menu(menu, at));
    }

    fn pointer_down(&mut self, pointer: &Pointer) -> bool {
        if is_secondary(pointer) {
            return match self.row_at(pointer.position) {
                Some(index) if self.dialog.is_none() => {
                    self.apps.select(index);
                    self.request_menu(pointer.position);
                    true
                }
                _ => false,
            };
        }
        if !pointer.is_primary() {
            return false;
        }
        if pointer.count == 2 && self.dialog.is_none() && self.row_at(pointer.position).is_some() {
            return self.confirm();
        }
        self.click(pointer.position)
    }

    fn scroll(&mut self, position: Point, delta: f32) -> bool {
        if self.dialog.is_some() || !contains(self.list_frame(), position) {
            return false;
        }
        let before = self.scroller;
        let rows = self.apps.len();
        let height = self.list_frame().height;
        self.scroller.scroll(delta, rows, ROW_HEIGHT, height);
        self.scroller != before
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
            let Some(index) = self.apps.selected_index() else {
                return false;
            };
            self.reveal_selection();
            let list = self.list_frame();
            let bottom = list.y + self.scroller.row_y(index + 1, ROW_HEIGHT);
            self.request_menu(Point::new(list.x + 16.0 + ICON_SIZE, bottom));
            return true;
        }
        match key {
            Key::Escape => self.finished = true,
            Key::Character('w') if modifiers.logo => self.finished = true,
            Key::Enter => return self.confirm(),
            Key::Up => self.apps.move_selection(-1),
            Key::Down => self.apps.move_selection(1),
            Key::Home => self.apps.select(0),
            Key::End => self.apps.select(self.apps.len().saturating_sub(1)),
            _ => return false,
        }
        self.reveal_selection();
        true
    }

    fn click(&mut self, at: Point) -> bool {
        if let Some(dialog) = &self.dialog {
            return match dialog_click(dialog, self.size, at) {
                Some(button) => {
                    self.press(button);
                    true
                }
                None => false,
            };
        }
        if contains(self.button_frame(), at) {
            return self.confirm();
        }
        match self.row_at(at) {
            Some(index) => {
                self.apps.select(index);
                true
            }
            None => false,
        }
    }
}

impl Screen for ForceQuitView {
    fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.reveal_selection();
        let palette = self.palette;
        let mut text = std::mem::take(&mut self.text);
        let mut painter = Painter {
            text: &mut text,
            palette,
            items: Vec::new(),
        };
        painter.text_in(
            &L10N.tr("force-quit-hint"),
            TextStyle::new(TEXT_SIZE - 1.0, palette.label_muted),
            Rect::new(PADDING, 0.0, self.size.0 - 2.0 * PADDING, HINT_HEIGHT),
            TextAlign::Start,
        );
        let list = self.list_frame();
        let shape = Shape::rounded(list, 8.0);
        painter.fill(shape, palette.surface_elevated);
        painter.items.push(Item::Stroke {
            shape,
            width: 1.0,
            paint: palette.separator_color.into(),
        });
        let selected = self.apps.selected_index();
        let mut rows = Painter {
            text: painter.text,
            palette,
            items: Vec::new(),
        };
        if self.apps.is_empty() {
            rows.text_in(
                &L10N.tr("force-quit-empty"),
                TextStyle::new(TEXT_SIZE, palette.label_muted),
                list,
                TextAlign::Center,
            );
        }
        let shown = self
            .apps
            .rows()
            .enumerate()
            .skip(self.scroller.first(ROW_HEIGHT))
            .take(self.visible_rows() + 2)
            .map(|(index, row)| {
                (
                    index,
                    row.name.to_owned(),
                    row.icon.map(str::to_owned),
                    row.responding,
                )
            })
            .collect::<Vec<_>>();
        for (index, name, icon, responding) in shown {
            let y = list.y + self.scroller.row_y(index, ROW_HEIGHT);
            let row = Rect::new(list.x + 4.0, y + 2.0, list.width - 8.0, ROW_HEIGHT - 4.0);
            let is_selected = selected == Some(index);
            if is_selected {
                let color = if self.focused {
                    palette.accent_color
                } else {
                    palette.label_muted.with_alpha(56)
                };
                rows.fill(Shape::rounded(row, 6.0), color);
            }
            if let Some(image) = icon.as_deref().and_then(|icon| self.icons.get(icon)) {
                rows.items.push(Item::Image {
                    rect: Rect::new(
                        row.x + 8.0,
                        y + ((ROW_HEIGHT - ICON_SIZE) / 2.0).round(),
                        ICON_SIZE,
                        ICON_SIZE,
                    ),
                    image,
                });
            }
            let color = if is_selected && self.focused {
                Color::hex(0xFFFFFF)
            } else {
                palette.label_primary
            };
            let text = Rect::new(
                row.x + 16.0 + ICON_SIZE,
                row.y,
                row.width - 24.0 - ICON_SIZE,
                row.height,
            );
            if responding {
                rows.text_in(
                    &name,
                    TextStyle::new(TEXT_SIZE, color),
                    text,
                    TextAlign::Start,
                );
            } else {
                let marker = if is_selected && self.focused {
                    color
                } else {
                    UNRESPONSIVE
                };
                let style = TextStyle::new(TEXT_SIZE, color);
                let name_width = rows
                    .paragraph(&format!("{name} "), style, None, TextAlign::Start)
                    .width()
                    .min(text.width);
                rows.text_in(
                    &format!("{name} "),
                    style,
                    Rect::new(text.x, text.y, name_width, text.height),
                    TextAlign::Start,
                );
                rows.text_in(
                    &L10N.tr("force-quit-not-responding"),
                    TextStyle::new(TEXT_SIZE, marker),
                    Rect::new(
                        text.x + name_width,
                        text.y,
                        (text.width - name_width).max(0.0),
                        text.height,
                    ),
                    TextAlign::Start,
                );
            }
        }
        let items = rows.items;
        painter.items.push(Item::Clip { shape, items });
        painter.button(
            self.button_frame(),
            &L10N.tr("button-force-quit"),
            selected.is_some(),
        );
        if let Some(dialog) = self.dialog.clone() {
            painter.dialog(self.size, &dialog, false);
        }
        let items = painter.items;
        self.text = text;
        Scene {
            width,
            height,
            background: palette.surface_background,
            items,
        }
    }

    fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::PointerDown(pointer) => self.pointer_down(pointer),
            Event::Scroll(scroll) => self.scroll(scroll.position, scroll.delta.y),
            _ => false,
        }
    }

    fn update(&mut self, snapshot: Snapshot) {
        self.apps.update(snapshot);
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
        self.apps.set_responding(pid, app_id, responding)
    }

    fn finished(&self) -> bool {
        self.finished
    }
}
