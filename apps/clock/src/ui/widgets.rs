use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Size, Widget, WidgetId};
use bande_toolkit::{Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

const LINE_PADDING: f32 = 12.0;
const LINE_GAP: f32 = 8.0;
const LEADING_GAP: f32 = 2.0;

pub struct Digits {
    id: WidgetId,
    text: String,
    size: f32,
    weight: f32,
    muted: bool,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Digits {
    pub fn new(text: &str, size: f32) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.to_owned(),
            size,
            weight: TextStyle::REGULAR,
            muted: false,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn light(self) -> Self {
        Self {
            weight: 300.0,
            ..self
        }
    }

    pub fn muted(self, muted: bool) -> Self {
        Self { muted, ..self }
    }

    pub fn set_text(&mut self, text: &str) -> bool {
        if self.text == text {
            return false;
        }
        text.clone_into(&mut self.text);
        self.paragraph = None;
        true
    }
}

impl Widget for Digits {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let ink = match self.muted {
            true => cx.palette.ink2,
            false => cx.palette.ink,
        };
        let style = TextStyle::new(self.size, ink).weight(self.weight).tabular();
        let paragraph =
            cx.text
                .paragraph(&self.text, style, Some(available.width), TextAlign::Start);
        let size = Size::new(paragraph.width().ceil(), paragraph.height().ceil());
        self.paragraph = Some(paragraph);
        size
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        if self.paragraph.is_none() {
            self.measure(cx, Size::new(frame.width, frame.height));
        }
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(self.frame.x, self.frame.y),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_value(self.text.clone());
        node
    }
}

pub struct Line {
    id: WidgetId,
    label: String,
    leading: usize,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Line {
    pub fn new(label: &str, leading: Vec<Box<dyn Widget>>, trailing: Vec<Box<dyn Widget>>) -> Self {
        let count = leading.len();
        let mut children = leading;
        children.extend(trailing);
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            leading: count,
            children,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Line {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let inner = Size::new(
            (available.width - 2.0 * LINE_PADDING).max(0.0),
            available.height,
        );
        let trailing: Vec<Size> = self.children[self.leading..]
            .iter_mut()
            .map(|child| child.measure(cx, inner))
            .collect();
        let trailing_width = trailing
            .iter()
            .map(|size| size.width + LINE_GAP)
            .sum::<f32>();
        let room = Size::new((inner.width - trailing_width).max(0.0), inner.height);
        let mut sizes: Vec<Size> = self.children[..self.leading]
            .iter_mut()
            .map(|child| child.measure(cx, room))
            .collect();
        let leading_height = sizes.iter().map(|size| size.height).sum::<f32>()
            + LEADING_GAP * sizes.len().saturating_sub(1) as f32;
        let height = trailing
            .iter()
            .map(|size| size.height)
            .fold(leading_height, f32::max);
        sizes.extend(trailing);
        self.sizes = sizes;
        Size::new(available.width, height + 2.0 * LINE_PADDING)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let inner = frame.inset(LINE_PADDING);
        let leading_height = self.sizes[..self.leading]
            .iter()
            .map(|size| size.height)
            .sum::<f32>()
            + LEADING_GAP * self.leading.saturating_sub(1) as f32;
        let mut y = inner.y + ((inner.height - leading_height) / 2.0).round();
        let mut x = inner.x + inner.width;
        for (index, (child, size)) in self.children.iter_mut().zip(&self.sizes).enumerate() {
            let child_frame = match index < self.leading {
                true => {
                    let placed = Rect::new(inner.x, y, size.width, size.height);
                    y += size.height + LEADING_GAP;
                    placed
                }
                false => Rect::new(
                    x - size.width,
                    inner.y + ((inner.height - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                ),
            };
            if index >= self.leading {
                x -= size.width + LINE_GAP;
            }
            child.place(cx, child_frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                self.frame.x + LINE_PADDING,
                self.frame.y + self.frame.height - 1.0,
                (self.frame.width - 2.0 * LINE_PADDING).max(0.0),
                1.0,
            )),
            paint: cx.palette.hairline.into(),
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ListItem);
        node.set_label(self.label.clone());
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Fixed {
    id: WidgetId,
    height: f32,
    child: [Box<dyn Widget>; 1],
    frame: Rect,
}

impl Fixed {
    pub fn new(height: f32, child: impl Widget + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            height,
            child: [Box::new(child)],
            frame: Rect::default(),
        }
    }
}

impl Widget for Fixed {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let size = Size::new(available.width, self.height);
        self.child[0].measure(cx, size);
        size
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.child[0].place(cx, frame);
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
