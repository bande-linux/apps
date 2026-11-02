use std::cell::{Cell, RefCell};
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Context, Cursor, Event, Key, PointerButton, ScrollState, Size, Widget, WidgetId,
};
use bande_toolkit::{
    Color, GlyphRun, Item, Point, PositionedGlyph, Rect, Shape, TextAlign, TextStyle,
};
use web_content::{Fonts, Page};
use web_net::{FetchError, Response, Url};
use web_paint::DisplayList;

use crate::L10N;

const MARGIN: f32 = 32.0;
const LINE_STEP: f32 = 40.0;
const PAGE_OVERLAP: f32 = 40.0;
const TITLE_SIZE: f32 = 22.0;
const BODY_SIZE: f32 = 15.0;
const MESSAGE_WIDTH: f32 = 560.0;
const FIRST_WIDTH: f32 = 1024.0;
const CANVAS: Color = Color::WHITE;
const TEXT: Color = Color::hex(0x202122);
const SECONDARY: Color = Color::hex(0x54595d);

pub(crate) enum Content {
    Blank,
    Loading(Url),
    Failed { url: Url, message: String },
    Unsupported(Response),
    Document { url: Url, page: Box<Page> },
}

impl Content {
    pub(crate) fn failed(url: Url, error: &FetchError) -> Self {
        Self::Failed {
            url,
            message: error.to_string(),
        }
    }
}

struct Placed {
    top: f32,
    bottom: f32,
    item: Item,
}

pub(crate) struct PageView {
    id: WidgetId,
    frame: Rect,
    radius: f32,
    fonts: Rc<RefCell<Fonts>>,
    content: Content,
    laid_out_width: Option<f32>,
    placed: Vec<Placed>,
    height: f32,
    scroll: ScrollState,
    pressed: Option<Point>,
    followed: Option<Url>,
    shown_offset: Cell<f32>,
}

impl PageView {
    pub(crate) fn new(fonts: Rc<RefCell<Fonts>>) -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            radius: 0.0,
            fonts,
            content: Content::Blank,
            laid_out_width: None,
            placed: Vec::new(),
            height: 0.0,
            scroll: ScrollState::default(),
            pressed: None,
            followed: None,
            shown_offset: Cell::new(0.0),
        }
    }

    pub(crate) fn show(&mut self, content: Content) {
        self.content = content;
        self.laid_out_width = None;
        self.scroll = ScrollState::default();
    }

    pub(crate) fn show_response(&mut self, response: Response) {
        let html = response
            .content_type
            .as_ref()
            .is_none_or(|content_type| content_type.is_html());
        if !html {
            return self.show(Content::Unsupported(response));
        }
        let width = match self.frame.width {
            width if width > 0.0 => width,
            _ => FIRST_WIDTH,
        };
        let document = web_content::parse(&response.body);
        let page = Page::new(document, width, &mut self.fonts.borrow_mut());
        self.show(Content::Document {
            url: response.url,
            page: Box::new(page),
        });
    }

    pub(crate) fn title(&self) -> Option<String> {
        match &self.content {
            Content::Document { page, .. } => page.title().filter(|title| !title.is_empty()),
            _ => None,
        }
    }

    pub(crate) fn take_followed(&mut self) -> Option<Url> {
        self.followed.take()
    }

    pub(crate) fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
    }

    fn link_at(&self, point: Point) -> Option<Url> {
        let Content::Document { url, page } = &self.content else {
            return None;
        };
        let href = page.link_at(
            point.x - self.frame.x,
            point.y - self.frame.y + self.shown_offset.get(),
        )?;
        url.join(href.trim())
            .ok()
            .filter(|target| matches!(target.scheme(), "http" | "https"))
    }

    fn lay_out(&mut self, cx: &mut Context, width: f32) {
        self.placed.clear();
        if let Content::Document { page, .. } = &mut self.content {
            if page.display_list.width != width {
                page.resize(width, &mut self.fonts.borrow_mut());
            }
            self.placed = placed(&page.display_list);
            self.height = page.display_list.height;
            self.laid_out_width = Some(width);
            return;
        }
        let lines = self.message();
        let column = (width - 2.0 * MARGIN).clamp(0.0, MESSAGE_WIDTH);
        let left = ((width - column) / 2.0).max(MARGIN);
        let mut y = MARGIN + LINE_STEP;
        for (text, size, weight, color) in lines {
            let style = TextStyle::new(size, color).weight(weight);
            let paragraph = cx
                .text
                .paragraph(&text, style, Some(column), TextAlign::Center);
            let height = paragraph.height();
            self.placed.push(Placed {
                top: y,
                bottom: y + height,
                item: Item::Text {
                    origin: Point::new(left, y),
                    paragraph,
                },
            });
            y += height + size * 0.6;
        }
        self.height = y + MARGIN;
        self.laid_out_width = Some(width);
    }

    fn message(&self) -> Vec<(String, f32, f32, Color)> {
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
            Content::Unsupported(response) => vec![
                title(L10N.tr("cannot-show")),
                body(
                    response
                        .content_type
                        .as_ref()
                        .map(|content_type| content_type.essence.clone())
                        .unwrap_or_default(),
                    TEXT,
                ),
                body(response.url.to_string(), SECONDARY),
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
                self.followed = self.link_at(pointer.position);
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

fn placed(list: &DisplayList) -> Vec<Placed> {
    list.items
        .iter()
        .filter_map(|item| match item {
            web_paint::Item::Rect { rect, color: fill } => Some(Placed {
                top: rect.y,
                bottom: rect.y + rect.height,
                item: Item::Fill {
                    shape: Shape::rect(Rect::new(rect.x, rect.y, rect.width, rect.height)),
                    paint: color(*fill).into(),
                },
            }),
            web_paint::Item::Glyphs {
                rect,
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
                    .map(|glyph| PositionedGlyph {
                        id: glyph.id,
                        x: glyph.x,
                        y: glyph.y,
                    })
                    .collect();
                Some(Placed {
                    top: rect.y,
                    bottom: rect.y + rect.height,
                    item: Item::Glyphs(GlyphRun::new(font, *size, color(*ink), glyphs)),
                })
            }
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
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        self.shown_offset.set(offset);
        let shown = offset..offset + self.frame.height;
        let visible = self
            .placed
            .iter()
            .filter(|placed| placed.bottom >= shown.start && placed.top <= shown.end)
            .map(|placed| placed.item.clone())
            .collect();
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
