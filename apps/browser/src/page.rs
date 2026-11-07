use std::cell::Cell;
use std::sync::Arc;

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Context, Cursor, Event, Key, PointerButton, ScrollState, Size, Widget, WidgetId,
};
use bande_toolkit::{Color, Item, Point, Rect, Shape, TextAlign, TextStyle};
use web_host::Url;
use web_host::Viewport;
use web_paint::DisplayList;

use crate::L10N;
use crate::renderer::Links;
use crate::tiles::{Images, Tiles};

const MARGIN: f32 = 32.0;
const LINE_STEP: f32 = 40.0;
const PAGE_OVERLAP: f32 = 40.0;
const TITLE_SIZE: f32 = 22.0;
const BODY_SIZE: f32 = 15.0;
const MESSAGE_WIDTH: f32 = 560.0;
const CANVAS: Color = Color::WHITE;
const TEXT: Color = Color::hex(0x202122);
const SECONDARY: Color = Color::hex(0x54595d);
const BUTTON: Color = Color::hex(0xeaecf0);
const BUTTON_PADDING: f32 = 14.0;
const BUTTON_RADIUS: f32 = 8.0;

pub(crate) enum Content {
    Blank,
    Loading(Url),
    Failed {
        url: Url,
        message: String,
    },
    Unsupported {
        url: Url,
        kind: String,
    },
    Crashed(Url),
    Insecure {
        https: Url,
        http: Url,
        message: String,
    },
    Document {
        list: Arc<DisplayList>,
        links: Arc<Links>,
    },
}

pub(crate) struct PageView {
    id: WidgetId,
    frame: Rect,
    radius: f32,
    content: Content,
    laid_out_width: Option<f32>,
    message: Vec<Item>,
    tiles: Option<Tiles>,
    images: Images,
    height: f32,
    scroll: ScrollState,
    jump: Option<f32>,
    pressed: Option<Point>,
    followed: Option<Url>,
    continued: Option<Url>,
    reload: Option<Rect>,
    reload_pressed: bool,
    hovered: Option<(f32, f32)>,
    shown_offset: Cell<f32>,
    zoom: f32,
}

