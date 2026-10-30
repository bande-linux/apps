use std::collections::HashMap;
use std::path::PathBuf;

use accesskit::{ActionRequest, TreeUpdate};
use unbound_toolkit::ui::{Cursor, Event, Key, Menu, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

use crate::L10N;
use crate::model::{StopTarget, stop_target};
use crate::sampler::Snapshot;
use crate::signal::{SignalError, Stop};

const TEXT_SIZE: f32 = 12.0;
const DIALOG_WIDTH: f32 = 400.0;
const DIALOG_HEIGHT: f32 = 150.0;
const BUTTON_HEIGHT: f32 = 26.0;
const BUTTON_GAP: f32 = 8.0;
const DIALOG_PADDING: f32 = 20.0;
const WHITE: Color = Color::hex(0xFFFFFF);
pub(crate) const NOT_RESPONDING: Color = Color::hex(0xE5484D);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogButton {
    Cancel,
    ForceQuit,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Confirm(StopTarget),
    ConfirmForce(StopTarget),
    Denied(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Resample,
    ShowInFiles(PathBuf),
    Copy(String),
    Menu(Menu, Point),
}

type FindIcon = Box<dyn Fn(&str) -> Option<Image>>;

pub struct Icons {
    loaded: HashMap<String, Option<Image>>,
    find: FindIcon,
}

impl Icons {
    pub fn new(find: impl Fn(&str) -> Option<Image> + 'static) -> Self {
        Self {
            loaded: HashMap::new(),
            find: Box::new(find),
        }
    }

    pub fn get(&mut self, name: &str) -> Option<Image> {
        if let Some(image) = self.loaded.get(name) {
            return image.clone();
        }
        let image = (self.find)(name);
        self.loaded.insert(name.to_owned(), image.clone());
        image
    }
}

pub fn button_frames(panel: Rect, buttons: &[DialogButton]) -> Vec<(DialogButton, Rect)> {
    let mut right = panel.x + panel.width - DIALOG_PADDING;
    let mut frames = Vec::new();
    for button in buttons.iter().rev() {
        let width = match button {
            DialogButton::ForceQuit => 96.0,
            _ => 76.0,
        };
        right -= width;
        frames.push((
            *button,
            Rect::new(
                right,
                panel.y + panel.height - DIALOG_PADDING - BUTTON_HEIGHT,
                width,
                BUTTON_HEIGHT,
            ),
        ));
        right -= BUTTON_GAP;
    }
    frames.reverse();
    frames
}

pub fn dialog_frame(size: (f32, f32)) -> Rect {
    Rect::new(
        ((size.0 - DIALOG_WIDTH) / 2.0).round(),
        ((size.1 - DIALOG_HEIGHT) / 3.0).round().max(0.0),
        DIALOG_WIDTH,
        DIALOG_HEIGHT,
    )
}

pub(crate) fn contains(rect: Rect, at: Point) -> bool {
    at.x >= rect.x && at.x < rect.x + rect.width && at.y >= rect.y && at.y < rect.y + rect.height
}

pub fn is_secondary(pointer: &Pointer) -> bool {
    pointer.button == PointerButton::Secondary
        || (pointer.button == PointerButton::Primary && pointer.modifiers.control)
}

pub fn opens_menu(key: Key, modifiers: Modifiers) -> bool {
    key == Key::Menu || (key == Key::Function(10) && modifiers.shift && !modifiers.logo)
}

pub struct Painter<'a> {
    pub text: &'a mut TextSystem,
    pub palette: Palette,
    pub items: Vec<Item>,
}

impl Painter<'_> {
    pub fn paragraph(
        &mut self,
        text: &str,
        style: TextStyle,
        width: Option<f32>,
        align: TextAlign,
    ) -> Paragraph {
        self.text.paragraph(text, style, width, align)
    }

    pub fn text_in(&mut self, text: &str, style: TextStyle, frame: Rect, align: TextAlign) {
        let paragraph = self.paragraph(text, style, Some(frame.width), align);
        let y = frame.y + ((frame.height - paragraph.height()) / 2.0).round();
        self.items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Text {
                origin: Point::new(frame.x, y),
                paragraph,
            }],
        });
    }

    pub fn fill(&mut self, shape: Shape, color: Color) {
        self.items.push(Item::Fill {
            shape,
            paint: color.into(),
        });
    }

    pub fn button(&mut self, frame: Rect, label: &str, default: bool) {
        let shape = Shape::rounded(frame, 6.0);
        let (fill, color) = if default {
            (self.palette.accent_color, WHITE)
        } else {
            (self.palette.surface_background, self.palette.label_primary)
        };
        self.items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 0.5),
            blur_radius: 1.5,
            color: Color::hex(0x000000).with_alpha(50),
        });
        self.fill(shape, fill);
        self.text_in(
            label,
            TextStyle::new(TEXT_SIZE + 1.0, color).weight(TextStyle::MEDIUM),
            frame,
            TextAlign::Center,
        );
    }

    pub fn dialog(&mut self, size: (f32, f32), dialog: &Dialog, alternate: bool) {
        let panel = dialog_frame(size);
        self.fill(
            Shape::rect(Rect::new(0.0, 0.0, size.0, size.1)),
            Color::hex(0x000000).with_alpha(40),
        );
        let shape = Shape::rounded(panel, 12.0);
        self.items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 8.0),
            blur_radius: 24.0,
            color: Color::hex(0x000000).with_alpha(90),
        });
        self.fill(shape, self.palette.surface_elevated);
        let (title, message, buttons) = dialog_content(dialog);
        let inner = panel.width - 2.0 * DIALOG_PADDING;
        let title = self.paragraph(
            &title,
            TextStyle::new(TEXT_SIZE + 2.0, self.palette.label_primary).weight(TextStyle::SEMIBOLD),
            Some(inner),
            TextAlign::Start,
        );
        let title_height = title.height();
        self.items.push(Item::Text {
            origin: Point::new(panel.x + DIALOG_PADDING, panel.y + DIALOG_PADDING),
            paragraph: title,
        });
        let message = self.paragraph(
            &message,
            TextStyle::new(TEXT_SIZE, self.palette.label_muted),
            Some(inner),
            TextAlign::Start,
        );
        self.items.push(Item::Text {
            origin: Point::new(
                panel.x + DIALOG_PADDING,
                panel.y + DIALOG_PADDING + title_height + 6.0,
            ),
            paragraph: message,
        });
        let default = default_button(dialog, alternate);
        for (button, frame) in button_frames(panel, buttons) {
            self.button(frame, &button_label(button, dialog), button == default);
        }
    }
}

