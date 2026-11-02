use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Size, Widget, WidgetId};
use bande_toolkit::{Item, Rect, Shape};

pub struct Slot {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    frame: Rect,
}

impl Slot {
    pub fn new(child: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            child: [child],
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, child: Box<dyn Widget>) {
        self.child = [child];
    }
}

impl Widget for Slot {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.child[0].measure(cx, available);
        available
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

pub struct Split {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    side: f32,
    frame: Rect,
}

impl Split {
    pub fn new(main: Box<dyn Widget>, side: f32) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![main],
            side,
            frame: Rect::default(),
        }
    }

    pub fn set_side(&mut self, side: Option<Box<dyn Widget>>) {
        self.children.truncate(1);
        self.children.extend(side);
    }

    fn frames(&self) -> [Rect; 2] {
        let frame = self.frame;
        let side = match self.children.len() > 1 {
            true => self.side.min(frame.width),
            false => 0.0,
        };
        [
            Rect::new(frame.x, frame.y, frame.width - side, frame.height),
            Rect::new(frame.x + frame.width - side, frame.y, side, frame.height),
        ]
    }
}

impl Widget for Split {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let frames = self.frames();
        for (child, frame) in self.children.iter_mut().zip(frames) {
            child.measure(cx, Size::new(frame.width, frame.height));
            child.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
        if self.children.len() > 1 {
            let side = self.frames()[1];
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(side.x, side.y, 1.0, side.height)),
                paint: cx.palette.hairline.into(),
            });
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

pub struct Bar {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    toolbar: f32,
    start: f32,
    gap: f32,
    inset: f32,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Bar {
    pub const CONTENT: usize = 0;

    pub fn new(
        content: Box<dyn Widget>,
        tools: Vec<Box<dyn Widget>>,
        toolbar: f32,
        start: f32,
    ) -> Self {
        let mut children = vec![content];
        children.extend(tools);
        Self {
            id: WidgetId::next(),
            children,
            toolbar,
            start,
            gap: 8.0,
            inset: 12.0,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn set_content(&mut self, content: Box<dyn Widget>) {
        self.children[Self::CONTENT] = content;
    }

    pub fn set_tools(&mut self, tools: Vec<Box<dyn Widget>>) {
        self.children.truncate(1);
        self.children.extend(tools);
    }

    pub fn set_start(&mut self, start: f32) {
        self.start = start;
    }

    pub fn tool_frames(&self) -> Vec<Rect> {
        let width: f32 = self.sizes.iter().map(|size| size.width).sum::<f32>()
            + self.gap * self.sizes.len().saturating_sub(1) as f32;
        let mut x = (self.frame.width - self.inset - width).max(self.start);
        self.sizes
            .iter()
            .map(|size| {
                let frame = Rect::new(
                    x.round(),
                    ((self.toolbar - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                );
                x += size.width + self.gap;
                frame
            })
            .collect()
    }
}

impl Widget for Bar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let available = Size::new(frame.width, self.toolbar);
        self.sizes = self.children[1..]
            .iter_mut()
            .map(|tool| tool.measure(cx, available))
            .collect();
        let content = Rect::new(
            frame.x,
            frame.y + self.toolbar,
            frame.width,
            (frame.height - self.toolbar).max(0.0),
        );
        self.children[Self::CONTENT].measure(cx, Size::new(content.width, content.height));
        self.children[Self::CONTENT].place(cx, content);
        let frames = self.tool_frames();
        for (tool, frame) in self.children[1..].iter_mut().zip(frames) {
            tool.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: Shape::rect(self.children[Self::CONTENT].frame()),
            paint: cx.palette.sheet.into(),
        });
        for child in &self.children {
            child.paint(cx, items);
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