impl PageView {
    pub(crate) fn new() -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            radius: 0.0,
            content: Content::Blank,
            laid_out_width: None,
            message: Vec::new(),
            tiles: None,
            images: Images::default(),
            height: 0.0,
            scroll: ScrollState::default(),
            jump: None,
            pressed: None,
            followed: None,
            continued: None,
            reload: None,
            reload_pressed: false,
            hovered: None,
            shown_offset: Cell::new(0.0),
            zoom: 1.0,
        }
    }

    pub(crate) fn show(&mut self, content: Content) {
        self.content = content;
        self.laid_out_width = None;
        self.scroll = ScrollState::default();
        self.shown_offset.set(0.0);
        self.jump = None;
    }

    pub(crate) fn relaid(
        &mut self,
        list: Arc<DisplayList>,
        links: Arc<Links>,
        damage: Option<web_paint::Rect>,
    ) {
        if !matches!(self.content, Content::Document { .. }) {
            return;
        }
        match (&mut self.tiles, damage) {
            (Some(tiles), Some(damage)) => {
                tiles.relaid(Arc::clone(&list), damage, &mut self.images)
            }
            _ => self.laid_out_width = None,
        }
        self.content = Content::Document { list, links };
    }

    pub(crate) fn has_document(&self) -> bool {
        matches!(self.content, Content::Document { .. })
    }

    pub(crate) fn jump_to(&mut self, y: f32) {
        self.jump = Some(y * self.document_zoom());
    }

    pub(crate) fn restore_offset(&mut self, offset: f32) {
        self.jump = Some(offset);
    }

    pub(crate) fn set_zoom(&mut self, zoom: f32) {
        if self.zoom != zoom {
            self.zoom = zoom;
            self.laid_out_width = None;
        }
    }

    fn document_zoom(&self) -> f32 {
        match self.content {
            Content::Document { .. } => self.zoom,
            _ => 1.0,
        }
    }

    fn document_point(&self, point: Point) -> (f32, f32) {
        let zoom = self.document_zoom();
        (
            (point.x - self.frame.x) / zoom,
            (point.y - self.frame.y + self.shown_offset.get()) / zoom,
        )
    }

    pub(crate) fn offset(&self) -> f32 {
        self.shown_offset.get()
    }

    pub(crate) fn frame_size(&self) -> Option<(f32, f32)> {
        (self.frame.width > 0.0 && self.frame.height > 0.0)
            .then_some((self.frame.width, self.frame.height))
    }

    pub(crate) fn viewport(&self) -> Option<Viewport> {
        (self.frame.width > 0.0 && self.frame.height > 0.0).then_some(Viewport {
            width: (self.frame.width / self.zoom).round(),
            height: (self.frame.height / self.zoom).round(),
        })
    }

    pub(crate) fn take_hovered(&mut self) -> Option<(f32, f32)> {
        self.hovered.take()
    }

    fn hover(&mut self, point: Option<Point>) {
        if !self.has_document() {
            return;
        }
        self.hovered = Some(match point {
            Some(point) => self.document_point(point),
            None => (-1.0, -1.0),
        });
    }

    pub(crate) fn take_followed(&mut self) -> Option<Url> {
        self.followed.take()
    }

    pub(crate) fn take_continued(&mut self) -> Option<Url> {
        self.continued.take()
    }

    pub(crate) fn take_reload(&mut self) -> bool {
        std::mem::take(&mut self.reload_pressed)
    }

    fn reload_at(&self, point: Point) -> bool {
        let x = point.x - self.frame.x;
        let y = point.y - self.frame.y + self.shown_offset.get();
        self.reload.is_some_and(|button| {
            x >= button.x
                && y >= button.y
                && x < button.x + button.width
                && y < button.y + button.height
        })
    }

    pub(crate) fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
    }

    fn link_at(&self, point: Point) -> Option<&Url> {
        let Content::Document { links, .. } = &self.content else {
            return None;
        };
        let (x, y) = self.document_point(point);
        links.at(x, y)
    }

    fn lay_out(&mut self, cx: &mut Context, width: f32) {
        self.laid_out_width = Some(width);
        self.message.clear();
        self.reload = None;
        self.tiles = None;
        if let Content::Document { list, .. } = &self.content {
            let tiles = Tiles::new(Arc::clone(list), &mut self.images);
            self.height = tiles.height() * self.zoom;
            self.tiles = Some(tiles);
            return;
        }
        let column = (width - 2.0 * MARGIN).clamp(0.0, MESSAGE_WIDTH);
        let left = ((width - column) / 2.0).max(MARGIN);
        let mut y = MARGIN + LINE_STEP;
        for (text, size, weight, color) in self.lines() {
            let style = TextStyle::new(size, color).weight(weight);
            let paragraph = cx
                .text
                .paragraph(&text, style, Some(column), TextAlign::Center);
            let height = paragraph.height();
            self.message.push(Item::Text {
                origin: Point::new(left, y),
                paragraph,
            });
            y += height + size * 0.6;
        }
        let action = match &self.content {
            Content::Crashed(_) => Some(L10N.tr("reload")),
            Content::Insecure { .. } => Some(L10N.tr("continue-insecure")),
            _ => None,
        };
        if let Some(action) = action {
            let label = cx.text.paragraph(
                &action,
                TextStyle::new(BODY_SIZE, TEXT).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            );
            let button = Rect::new(
                (width - label.painted_width()) / 2.0 - BUTTON_PADDING,
                y,
                label.painted_width() + 2.0 * BUTTON_PADDING,
                label.height() + BUTTON_PADDING,
            );
            self.message.push(Item::Fill {
                shape: Shape::rounded(button, BUTTON_RADIUS),
                paint: BUTTON.into(),
            });
            self.message.push(Item::Text {
                origin: Point::new(button.x + BUTTON_PADDING, y + BUTTON_PADDING / 2.0),
                paragraph: label,
            });
            self.reload = Some(button);
            y += button.height + BODY_SIZE;
        }
        self.height = y + MARGIN;
    }

    fn lines(&self) -> Vec<(String, f32, f32, Color)> {
        let title = |text: String| (text, TITLE_SIZE, TextStyle::SEMIBOLD, TEXT);
        let body = |text: String, color: Color| (text, BODY_SIZE, TextStyle::REGULAR, color);
        match &self.content {
            Content::Blank | Content::Document { .. } => Vec::new(),
            Content::Loading(url) => {
                vec![title(L10N.tr("loading")), body(url.to_string(), SECONDARY)]
            }
            Content::Failed { url, message } => vec![
                title(L10N.tr("cannot-open")),
                body(message.clone(), TEXT),
                body(url.to_string(), SECONDARY),
            ],
            Content::Unsupported { url, kind } => vec![
                title(L10N.tr("cannot-show")),
                body(kind.clone(), TEXT),
                body(url.to_string(), SECONDARY),
            ],
            Content::Crashed(url) => vec![
                title(L10N.tr("page-crashed")),
                body(L10N.tr("page-crashed-body"), TEXT),
                body(url.to_string(), SECONDARY),
            ],
            Content::Insecure {
                https,
                http,
                message,
            } => vec![
                title(L10N.tr("insecure-title")),
                body(
                    L10N.tr("insecure-body")
                        .replace("HOST", http.host_str().unwrap_or_default()),
                    TEXT,
                ),
                body(message.clone(), SECONDARY),
                body(https.to_string(), SECONDARY),
            ],
        }
    }

    fn scroll_by_page(&mut self, cx: &Context, pages: f32) -> bool {
        let page = (self.frame.height - PAGE_OVERLAP).max(LINE_STEP);
        self.scroll_by(cx, pages * page)
    }

    fn scroll_by(&mut self, cx: &Context, delta: f32) -> bool {
        let offset = self.scroll.offset(cx);
        self.scroll.scroll_to(cx, offset + delta)
    }

    fn click(&mut self, event: &Event) -> bool {
        match event {
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                self.hover(Some(pointer.position));
                false
            }
            Event::PointerLeave => {
                self.hover(None);
                false
            }
            Event::PointerDown(pointer) if pointer.button == PointerButton::Primary => {
                self.pressed = Some(pointer.position);
                false
            }
            Event::PointerUp(pointer) if pointer.button == PointerButton::Primary => {
                let Some(pressed) = self.pressed.take() else {
                    return false;
                };
                let moved =
                    (pressed.x - pointer.position.x).abs() + (pressed.y - pointer.position.y).abs();
                if moved > 4.0 {
                    return false;
                }
                if self.reload_at(pointer.position) {
                    match &self.content {
                        Content::Insecure { http, .. } => self.continued = Some(http.clone()),
                        _ => self.reload_pressed = true,
                    }
                    return true;
                }
                self.followed = self.link_at(pointer.position).cloned();
                self.followed.is_some()
            }
            _ => false,
        }
    }
}

