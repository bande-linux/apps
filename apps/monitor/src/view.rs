use std::collections::HashMap;
use std::path::PathBuf;

use unbound_toolkit::ui::{Event, Key, Menu, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

use crate::L10N;
use crate::menu::{MenuCommand, Selected, process_menu};
use crate::model::{Column, Disclosure, Graph, Monitor, Row, StopTarget, Tab, stop_target};
use crate::sampler::Snapshot;
use crate::signal::{SignalError, Stop};

pub const TOOLBAR_HEIGHT: f32 = 48.0;
pub const HEADER_HEIGHT: f32 = 26.0;
pub const ROW_HEIGHT: f32 = 22.0;
pub const FOOTER_HEIGHT: f32 = 128.0;
const SIDE: f32 = 12.0;
const TEXT_SIZE: f32 = 12.0;
const SMALL_SIZE: f32 = 11.0;
const ICON_SIZE: f32 = 16.0;
const INDENT: f32 = 18.0;
const DISCLOSURE_WIDTH: f32 = 14.0;
const SEGMENT_WIDTH: f32 = 84.0;
const CONTROL_HEIGHT: f32 = 26.0;
const SEARCH_WIDTH: f32 = 200.0;
const STOP_SIZE: f32 = 28.0;
const GRAPH_WIDTH: f32 = 240.0;
const DIALOG_WIDTH: f32 = 400.0;
const DIALOG_HEIGHT: f32 = 150.0;
const BUTTON_HEIGHT: f32 = 26.0;
const BUTTON_GAP: f32 = 8.0;
const DIALOG_PADDING: f32 = 20.0;
const WHITE: Color = Color::hex(0xFFFFFF);
const SYSTEM_COLOR: Color = Color::hex(0xE5484D);
const USER_COLOR: Color = Color::hex(0x3E8EF7);
const MEMORY_COLOR: Color = Color::hex(0x30A46C);

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

pub fn column_width(column: Column) -> f32 {
    match column {
        Column::Name => 0.0,
        Column::Cpu => 64.0,
        Column::Pid => 64.0,
        Column::Threads => 68.0,
        Column::CpuTime => 84.0,
        Column::User => 96.0,
        _ => 108.0,
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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scroller {
    offset: f32,
}

impl Scroller {
    pub fn first(self, row_height: f32) -> usize {
        (self.offset / row_height).floor() as usize
    }

    pub fn row_y(self, index: usize, row_height: f32) -> f32 {
        index as f32 * row_height - self.offset
    }

    pub fn index_at(self, y: f32, row_height: f32) -> usize {
        ((y + self.offset) / row_height).floor().max(0.0) as usize
    }

    pub fn reset(&mut self) {
        self.offset = 0.0;
    }

    pub fn scroll(&mut self, delta: f32, rows: usize, row_height: f32, height: f32) {
        self.offset += delta;
        self.clamp(rows, row_height, height);
    }

    pub fn clamp(&mut self, rows: usize, row_height: f32, height: f32) {
        let end = (rows as f32 * row_height - height).max(0.0);
        self.offset = self.offset.clamp(0.0, end);
    }

    pub fn reveal(&mut self, index: usize, rows: usize, row_height: f32, height: f32) {
        let top = index as f32 * row_height;
        if top < self.offset {
            self.offset = top;
        } else if top + row_height > self.offset + height {
            self.offset = top + row_height - height;
        }
        self.clamp(rows, row_height, height);
    }
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
            L10N.tr_args("quit-title", [("name", target.name.clone().into())]),
            L10N.tr("quit-message"),
            dialog_buttons(dialog),
        ),
        Dialog::ConfirmForce(target) => (
            L10N.tr_args("force-quit-confirm", [("name", target.name.clone().into())]),
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

    fn set_responding(&mut self, _pid: u32, _app_id: &str, _responding: bool) -> bool {
        false
    }
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

pub struct MonitorView {
    pub monitor: Monitor,
    palette: Palette,
    text: TextSystem,
    icons: Icons,
    size: (f32, f32),
    scroller: Scroller,
    searching: bool,
    focused: bool,
    alternate: bool,
    dialog: Option<Dialog>,
    requests: Vec<Request>,
}

impl MonitorView {
    pub fn new(monitor: Monitor, palette: Palette, text: TextSystem, icons: Icons) -> Self {
        Self {
            monitor,
            palette,
            text,
            icons,
            size: (0.0, 0.0),
            scroller: Scroller::default(),
            searching: false,
            focused: true,
            alternate: false,
            dialog: None,
            requests: Vec::new(),
        }
    }

    fn list_height(&self) -> f32 {
        (self.size.1 - TOOLBAR_HEIGHT - HEADER_HEIGHT - FOOTER_HEIGHT).max(ROW_HEIGHT)
    }

    fn visible_rows(&self) -> usize {
        ((self.list_height() / ROW_HEIGHT).floor() as usize).max(1)
    }

    fn reveal_selection(&mut self) {
        let rows = self.monitor.rows().len();
        let height = self.list_height();
        match self.monitor.selected_index() {
            Some(index) => self.scroller.reveal(index, rows, ROW_HEIGHT, height),
            None => self.scroller.clamp(rows, ROW_HEIGHT, height),
        }
    }

    fn clamp_scroll(&mut self) {
        let rows = self.monitor.rows().len();
        let height = self.list_height();
        self.scroller.clamp(rows, ROW_HEIGHT, height);
    }

    fn set_tab(&mut self, tab: Tab) {
        let measured = self.monitor.measures_proportional_memory();
        self.monitor.set_tab(tab);
        if self.monitor.measures_proportional_memory() != measured {
            self.requests.push(Request::Resample);
        }
        self.scroller.reset();
        self.reveal_selection();
    }

    fn open_stop_dialog(&mut self) -> bool {
        match self.monitor.stop_target() {
            Some(target) => {
                self.dialog = Some(Dialog::Confirm(target));
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

    fn open_force_dialog(&mut self) -> bool {
        match self.monitor.stop_target() {
            Some(target) => {
                self.dialog = Some(Dialog::ConfirmForce(target));
                true
            }
            None => false,
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
            has_pid: self.monitor.selected_pid().is_some(),
        };
        self.requests
            .push(Request::Menu(process_menu(selected), at));
    }

    fn menu_from_keyboard(&mut self) -> bool {
        let Some(index) = self.monitor.selected_index() else {
            return false;
        };
        self.reveal_selection();
        let top = TOOLBAR_HEIGHT + HEADER_HEIGHT + self.scroller.row_y(index, ROW_HEIGHT);
        self.request_menu(Point::new(SIDE + 40.0, top + ROW_HEIGHT));
        true
    }

    fn search_key(&mut self, key: Key, modifiers: Modifiers) -> Option<bool> {
        if !self.searching || modifiers.logo || modifiers.control {
            return None;
        }
        let mut search = self.monitor.search().to_owned();
        match key {
            Key::Escape if search.is_empty() => self.searching = false,
            Key::Escape => search.clear(),
            Key::Backspace => {
                search.pop();
            }
            Key::Enter | Key::Down | Key::Tab => {
                self.searching = false;
                if self.monitor.selection().is_none() {
                    self.monitor.move_selection(1);
                }
                return Some(true);
            }
            Key::Character(character) => search.push(character),
            Key::Space => search.push(' '),
            _ => return Some(false),
        }
        self.monitor.set_search(&search);
        self.scroller.reset();
        self.reveal_selection();
        Some(true)
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
        let page = self.visible_rows().saturating_sub(1).max(1) as isize;
        match key {
            Key::Character(digit @ '1'..='4') if modifiers.logo => {
                let index = digit as usize - '1' as usize;
                self.set_tab(Tab::ALL[index]);
                return true;
            }
            Key::Character('f') if modifiers.logo => {
                self.searching = true;
                return true;
            }
            Key::Character('q' | 'Q') if modifiers.logo && modifiers.alt => {
                return self.open_stop_dialog();
            }
            Key::Character('r' | 'R') if modifiers.logo && modifiers.shift => {
                return self.show_in_files();
            }
            Key::Escape if !self.monitor.search().is_empty() => {
                self.monitor.set_search("");
            }
            Key::Up => self.monitor.move_selection(-1),
            Key::Down => self.monitor.move_selection(1),
            Key::PageUp => self.monitor.move_selection(-page),
            Key::PageDown => self.monitor.move_selection(page),
            Key::Home => self.monitor.select(0),
            Key::End => self
                .monitor
                .select(self.monitor.rows().len().saturating_sub(1)),
            Key::Right => return self.monitor.expand(),
            Key::Left => {
                if !self.monitor.collapse() {
                    return false;
                }
            }
            _ => return false,
        }
        self.reveal_selection();
        true
    }

    fn commit(&mut self, text: &str) -> bool {
        if !self.searching || self.dialog.is_some() {
            return false;
        }
        let search = format!("{}{text}", self.monitor.search());
        self.monitor.set_search(&search);
        self.scroller.reset();
        true
    }

    fn tab_frames(&self) -> Vec<(Tab, Rect)> {
        let total = SEGMENT_WIDTH * Tab::ALL.len() as f32;
        let left = ((self.size.0 - total) / 2.0).round();
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

    fn search_frame(&self) -> Rect {
        Rect::new(
            self.size.0 - SIDE - SEARCH_WIDTH,
            ((TOOLBAR_HEIGHT - CONTROL_HEIGHT) / 2.0).round(),
            SEARCH_WIDTH,
            CONTROL_HEIGHT,
        )
    }

    fn stop_frame(&self) -> Rect {
        Rect::new(
            SIDE,
            ((TOOLBAR_HEIGHT - STOP_SIZE) / 2.0).round(),
            STOP_SIZE,
            STOP_SIZE,
        )
    }

    fn column_frames(&self) -> Vec<(Column, f32, f32)> {
        let columns = self.monitor.tab().columns();
        let fixed = columns
            .iter()
            .map(|column| column_width(*column))
            .sum::<f32>();
        let name_width = (self.size.0 - 2.0 * SIDE - fixed).max(120.0);
        let mut x = SIDE;
        columns
            .iter()
            .map(|column| {
                let width = match column {
                    Column::Name => name_width,
                    other => column_width(*other),
                };
                let frame = (*column, x, width);
                x += width;
                frame
            })
            .collect()
    }

    fn row_at(&self, at: Point) -> Option<usize> {
        let list_top = TOOLBAR_HEIGHT + HEADER_HEIGHT;
        if at.y < list_top || at.y >= self.size.1 - FOOTER_HEIGHT {
            return None;
        }
        let index = self.scroller.index_at(at.y - list_top, ROW_HEIGHT);
        (index < self.monitor.rows().len()).then_some(index)
    }

    fn pointer_down(&mut self, pointer: &Pointer) -> bool {
        if is_secondary(pointer) {
            return match self.row_at(pointer.position) {
                Some(index) if self.dialog.is_none() => {
                    self.searching = false;
                    self.monitor.select(index);
                    self.request_menu(pointer.position);
                    true
                }
                _ => false,
            };
        }
        if !pointer.is_primary() {
            return false;
        }
        if pointer.count == 2
            && self.dialog.is_none()
            && let Some(index) = self.row_at(pointer.position)
        {
            if self.monitor.rows()[index].disclosure != Disclosure::None {
                self.monitor.toggle(index);
                self.clamp_scroll();
            }
            return true;
        }
        self.click(pointer.position)
    }

    fn scroll(&mut self, position: Point, delta: f32) -> bool {
        let list_top = TOOLBAR_HEIGHT + HEADER_HEIGHT;
        if self.dialog.is_some()
            || position.y < list_top
            || position.y >= self.size.1 - FOOTER_HEIGHT
        {
            return false;
        }
        let before = self.scroller;
        let rows = self.monitor.rows().len();
        let height = self.list_height();
        self.scroller.scroll(delta, rows, ROW_HEIGHT, height);
        self.scroller != before
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
        if at.y < TOOLBAR_HEIGHT {
            self.searching = contains(self.search_frame(), at);
            if contains(self.stop_frame(), at) {
                return self.open_stop_dialog();
            }
            if let Some((tab, _)) = self
                .tab_frames()
                .into_iter()
                .find(|(_, frame)| contains(*frame, at))
            {
                self.set_tab(tab);
            }
            return true;
        }
        self.searching = false;
        if at.y < TOOLBAR_HEIGHT + HEADER_HEIGHT {
            if let Some((column, _, _)) = self
                .column_frames()
                .into_iter()
                .find(|(_, x, width)| at.x >= *x && at.x < x + width)
            {
                self.monitor.sort_by(column);
            }
            return true;
        }
        let Some(index) = self.row_at(at) else {
            return true;
        };
        let row = &self.monitor.rows()[index];
        let disclosure_x = SIDE + f32::from(row.depth) * INDENT;
        if row.disclosure != Disclosure::None
            && at.x >= disclosure_x - 4.0
            && at.x < disclosure_x + DISCLOSURE_WIDTH + 4.0
        {
            self.monitor.toggle(index);
        } else {
            self.monitor.select(index);
        }
        self.clamp_scroll();
        true
    }

    fn paint_toolbar(&mut self, painter: &mut Painter) {
        let palette = self.palette;
        let stop = self.stop_frame();
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
        let frames = self.tab_frames();
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
        let search = self.search_frame();
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
        let query = self.monitor.search().to_owned();
        if query.is_empty() && !self.searching {
            painter.text_in(
                &L10N.tr("search-placeholder"),
                TextStyle::new(TEXT_SIZE, palette.label_muted),
                text_frame,
                TextAlign::Start,
            );
        } else {
            let paragraph = painter.paragraph(
                &query,
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
        painter.fill(
            Shape::rect(Rect::new(0.0, TOOLBAR_HEIGHT - 1.0, self.size.0, 1.0)),
            palette.separator_color,
        );
    }

    fn paint_header(&mut self, painter: &mut Painter) {
        let palette = self.palette;
        let (sorted, descending) = self.monitor.sort();
        let tab = self.monitor.tab();
        for (column, x, width) in self.column_frames() {
            let mut title = column.title(tab);
            if column == sorted {
                title = if column.numeric() {
                    format!(
                        "{} {title}",
                        if descending { "\u{25BE}" } else { "\u{25B4}" }
                    )
                } else {
                    format!(
                        "{title} {}",
                        if descending { "\u{25BE}" } else { "\u{25B4}" }
                    )
                };
            }
            let color = if column == sorted {
                palette.label_primary
            } else {
                palette.label_muted
            };
            let align = if column.numeric() {
                TextAlign::End
            } else {
                TextAlign::Start
            };
            let inset = if column == Column::Name {
                DISCLOSURE_WIDTH + ICON_SIZE + 8.0
            } else {
                8.0
            };
            let frame = match align {
                TextAlign::End => Rect::new(x, TOOLBAR_HEIGHT, width - 8.0, HEADER_HEIGHT),
                _ => Rect::new(x + inset, TOOLBAR_HEIGHT, width - inset, HEADER_HEIGHT),
            };
            painter.text_in(
                &title,
                TextStyle::new(SMALL_SIZE, color).weight(TextStyle::SEMIBOLD),
                frame,
                align,
            );
        }
        painter.fill(
            Shape::rect(Rect::new(
                0.0,
                TOOLBAR_HEIGHT + HEADER_HEIGHT - 1.0,
                self.size.0,
                1.0,
            )),
            palette.separator_color,
        );
    }

    fn paint_row(&mut self, painter: &mut Painter, row: &Row, y: f32, stripe: bool) {
        let palette = self.palette;
        let selected = self.monitor.selection() == Some(&row.key);
        let frame = Rect::new(SIDE / 2.0, y, self.size.0 - SIDE, ROW_HEIGHT);
        if selected {
            let color = if self.focused {
                palette.accent_color
            } else {
                palette.label_muted.with_alpha(56)
            };
            painter.fill(Shape::rounded(frame, 5.0), color);
        } else if stripe {
            painter.fill(
                Shape::rounded(frame, 5.0),
                palette.label_muted.with_alpha(14),
            );
        }
        let color = if selected && self.focused {
            WHITE
        } else {
            palette.label_primary
        };
        let muted = if selected && self.focused {
            WHITE.with_alpha(220)
        } else {
            palette.label_muted
        };
        for ((column, x, width), cell) in self.column_frames().into_iter().zip(&row.cells) {
            if column == Column::Name {
                let mut left = x + f32::from(row.depth) * INDENT;
                if row.disclosure != Disclosure::None {
                    let arrow = match row.disclosure {
                        Disclosure::Expanded => "\u{25BE}",
                        _ => "\u{25B8}",
                    };
                    painter.text_in(
                        arrow,
                        TextStyle::new(SMALL_SIZE, muted),
                        Rect::new(left, y, DISCLOSURE_WIDTH, ROW_HEIGHT),
                        TextAlign::Center,
                    );
                }
                left += DISCLOSURE_WIDTH + 2.0;
                if let Some(image) = row.icon.as_deref().and_then(|name| self.icons.get(name)) {
                    painter.items.push(Item::Image {
                        rect: Rect::new(
                            left,
                            y + ((ROW_HEIGHT - ICON_SIZE) / 2.0).round(),
                            ICON_SIZE,
                            ICON_SIZE,
                        ),
                        image,
                    });
                }
                left += ICON_SIZE + 6.0;
                painter.text_in(
                    cell,
                    TextStyle::new(TEXT_SIZE, color),
                    Rect::new(left, y, x + width - left - 8.0, ROW_HEIGHT),
                    TextAlign::Start,
                );
            } else {
                let (frame, align) = if column.numeric() {
                    (Rect::new(x, y, width - 8.0, ROW_HEIGHT), TextAlign::End)
                } else {
                    (
                        Rect::new(x + 8.0, y, width - 8.0, ROW_HEIGHT),
                        TextAlign::Start,
                    )
                };
                painter.text_in(cell, TextStyle::new(TEXT_SIZE, color), frame, align);
            }
        }
    }

    fn paint_rows(&mut self, painter: &mut Painter) {
        let top = TOOLBAR_HEIGHT + HEADER_HEIGHT;
        let area = Rect::new(0.0, top, self.size.0, self.size.1 - top - FOOTER_HEIGHT);
        let rows = self
            .monitor
            .rows()
            .iter()
            .enumerate()
            .skip(self.scroller.first(ROW_HEIGHT))
            .take(self.visible_rows() + 2)
            .map(|(index, row)| (index, row.clone()))
            .collect::<Vec<_>>();
        let mut list = Painter {
            text: painter.text,
            palette: painter.palette,
            items: Vec::new(),
        };
        for (index, row) in rows {
            let y = top + self.scroller.row_y(index, ROW_HEIGHT);
            self.paint_row(&mut list, &row, y, index % 2 == 1);
        }
        let items = list.items;
        painter.items.push(Item::Clip {
            shape: Shape::rect(area),
            items,
        });
    }

    fn paint_graph(&self, painter: &mut Painter, frame: Rect) {
        let palette = self.palette;
        painter.fill(Shape::rounded(frame, 6.0), palette.surface_elevated);
        let inner = frame.inset(6.0);
        let bar = inner.width / crate::model::HISTORY as f32;
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

    fn paint_footer(&mut self, painter: &mut Painter) {
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
        let summary = self.monitor.summary();
        let rows_per_column = 3;
        let column_width = 210.0;
        for (index, (label, value)) in summary.iter().enumerate() {
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
                TextStyle::new(TEXT_SIZE, palette.label_primary).weight(TextStyle::MEDIUM),
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
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    fn menu_action(&mut self, action: &str) -> bool {
        match MenuCommand::parse(action) {
            Some(MenuCommand::Quit) => self.open_stop_dialog(),
            Some(MenuCommand::ForceQuit) => self.open_force_dialog(),
            Some(MenuCommand::ShowInFiles) => self.show_in_files(),
            Some(MenuCommand::CopyName) => {
                self.requests
                    .extend(self.monitor.selected_name().map(Request::Copy));
                false
            }
            Some(MenuCommand::CopyPid) => {
                self.requests.extend(
                    self.monitor
                        .selected_pid()
                        .map(|pid| Request::Copy(pid.to_string())),
                );
                false
            }
            Some(MenuCommand::Find) => {
                self.searching = true;
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
            Event::PointerDown(pointer) => self.pointer_down(pointer),
            Event::Scroll(scroll) => self.scroll(scroll.position, scroll.delta.y),
            Event::ModifiersChanged(modifiers) => {
                let changed = self.alternate != modifiers.alt;
                self.alternate = modifiers.alt;
                changed && self.dialog.is_some()
            }
            _ => false,
        }
    }

    fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.clamp_scroll();
        let mut text = std::mem::take(&mut self.text);
        let mut painter = Painter {
            text: &mut text,
            palette: self.palette,
            items: Vec::new(),
        };
        self.paint_toolbar(&mut painter);
        self.paint_header(&mut painter);
        self.paint_rows(&mut painter);
        self.paint_footer(&mut painter);
        if let Some(dialog) = self.dialog.clone() {
            painter.dialog(self.size, &dialog, self.alternate);
        }
        let items = painter.items;
        self.text = text;
        Scene {
            width,
            height,
            background: self.palette.surface_background,
            items,
        }
    }

    fn update(&mut self, snapshot: Snapshot) {
        self.monitor.update(snapshot);
        self.clamp_scroll();
    }

    fn measures_proportional_memory(&self) -> bool {
        self.monitor.measures_proportional_memory()
    }

    fn finished(&self) -> bool {
        false
    }
}
