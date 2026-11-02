use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_toolkit::ui::{BUTTON_HEIGHT, Context, Size, Widget, WidgetId};
use bande_toolkit::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, SHEET_INSET, Shape, Spacing, TextAlign,
    TextRole, TextStyle, WINDOW_HEADER_HEIGHT,
};

pub const SIDEBAR_WIDTH: f32 = 240.0;
pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const SIDEBAR_INSET: f32 = Spacing::S as f32;
const TRAILING_INSET: f32 = 12.0;
const TOOL_GAP: f32 = 6.0;
const ICON: f32 = 48.0;
const ICON_GAP: f32 = 14.0;
const BAR_HEIGHT: f32 = 22.0;
const SWATCH: f32 = 10.0;
const LEGEND_GAP: f32 = 18.0;
const LEGEND_TOP: f32 = 10.0;
const LINE_GAP: f32 = 2.0;
const ROW_HEIGHT: f32 = 26.0;
const LABEL_SHARE: f32 = 0.34;
const SYMBOL_INK: &str = "#201E1A";

fn hex(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

pub fn symbol(svg: &str, color: Color, pixels: u32) -> Option<Image> {
    let svg = svg.replace(SYMBOL_INK, &hex(color));
    Image::from_svg(svg.as_bytes(), pixels, pixels)
}

fn text(cx: &mut Context, text: &str, role: TextRole, color: Color, width: f32) -> Paragraph {
    cx.text.paragraph(
        text,
        TextStyle::role(role, color),
        Some(width.max(0.0)),
        TextAlign::Start,
    )
}

pub struct Root {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    inset: f32,
    frame: Rect,
}

impl Root {
    const CONTENT: usize = 1;
    const TOOLS: usize = 2;

    pub fn new(sidebar: Box<dyn Widget>, content: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![sidebar, content],
            sizes: Vec::new(),
            inset: SHEET_INSET as f32,
            frame: Rect::default(),
        }
    }

    pub fn set_inset(&mut self, inset: f32) {
        self.inset = inset;
    }

    pub fn set_content(&mut self, content: Box<dyn Widget>) {
        self.children[Self::CONTENT] = content;
    }

    pub fn set_tools(&mut self, tools: Vec<Box<dyn Widget>>) {
        self.children.truncate(Self::TOOLS);
        self.children.extend(tools);
    }

    fn sheet(&self) -> Rect {
        Rect::new(
            SIDEBAR_WIDTH,
            TOOLBAR_HEIGHT,
            (self.frame.width - SIDEBAR_WIDTH - self.inset).max(0.0),
            (self.frame.height - TOOLBAR_HEIGHT - self.inset).max(0.0),
        )
    }

    pub fn tool_frames(&self) -> Vec<Rect> {
        let mut x = self.frame.width - TRAILING_INSET;
        let mut frames: Vec<Rect> = self
            .sizes
            .iter()
            .rev()
            .map(|size| {
                x -= size.width;
                let frame = Rect::new(
                    x,
                    ((TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0).round(),
                    size.width,
                    BUTTON_HEIGHT,
                );
                x -= TOOL_GAP;
                frame
            })
            .collect();
        frames.reverse();
        frames
    }
}

impl Widget for Root {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sizes = self
            .children
            .iter_mut()
            .skip(Self::TOOLS)
            .map(|tool| tool.measure(cx, Size::new(available.width, BUTTON_HEIGHT)))
            .collect();
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let content = self.sheet();
        let mut frames = vec![
            Rect::new(
                SIDEBAR_INSET,
                TOOLBAR_HEIGHT,
                SIDEBAR_WIDTH - 2.0 * SIDEBAR_INSET,
                (frame.height - TOOLBAR_HEIGHT).max(0.0),
            ),
            content,
        ];
        frames.extend(self.tool_frames());
        for (index, (child, frame)) in self.children.iter_mut().zip(frames).enumerate() {
            if index < Self::TOOLS {
                child.measure(cx, Size::new(frame.width, frame.height));
            }
            child.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (index, child) in self.children.iter().enumerate() {
            if index != Self::CONTENT {
                child.paint(cx, items);
                continue;
            }
            let radius = match self.inset > 0.0 {
                true => Radius::concentric(Radius::XL, self.inset),
                false => 0.0,
            };
            let mut content = Vec::new();
            child.paint(cx, &mut content);
            items.push(Item::Clip {
                shape: Shape::rounded(self.sheet(), radius),
                items: content,
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Window)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Closable {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    closed: Rc<Cell<bool>>,
}

impl Closable {
    pub fn new(child: impl Widget + 'static, closed: Rc<Cell<bool>>) -> Self {
        Self {
            id: WidgetId::next(),
            child: [Box::new(child)],
            closed,
        }
    }
}

impl Widget for Closable {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.child[0].measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.child[0].place(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.child[0].frame()
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.child[0].paint(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn dismissed(&self) -> bool {
        self.closed.get() || self.child[0].dismissed()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}

pub struct Heading {
    id: WidgetId,
    icon: Option<Image>,
    title: String,
    subtitle: String,
    capacity: String,
    texts: Vec<(Point, Paragraph)>,
    frame: Rect,
}

impl Heading {
    pub fn new(icon: Option<Image>, title: &str, subtitle: &str, capacity: &str) -> Self {
        Self {
            id: WidgetId::next(),
            icon,
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            capacity: capacity.to_owned(),
            texts: Vec::new(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Heading {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, ICON)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let ink = cx.palette.ink;
        let ink2 = cx.palette.ink2;
        let capacity = text(
            cx,
            &self.capacity,
            TextRole::Heading,
            ink,
            frame.width / 3.0,
        );
        let left = frame.x + ICON + ICON_GAP;
        let width = frame.width - ICON - ICON_GAP - capacity.width() - ICON_GAP;
        let title = text(cx, &self.title, TextRole::Heading, ink, width);
        let subtitle = text(cx, &self.subtitle, TextRole::Callout, ink2, width);
        let top = frame.y + ((ICON - title.height() - LINE_GAP - subtitle.height()) / 2.0).round();
        self.texts = vec![
            (
                Point::new(
                    frame.x + frame.width - capacity.width(),
                    frame.y + ((ICON - capacity.height()) / 2.0).round(),
                ),
                capacity,
            ),
            (Point::new(left, top + title.height() + LINE_GAP), subtitle),
            (Point::new(left, top), title),
        ];
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(self.frame.x, self.frame.y, ICON, ICON),
                image: icon.clone(),
            });
        }
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Heading);
        node.set_label(self.title.clone());
        node.set_description(format!("{}, {}", self.subtitle, self.capacity));
        node
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub name: String,
    pub detail: String,
    pub size: u64,
    pub color: Color,
}

pub struct CapacityBar {
    id: WidgetId,
    segments: Vec<Segment>,
    total: u64,
    legend: Vec<(Rect, Point, Paragraph, Point, Paragraph)>,
    height: f32,
    frame: Rect,
}

impl CapacityBar {
    pub fn new(segments: Vec<Segment>, total: u64) -> Self {
        Self {
            id: WidgetId::next(),
            segments,
            total,
            legend: Vec::new(),
            height: BAR_HEIGHT,
            frame: Rect::default(),
        }
    }

    fn lay_out(&mut self, cx: &mut Context, frame: Rect) {
        let ink = cx.palette.ink;
        let ink2 = cx.palette.ink2;
        let columns = self.segments.len().clamp(1, 4) as f32;
        let column = (frame.width - (columns - 1.0) * LEGEND_GAP) / columns;
        let mut legend = Vec::new();
        let mut x = frame.x;
        let mut y = frame.y + BAR_HEIGHT + LEGEND_TOP;
        let mut row_height = 0.0f32;
        for (index, segment) in self.segments.iter().enumerate() {
            if index > 0 && index % 4 == 0 {
                x = frame.x;
                y += row_height + LEGEND_TOP;
                row_height = 0.0;
            }
            let width = column - SWATCH - Spacing::XS as f32;
            let name = text(cx, &segment.name, TextRole::BodyStrong, ink, width);
            let detail = text(cx, &segment.detail, TextRole::Callout, ink2, width);
            let swatch = Rect::new(x, y + 3.0, SWATCH, SWATCH);
            let text_x = x + SWATCH + Spacing::XS as f32;
            let detail_y = y + name.height() + LINE_GAP;
            row_height = row_height.max(name.height() + LINE_GAP + detail.height());
            legend.push((
                swatch,
                Point::new(text_x, y),
                name,
                Point::new(text_x, detail_y),
                detail,
            ));
            x += column + LEGEND_GAP;
        }
        self.height = y + row_height - frame.y;
        self.legend = legend;
    }
}

impl Widget for CapacityBar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.lay_out(cx, Rect::new(0.0, 0.0, available.width, 0.0));
        Size::new(available.width, self.height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.lay_out(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let bar = Rect::new(self.frame.x, self.frame.y, self.frame.width, BAR_HEIGHT);
        let mut segments = vec![Item::Fill {
            shape: Shape::rect(bar),
            paint: cx.palette.well.into(),
        }];
        let mut x = bar.x;
        for segment in &self.segments {
            let width = (bar.width * segment.size as f32 / self.total.max(1) as f32).max(2.0);
            segments.push(Item::Fill {
                shape: Shape::rect(Rect::new(x, bar.y, width, BAR_HEIGHT)),
                paint: segment.color.into(),
            });
            x += width;
            segments.push(Item::Line {
                from: Point::new(x, bar.y),
                to: Point::new(x, bar.y + BAR_HEIGHT),
                width: 1.0,
                paint: cx.palette.sheet.into(),
            });
        }
        items.push(Item::Clip {
            shape: Shape::rounded(bar, Radius::S),
            items: segments,
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(bar, Radius::S),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
        for ((swatch, name_at, name, detail_at, detail), segment) in
            self.legend.iter().zip(&self.segments)
        {
            items.push(Item::Fill {
                shape: Shape::rounded(*swatch, Radius::XS / 2.0),
                paint: segment.color.into(),
            });
            items.push(Item::Text {
                origin: *name_at,
                paragraph: name.clone(),
            });
            items.push(Item::Text {
                origin: *detail_at,
                paragraph: detail.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::List);
        node.set_label(
            self.segments
                .iter()
                .map(|segment| format!("{}: {}", segment.name, segment.detail))
                .collect::<Vec<_>>()
                .join("; "),
        );
        node
    }
}

pub struct InfoGrid {
    id: WidgetId,
    pairs: Vec<(String, String)>,
    texts: Vec<(Point, Paragraph)>,
    frame: Rect,
}

impl InfoGrid {
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        Self {
            id: WidgetId::next(),
            pairs,
            texts: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn rows(&self) -> usize {
        self.pairs.len().div_ceil(2)
    }
}

impl Widget for InfoGrid {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, self.rows() as f32 * ROW_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let half = frame.width / 2.0;
        let rows = self.rows();
        let ink = cx.palette.ink;
        let ink2 = cx.palette.ink2;
        let mut texts = Vec::new();
        for (index, (label, value)) in self.pairs.iter().enumerate() {
            let x = frame.x + (index / rows) as f32 * half;
            let y = frame.y + (index % rows) as f32 * ROW_HEIGHT;
            let label_width = half * LABEL_SHARE;
            let label = text(cx, label, TextRole::Callout, ink2, label_width);
            let value = text(cx, value, TextRole::Callout, ink, half - label_width - 12.0);
            let middle = |paragraph: &Paragraph| y + ((ROW_HEIGHT - paragraph.height()) / 2.0);
            texts.push((Point::new(x, middle(&label).round()), label));
            texts.push((Point::new(x + label_width, middle(&value).round()), value));
        }
        self.texts = texts;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for row in 1..self.rows() {
            let y = self.frame.y + row as f32 * ROW_HEIGHT;
            items.push(Item::Line {
                from: Point::new(self.frame.x, y),
                to: Point::new(self.frame.x + self.frame.width, y),
                width: 1.0,
                paint: cx.palette.hairline.into(),
            });
        }
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::DescriptionList);
        node.set_label(
            self.pairs
                .iter()
                .map(|(label, value)| format!("{label}: {value}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
        node
    }
}