impl Widget for PageView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        if self.laid_out_width != Some(frame.width) {
            self.lay_out(cx, frame.width);
        }
        self.scroll.set_extent(self.height, frame.height);
        if let Some(y) = self.jump.take() {
            self.scroll.follow(cx, y, y + frame.height);
        }
        let offset = self.scroll.offset(cx);
        let zoom = self.document_zoom();
        if let Some(tiles) = &mut self.tiles {
            tiles.prepare(
                (offset - frame.height) / zoom,
                (offset + 2.0 * frame.height) / zoom,
                &mut self.images,
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        self.shown_offset.set(offset);
        let zoom = self.document_zoom();
        let (visible, canvas) = match &self.tiles {
            Some(tiles) => {
                let shown = tiles.visible(offset / zoom, (offset + self.frame.height) / zoom);
                let shown = match zoom == 1.0 {
                    true => shown,
                    false => vec![Item::Scale {
                        origin: Point::new(0.0, 0.0),
                        factor: zoom,
                        items: shown,
                    }],
                };
                (shown, tiles.background())
            }
            None => (self.message.clone(), CANVAS),
        };
        let content = vec![
            Item::Fill {
                shape: Shape::rect(self.frame),
                paint: canvas.into(),
            },
            Item::Translate {
                offset: Point::new(self.frame.x, self.frame.y - offset),
                items: visible,
            },
        ];
        items.push(Item::Clip {
            shape: Shape::rounded(self.frame, self.radius),
            items: content,
        });
        self.scroll.paint_scrollbar(cx, self.frame, items);
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::ScrollView);
        self.scroll.describe(cx, &mut node);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        if self.scroll.pointer(cx, self.frame, event) {
            return true;
        }
        match event {
            Event::Scroll(scroll) => self.scroll.scroll(cx, scroll),
            Event::Key { key, modifiers } if !modifiers.control && !modifiers.alt => match key {
                Key::Space if modifiers.shift => self.scroll_by_page(cx, -1.0),
                Key::Space | Key::PageDown => self.scroll_by_page(cx, 1.0),
                Key::PageUp => self.scroll_by_page(cx, -1.0),
                Key::Down => self.scroll_by(cx, LINE_STEP),
                Key::Up => self.scroll_by(cx, -LINE_STEP),
                Key::Home => self.scroll.scroll_to(cx, 0.0),
                Key::End => self.scroll.scroll_to(cx, self.height),
                _ => false,
            },
            _ => self.click(event),
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        Some(
            match self.link_at(point).is_some() || self.reload_at(point) {
                true => Cursor::Pointer,
                false => Cursor::Default,
            },
        )
    }
}
