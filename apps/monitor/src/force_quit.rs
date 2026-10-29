use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::model::AppList;
use crate::sampler::Snapshot;
use crate::view::{
    Dialog, DialogButton, Icons, Painter, Request, Screen, contains, dialog_click, dialog_key,
    press_dialog_button,
};

const PADDING: f32 = 16.0;
const HINT_HEIGHT: f32 = 44.0;
const ROW_HEIGHT: f32 = 32.0;
const ICON_SIZE: f32 = 22.0;
const BUTTON_WIDTH: f32 = 104.0;
const BUTTON_HEIGHT: f32 = 26.0;
const FOOTER_HEIGHT: f32 = 58.0;
const TEXT_SIZE: f32 = 13.0;

pub struct ForceQuitView {
    apps: AppList,
    palette: Palette,
    text: TextSystem,
    icons: Icons,
    size: (f32, f32),
    top: usize,
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
            top: 0,
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
        let visible = self.visible_rows();
        if let Some(index) = self.apps.selected_index() {
            if index < self.top {
                self.top = index;
            } else if index >= self.top + visible {
                self.top = index + 1 - visible;
            }
        }
        self.top = self.top.min(self.apps.len().saturating_sub(visible));
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
        let list = self.list_frame();
        if !contains(list, at) {
            return false;
        }
        let index = self.top + ((at.y - list.y) / ROW_HEIGHT) as usize;
        if index >= self.apps.len() {
            return false;
        }
        self.apps.select(index);
        true
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
            .skip(self.top)
            .take(self.visible_rows() + 1)
            .map(|(index, (name, icon))| (index, name.to_owned(), icon.map(str::to_owned)))
            .collect::<Vec<_>>();
        for (index, name, icon) in shown {
            let y = list.y + (index - self.top) as f32 * ROW_HEIGHT;
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
            rows.text_in(
                &name,
                TextStyle::new(TEXT_SIZE, color),
                Rect::new(
                    row.x + 16.0 + ICON_SIZE,
                    row.y,
                    row.width - 24.0 - ICON_SIZE,
                    row.height,
                ),
                TextAlign::Start,
            );
        }
        let items = rows.items;
        painter.items.push(Item::Clip { shape, items });
        painter.button(
            self.button_frame(),
            &L10N.tr("button-force-quit"),
            selected.is_some(),
        );
        if let Some(dialog) = self.dialog.clone() {
            painter.dialog(self.size, &dialog);
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
            Event::PointerDown(at) => self.click(*at),
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

    fn finished(&self) -> bool {
        self.finished
    }
}
