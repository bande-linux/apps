use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{Context, Event, Graph, GraphKind, Series, Size, Widget, WidgetId};
use bande_toolkit::{Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle};

const PADDING: f32 = 12.0;
const GAP: f32 = 4.0;
const STAT_HEIGHT: f32 = 84.0;
const LIVE_HEIGHT: f32 = 118.0;

fn text(cx: &mut Context, text: &str, role: TextRole, width: f32) -> Paragraph {
    let ink = match role {
        TextRole::Title => cx.palette.ink,
        TextRole::Label => cx.palette.ink3,
        _ => cx.palette.ink2,
    };
    cx.text.paragraph(
        text,
        TextStyle::role(role, ink).tabular(),
        Some(width),
        TextAlign::Start,
    )
}

fn panel(cx: &Context, frame: Rect, items: &mut Vec<Item>) {
    let shape = Shape::rounded(frame, Radius::L);
    items.push(Item::Fill {
        shape,
        paint: cx.palette.well.into(),
    });
    items.push(Item::Stroke {
        shape: Shape::rounded(frame.inset(0.5), Radius::L - 0.5),
        width: 1.0,
        paint: cx.palette.hairline.into(),
    });
}

pub struct Card {
    id: WidgetId,
    title: String,
    value: String,
    caption: String,
    on_click: Box<dyn FnMut()>,
    pressed: bool,
    texts: Vec<(Point, Paragraph)>,
    frame: Rect,
}

impl Card {
    pub fn new(title: &str, on_click: impl FnMut() + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            value: String::new(),
            caption: String::new(),
            on_click: Box::new(on_click),
            pressed: false,
            texts: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, value: &str, caption: &str) {
        value.clone_into(&mut self.value);
        caption.clone_into(&mut self.caption);
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, STAT_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = (frame.width - 2.0 * PADDING).max(0.0);
        let mut y = frame.y + PADDING;
        self.texts.clear();
        for (content, role) in [
            (self.title.as_str(), TextRole::Label),
            (self.value.as_str(), TextRole::Title),
            (self.caption.as_str(), TextRole::Callout),
        ] {
            let paragraph = text(cx, content, role, width);
            let height = paragraph.height();
            self.texts
                .push((Point::new(frame.x + PADDING, y), paragraph));
            y += height + GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        panel(cx, self.frame, items);
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerLeave => self.pressed = false,
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let clicked = self.pressed && self.frame.contains(pointer.position);
                self.pressed = false;
                if clicked {
                    (self.on_click)();
                }
            }
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            (self.on_click)();
            return true;
        }
        false
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label(self.title.clone());
        node.set_value(format!("{} {}", self.value, self.caption));
        node.add_action(Action::Click);
        node
    }
}

pub struct Live {
    id: WidgetId,
    title: String,
    value: String,
    graph: Graph,
    texts: Vec<(Point, Paragraph)>,
    frame: Rect,
}

impl Live {
    pub fn new(title: &str, span: usize) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            value: String::new(),
            graph: Graph::new(title, GraphKind::Area, span),
            texts: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, value: &str, series: Vec<Series>, note: Option<&str>) {
        value.clone_into(&mut self.value);
        self.graph.set_series(series);
        self.graph.set_note(note);
    }
}

impl Widget for Live {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, LIVE_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = (frame.width - 2.0 * PADDING).max(0.0);
        let title = text(cx, &self.title, TextRole::Label, width / 2.0);
        let value = cx.text.paragraph(
            &self.value,
            TextStyle::role(TextRole::Callout, cx.palette.ink).tabular(),
            Some(width),
            TextAlign::End,
        );
        let top = frame.y + PADDING;
        let header = title.height().max(value.height()).ceil();
        self.texts = vec![
            (Point::new(frame.x + PADDING, top), title),
            (Point::new(frame.x + PADDING, top), value),
        ];
        let graph_top = top + header + GAP * 2.0;
        let graph = Rect::new(
            frame.x + PADDING,
            graph_top,
            width,
            (frame.y + frame.height - PADDING - graph_top).max(1.0),
        );
        self.graph.measure(cx, Size::new(graph.width, graph.height));
        self.graph.place(cx, graph);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        panel(cx, self.frame, items);
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
        self.graph.paint(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.title.clone());
        node.set_value(self.value.clone());
        node
    }
}

pub struct Grid {
    id: WidgetId,
    columns: usize,
    gap: f32,
    children: Vec<Box<dyn Widget>>,
    heights: Vec<f32>,
    frame: Rect,
}

impl Grid {
    pub fn new(columns: usize, gap: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            columns: columns.max(1),
            gap,
            children,
            heights: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn cell_width(&self, width: f32) -> f32 {
        ((width - self.gap * (self.columns - 1) as f32) / self.columns as f32).max(0.0)
    }
}

impl Widget for Grid {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = self.cell_width(available.width);
        let sizes: Vec<f32> = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(width, available.height)).height)
            .collect();
        self.heights = sizes
            .chunks(self.columns)
            .map(|row| row.iter().copied().fold(0.0, f32::max))
            .collect();
        let height = self.heights.iter().sum::<f32>()
            + self.gap * self.heights.len().saturating_sub(1) as f32;
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = self.cell_width(frame.width);
        let mut y = frame.y;
        for (row, height) in self.children.chunks_mut(self.columns).zip(&self.heights) {
            for (index, child) in row.iter_mut().enumerate() {
                let x = frame.x + index as f32 * (width + self.gap);
                child.place(cx, Rect::new(x, y, width, *height));
            }
            y += height + self.gap;
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

pub struct Caption {
    id: WidgetId,
    text: String,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Caption {
    pub fn new() -> Self {
        Self {
            id: WidgetId::next(),
            text: String::new(),
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn set_text(&mut self, text: &str) {
        text.clone_into(&mut self.text);
    }
}

impl Widget for Caption {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let paragraph = cx.text.paragraph(
            &self.text,
            TextStyle::role(TextRole::Body, cx.palette.ink2),
            Some(available.width),
            TextAlign::Start,
        );
        let height = paragraph.height().ceil();
        self.paragraph = Some(paragraph);
        Size::new(available.width, height)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
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