fn button_label(button: DialogButton, dialog: &Dialog) -> String {
    match (button, dialog) {
        (DialogButton::Cancel, Dialog::Denied(_)) => L10N.tr("button-ok"),
        (DialogButton::Cancel, _) => L10N.tr("button-cancel"),
        (DialogButton::ForceQuit, _) => L10N.tr("button-force-quit"),
        (DialogButton::Quit, _) => L10N.tr("button-quit"),
    }
}

pub fn dialog_buttons(dialog: &Dialog) -> &'static [DialogButton] {
    match dialog {
        Dialog::Confirm(_) => &[
            DialogButton::Cancel,
            DialogButton::ForceQuit,
            DialogButton::Quit,
        ],
        Dialog::ConfirmForce(_) => &[DialogButton::Cancel, DialogButton::ForceQuit],
        Dialog::Denied(_) => &[DialogButton::Cancel],
    }
}

fn dialog_content(dialog: &Dialog) -> (String, String, &'static [DialogButton]) {
    match dialog {
        Dialog::Confirm(target) => (
            L10N.tr_args(
                "quit-title",
                [
                    ("name", target.name.clone().into()),
                    ("count", target.count.into()),
                ],
            ),
            L10N.tr("quit-message"),
            dialog_buttons(dialog),
        ),
        Dialog::ConfirmForce(target) => (
            L10N.tr_args(
                "force-quit-confirm",
                [
                    ("name", target.name.clone().into()),
                    ("count", target.count.into()),
                ],
            ),
            L10N.tr("force-quit-confirm-message"),
            dialog_buttons(dialog),
        ),
        Dialog::Denied(name) => (
            L10N.tr_args("quit-denied", [("name", name.clone().into())]),
            L10N.tr("quit-denied-message"),
            dialog_buttons(dialog),
        ),
    }
}

pub fn finish_stop(target: &StopTarget, how: Stop) -> Option<Dialog> {
    match stop_target(target, how) {
        Ok(()) => None,
        Err(SignalError::PermissionDenied) => Some(Dialog::Denied(target.name.clone())),
        Err(error) => {
            tracing::warn!("cannot stop {}: {error:?}", target.name);
            None
        }
    }
}

pub trait Screen {
    fn scene(&mut self, width: u32, height: u32) -> Scene;
    fn set_palette(&mut self, palette: Palette);
    fn set_focused(&mut self, focused: bool);
    fn handle(&mut self, event: &Event) -> bool;
    fn update(&mut self, snapshot: Snapshot);
    fn measures_proportional_memory(&self) -> bool;
    fn take_requests(&mut self) -> Vec<Request>;
    fn menu_action(&mut self, action: &str) -> bool;
    fn finished(&self) -> bool;

    fn set_responding(&mut self, pid: u32, app_id: &str, responding: bool) -> bool;
    fn accessibility(&self) -> Option<TreeUpdate>;
    fn perform(&mut self, request: &ActionRequest) -> bool;
    fn cursor(&self) -> Cursor;
    fn animating(&self) -> bool;
}

pub fn press_dialog_button(dialog: Dialog, button: DialogButton) -> Option<Dialog> {
    let how = match button {
        DialogButton::Cancel => return None,
        DialogButton::Quit => Stop::Quit,
        DialogButton::ForceQuit => Stop::ForceQuit,
    };
    match dialog {
        Dialog::Confirm(target) | Dialog::ConfirmForce(target) => finish_stop(&target, how),
        Dialog::Denied(_) => None,
    }
}

pub fn default_button(dialog: &Dialog, alternate: bool) -> DialogButton {
    match dialog {
        Dialog::Denied(_) => DialogButton::Cancel,
        Dialog::ConfirmForce(_) => DialogButton::ForceQuit,
        Dialog::Confirm(_) if alternate => DialogButton::ForceQuit,
        Dialog::Confirm(_) => DialogButton::Quit,
    }
}

pub fn dialog_key(dialog: &Dialog, key: Key, modifiers: Modifiers) -> Option<DialogButton> {
    match key {
        Key::Escape => Some(DialogButton::Cancel),
        Key::Enter => Some(default_button(dialog, modifiers.alt)),
        _ => None,
    }
}

pub fn dialog_click(dialog: &Dialog, size: (f32, f32), at: Point) -> Option<DialogButton> {
    button_frames(dialog_frame(size), dialog_buttons(dialog))
        .into_iter()
        .find(|(_, frame)| contains(*frame, at))
        .map(|(button, _)| button)
}
