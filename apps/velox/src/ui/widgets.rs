use accesskit::{Node, Role};
use bande_toolkit::ui::{BUTTON_HEIGHT, Context, Size, TEXT_FIELD_HEIGHT, Widget, WidgetId};
use bande_toolkit::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, SHEET_INSET, Shape, Spacing, TextAlign,
    TextRole, TextStyle, WINDOW_HEADER_HEIGHT,
};

pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const SIDEBAR_INSET: f32 = Spacing::S as f32;
const PILL_HEIGHT: f32 = 36.0;
const TRAILING_INSET: f32 = 12.0;
const TOOL_GAP: f32 = 4.0;
const SEARCH_GAP: f32 = 12.0;
const SEARCH_WIDTH: f32 = 200.0;
const DOT: f32 = 8.0;
const DOT_GAP: f32 = 8.0;
const CODE_PADDING: f32 = Spacing::M as f32;
const SYMBOL_PIXELS: u32 = 32;
const SYMBOL_INK: &str = "#201E1A";

fn hex(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

pub fn symbol(svg: &str, color: Color) -> Option<Image> {
    let svg = svg.replace(SYMBOL_INK, &hex(color));
    Image::from_svg(svg.as_bytes(), SYMBOL_PIXELS, SYMBOL_PIXELS)
}

pub fn dot(color: Color, filled: bool) -> Option<Image> {
    let paint = match filled {
        true => format!(r#"fill="{}""#, hex(color)),
        false => format!(r#"fill="none" stroke="{}" stroke-width="1.2""#, hex(color)),
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="8" cy="8" r="3.6" {paint}/></svg>"#
    );
    Image::from_svg(svg.as_bytes(), SYMBOL_PIXELS, SYMBOL_PIXELS)
}

pub struct Root {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    inset: f32,
    frame: Rect,
}

impl Root {
    const PILL: usize = 1;
    const CONTENT: usize = 2;
    const TOOLS: usize = 3;

    pub fn new(sidebar: Box<dyn Widget>, pill: Box<dyn Widget>, content: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![sidebar, pill, content],
            inset: SHEET_INSET as f32,
            frame: Rect::default(),
        }
    }

    pub fn set_inset(&mut self, inset: f32) {
        self.inset = inset;
    }

    pub fn set_pill(&mut self, pill: Box<dyn Widget>) {
        self.children[Self::PILL] = pill;
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
        let tools = self.children.len().saturating_sub(Self::TOOLS);
        let Some(buttons) = tools.checked_sub(1) else {
            return Vec::new();
        };
        let right = self.frame.width - TRAILING_INSET;
        let search_x = right - SEARCH_WIDTH;
        let button_y = ((TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0).round();
        let mut x = search_x - SEARCH_GAP - buttons as f32 * (BUTTON_HEIGHT + TOOL_GAP) + TOOL_GAP;
        let mut frames = Vec::new();
        for _ in 0..buttons {
            frames.push(Rect::new(x, button_y, BUTTON_HEIGHT, BUTTON_HEIGHT));
            x += BUTTON_HEIGHT + TOOL_GAP;
        }
        frames.push(Rect::new(
            search_x,
            ((TOOLBAR_HEIGHT - TEXT_FIELD_HEIGHT) / 2.0).round(),
            SEARCH_WIDTH,
            TEXT_FIELD_HEIGHT,
        ));
        frames
    }
}

impl Widget for Root {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = SIDEBAR_WIDTH - 2.0 * SIDEBAR_INSET;
        let mut frames = vec![
            Rect::new(
                SIDEBAR_INSET,
                TOOLBAR_HEIGHT,
                width,
                (frame.height - TOOLBAR_HEIGHT - PILL_HEIGHT).max(0.0),
            ),
            Rect::new(
                SIDEBAR_INSET,
                (frame.height - PILL_HEIGHT).max(0.0),
                width,
                PILL_HEIGHT,
            ),
            self.sheet(),
        ];
        frames.extend(self.tool_frames());
        for (child, frame) in self.children.iter_mut().zip(frames) {
            child.measure(cx, Size::new(frame.width, frame.height));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    On,
    Waiting,
    Off,
}

pub struct Pill {
    id: WidgetId,
    text: String,
    light: Light,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Pill {
    pub fn new(text: &str, light: Light) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.to_owned(),
            light,
            paragraph: None,
            frame: Rect::default(),
        }
    }
}

impl Widget for Pill {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let style = TextStyle::role(TextRole::Callout, cx.palette.ink2);
        let width = (available.width - 2.0 * DOT_GAP - DOT).max(0.0);
        self.paragraph = Some(
            cx.text
                .paragraph(&self.text, style, Some(width), TextAlign::Start),
        );
        available
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let palette = &cx.palette;
        let middle = self.frame.y + self.frame.height / 2.0;
        let dot = Rect::new(self.frame.x + DOT_GAP, middle - DOT / 2.0, DOT, DOT);
        let shape = Shape::rounded(dot, DOT / 2.0);
        match self.light {
            Light::On => items.push(Item::Fill {
                shape,
                paint: palette.success.into(),
            }),
            Light::Waiting => items.push(Item::Fill {
                shape,
                paint: palette.warning.into(),
            }),
            Light::Off => items.push(Item::Stroke {
                shape,
                width: 1.2,
                paint: palette.ink3.into(),
            }),
        }
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(
                    dot.x + DOT + DOT_GAP,
                    (middle - paragraph.height() / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Status);
        node.set_value(self.text.clone());
        node
    }
}

pub struct Code {
    id: WidgetId,
    text: String,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Code {
    pub fn new(text: &str) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.to_owned(),
            paragraph: None,
            frame: Rect::default(),
        }
    }
}

impl Widget for Code {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let style = TextStyle::role(TextRole::Mono, cx.palette.ink);
        let paragraph = cx.text.paragraph(
            &self.text,
            style,
            Some((available.width - 2.0 * CODE_PADDING).max(0.0)),
            TextAlign::Start,
        );
        let height = paragraph.height().ceil() + 2.0 * CODE_PADDING;
        self.paragraph = Some(paragraph);
        Size::new(available.width, height)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, Radius::M);
        items.push(Item::Fill {
            shape,
            paint: cx.palette.well.into(),
        });
        items.push(Item::Stroke {
            shape,
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(self.frame.x + CODE_PADDING, self.frame.y + CODE_PADDING),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Code);
        node.set_value(self.text.clone());
        node
    }
}

pub struct Ends {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Ends {
    pub fn new(start: Box<dyn Widget>, end: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![start, end],
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Ends {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sizes = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, available))
            .collect();
        let height = self
            .sizes
            .iter()
            .map(|size| size.height)
            .fold(0.0, f32::max);
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let last = self.children.len().saturating_sub(1);
        for (index, (child, size)) in self.children.iter_mut().zip(&self.sizes).enumerate() {
            let x = match index == last {
                true => frame.x + frame.width - size.width,
                false => frame.x,
            };
            let y = frame.y + ((frame.height - size.height) / 2.0).round();
            child.place(cx, Rect::new(x, y, size.width, size.height));
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Centered {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    width: f32,
    size: Size,
    frame: Rect,
}

impl Centered {
    pub fn new(child: Box<dyn Widget>, width: f32) -> Self {
        Self {
            id: WidgetId::next(),
            child: [child],
            width,
            size: Size::default(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Centered {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = self.width.min(available.width);
        self.size = self.child[0].measure(cx, Size::new(width, available.height));
        Size::new(available.width, available.height.max(self.size.height))
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let x = frame.x + ((frame.width - self.size.width) / 2.0).max(0.0).round();
        let y = frame.y + ((frame.height - self.size.height) / 2.0).max(0.0).round();
        self.child[0].place(cx, Rect::new(x, y, self.size.width, self.size.height));
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.child[0].paint(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}
