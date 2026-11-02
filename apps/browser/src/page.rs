use std::cell::Cell;
use std::sync::Arc;

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Context, Cursor, Event, Key, PointerButton, ScrollState, Size, Widget, WidgetId,
};
use bande_toolkit::{
    Color, GlyphRun, Item, Picture, Point, PositionedGlyph, Rect, Shape, TextAlign, TextStyle,
};
use web_content::Viewport;
use web_net::Url;
use web_paint::DisplayList;

use crate::L10N;
use crate::renderer::Links;

const MARGIN: f32 = 32.0;
const LINE_STEP: f32 = 40.0;
const PAGE_OVERLAP: f32 = 40.0;
const TITLE_SIZE: f32 = 22.0;
const BODY_SIZE: f32 = 15.0;
const MESSAGE_WIDTH: f32 = 560.0;
const TILE_HEIGHT: f32 = 1024.0;
const GLYPH_REACH: f32 = 0.5;
const CANVAS: Color = Color::WHITE;
const TEXT: Color = Color::hex(0x202122);
const SECONDARY: Color = Color::hex(0x54595d);

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
    tiles: Vec<Picture>,
    height: f32,
    scroll: ScrollState,
    jump: Option<f32>,
    pressed: Option<Point>,
    followed: Option<Url>,
    hovered: Option<(f32, f32)>,
    shown_offset: Cell<f32>,
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
            tiles: Vec::new(),
            height: 0.0,
            scroll: ScrollState::default(),
            jump: None,
            pressed: None,
            followed: None,
            hovered: None,
            shown_offset: Cell::new(0.0),
        }
    }

    pub(crate) fn show(&mut self, content: Content) {
        self.content = content;
        self.laid_out_width = None;
        self.scroll = ScrollState::default();
        self.shown_offset.set(0.0);
        self.jump = None;
    }

    pub(crate) fn relaid(&mut self, list: Arc<DisplayList>, links: Arc<Links>) {
        if matches!(self.content, Content::Document { .. }) {
            self.content = Content::Document { list, links };
            self.laid_out_width = None;
        }
    }

    pub(crate) fn has_document(&self) -> bool {
        matches!(self.content, Content::Document { .. })
    }

    pub(crate) fn jump_to(&mut self, y: f32) {
        self.jump = Some(y);
    }

    pub(crate) fn offset(&self) -> f32 {
        self.shown_offset.get()
    }

    pub(crate) fn viewport(&self) -> Option<Viewport> {
        (self.frame.width > 0.0 && self.frame.height > 0.0).then_some(Viewport {
            width: self.frame.width,
            height: self.frame.height,
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
            Some(point) => (
                point.x - self.frame.x,
                point.y - self.frame.y + self.shown_offset.get(),
            ),
            None => (-1.0, -1.0),
        });
    }

    pub(crate) fn take_followed(&mut self) -> Option<Url> {
        self.followed.take()
    }

    pub(crate) fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
    }

    fn link_at(&self, point: Point) -> Option<&Url> {
        let Content::Document { links, .. } = &self.content else {
            return None;
        };
        links.at(
            point.x - self.frame.x,
            point.y - self.frame.y + self.shown_offset.get(),
        )
    }

    fn lay_out(&mut self, cx: &mut Context, width: f32) {
        self.laid_out_width = Some(width);
        self.message.clear();
        self.tiles.clear();
        if let Content::Document { list, .. } = &self.content {
            self.tiles = tiles(list);
            self.height = list.height;
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
                self.followed = self.link_at(pointer.position).cloned();
                self.followed.is_some()
            }
            _ => false,
        }
    }
}

fn color(color: web_paint::Color) -> Color {
    Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    }
}

fn converted(list: &DisplayList, item: &web_paint::Item) -> Option<(f32, f32, Item)> {
    match item {
        web_paint::Item::Rect { rect, color: fill } => Some((
            rect.y,
            rect.y + rect.height,
            Item::Fill {
                shape: Shape::rect(Rect::new(rect.x, rect.y, rect.width, rect.height)),
                paint: color(*fill).into(),
            },
        )),
        web_paint::Item::Glyphs {
            rect,
            baseline,
            font,
            size,
            color: ink,
            glyphs,
            ..
        } => {
            let font = list.fonts.get(*font as usize)?.clone();
            let glyphs = list
                .glyphs(glyphs)
                .iter()
                .map(|glyph| {
                    let (id, x, y) = glyph.translated(0.0, *baseline);
                    PositionedGlyph { id, x, y }
                })
                .collect();
            let reach = size * GLYPH_REACH;
            Some((
                rect.y - reach,
                rect.y + rect.height + reach,
                Item::Glyphs(GlyphRun::new(font, *size, color(*ink), glyphs)),
            ))
        }
    }
}

fn band(y: f32, count: usize) -> usize {
    ((y / TILE_HEIGHT).floor().max(0.0) as usize).min(count.saturating_sub(1))
}

fn tiles(list: &DisplayList) -> Vec<Picture> {
    let count = ((list.height / TILE_HEIGHT).ceil() as usize).max(1);
    let mut bands: Vec<Vec<Item>> = vec![Vec::new(); count];
    for (top, bottom, item) in list.items.iter().filter_map(|item| converted(list, item)) {
        let (first, last) = (band(top, count), band(bottom, count));
        for items in &mut bands[first..=last] {
            items.push(item.clone());
        }
    }
    bands
        .into_iter()
        .enumerate()
        .map(|(index, items)| {
            let top = index as f32 * TILE_HEIGHT;
            Picture::new(vec![Item::Clip {
                shape: Shape::rect(Rect::new(0.0, top, list.width, TILE_HEIGHT)),
                items,
            }])
        })
        .collect()
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
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        self.shown_offset.set(offset);
        let visible = match self.tiles.len() {
            0 => self.message.clone(),
            count => self.tiles[band(offset, count)..=band(offset + self.frame.height, count)]
                .iter()
                .cloned()
                .map(Item::Picture)
                .collect(),
        };
        let content = vec![
            Item::Fill {
                shape: Shape::rect(self.frame),
                paint: CANVAS.into(),
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
        Some(match self.link_at(point) {
            Some(_) => Cursor::Pointer,
            None => Cursor::Default,
        })
    }
}
