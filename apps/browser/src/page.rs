use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Cursor, Event, Key, ScrollState, Size, Widget, WidgetId};
use bande_toolkit::{Color, Item, Point, Rect, Shape, TextAlign, TextStyle};
use web_net::{FetchError, Response, Url};

use crate::L10N;

const MARGIN: f32 = 32.0;
const LINE_STEP: f32 = 40.0;
const PAGE_OVERLAP: f32 = 40.0;
const TITLE_SIZE: f32 = 22.0;
const BODY_SIZE: f32 = 15.0;
const MESSAGE_WIDTH: f32 = 560.0;
const CANVAS: Color = Color::WHITE;
const TEXT: Color = Color::hex(0x202122);
const SECONDARY: Color = Color::hex(0x54595d);

pub(crate) enum Content {
    Blank,
    Loading(Url),
    Failed { url: Url, message: String },
    Loaded(Response),
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
    content: Content,
    laid_out_width: Option<f32>,
    placed: Vec<Placed>,
    height: f32,
    scroll: ScrollState,
}

impl PageView {
    pub(crate) fn new() -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            radius: 0.0,
            content: Content::Blank,
            laid_out_width: None,
            placed: Vec::new(),
            height: 0.0,
            scroll: ScrollState::default(),
        }
    }

    pub(crate) fn show(&mut self, content: Content) {
        self.content = content;
        self.laid_out_width = None;
        self.scroll = ScrollState::default();
    }

    pub(crate) fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
    }

    fn lay_out(&mut self, cx: &mut Context, width: f32) {
        self.placed.clear();
        let column = (width - 2.0 * MARGIN).max(0.0);
        let lines: Vec<(String, f32, f32, Color)> = match &self.content {
            Content::Blank => Vec::new(),
            Content::Loading(url) => vec![
                (L10N.tr("loading"), TITLE_SIZE, TextStyle::SEMIBOLD, TEXT),
                (url.to_string(), BODY_SIZE, TextStyle::REGULAR, SECONDARY),
            ],
            Content::Failed { url, message } => vec![
                (
                    L10N.tr("cannot-open"),
                    TITLE_SIZE,
                    TextStyle::SEMIBOLD,
                    TEXT,
                ),
                (message.clone(), BODY_SIZE, TextStyle::REGULAR, TEXT),
                (url.to_string(), BODY_SIZE, TextStyle::REGULAR, SECONDARY),
            ],
            Content::Loaded(response) => vec![
                (
                    response.url.host_str().unwrap_or_default().to_owned(),
                    TITLE_SIZE,
                    TextStyle::SEMIBOLD,
                    TEXT,
                ),
                (
                    format!(
                        "{} · {} · {} bytes",
                        response.status,
                        response
                            .content_type
                            .as_ref()
                            .map_or("", |content_type| content_type.essence.as_str()),
                        response.body.len()
                    ),
                    BODY_SIZE,
                    TextStyle::REGULAR,
                    SECONDARY,
                ),
                (
                    L10N.tr("engine-pending"),
                    BODY_SIZE,
                    TextStyle::REGULAR,
                    TEXT,
                ),
            ],
        };
        let column = column.min(MESSAGE_WIDTH);
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

    fn scroll_by_page(&mut self, cx: &Context, pages: f32) -> bool {
        let offset = self.scroll.offset(cx);
        let page = (self.frame.height - PAGE_OVERLAP).max(LINE_STEP);
        self.scroll.scroll_to(cx, offset + pages * page)
    }

    fn scroll_by(&mut self, cx: &Context, delta: f32) -> bool {
        let offset = self.scroll.offset(cx);
        self.scroll.scroll_to(cx, offset + delta)
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
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        let shown = offset..offset + self.frame.height;
        let mut content = vec![Item::Fill {
            shape: Shape::rect(self.frame),
            paint: CANVAS.into(),
        }];
        let visible = self
            .placed
            .iter()
            .filter(|placed| placed.bottom >= shown.start && placed.top <= shown.end)
            .map(|placed| placed.item.clone())
            .collect();
        content.push(Item::Translate {
            offset: Point::new(self.frame.x, self.frame.y - offset),
            items: visible,
        });
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
            _ => false,
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn cursor(&self, _point: Point) -> Option<Cursor> {
        Some(Cursor::Default)
    }
}
