use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{Context, Event, Key, Size, Widget, WidgetId, focus_ring};
use bande_toolkit::{Color, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextStyle};

const ROW_PADDING: f32 = 8.0;
const ROW_GAP: f32 = 12.0;
const NAME_SIZE: f32 = 13.0;
const SUMMARY_SIZE: f32 = 12.0;

pub struct Grid {
    id: WidgetId,
    columns: usize,
    spacing: f32,
    children: Vec<Box<dyn Widget>>,
    heights: Vec<f32>,
    frame: Rect,
}

impl Grid {
    pub fn new(columns: usize, spacing: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            columns: columns.max(1),
            spacing,
            children,
            heights: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn column_width(&self, width: f32) -> f32 {
        let gaps = self.spacing * (self.columns - 1) as f32;
        ((width - gaps) / self.columns as f32).floor().max(0.0)
    }
}

impl Widget for Grid {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = self.column_width(available.width);
        let sizes: Vec<Size> = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(width, available.height)))
            .collect();
        self.heights = sizes
            .chunks(self.columns)
            .map(|row| row.iter().map(|size| size.height).fold(0.0, f32::max))
            .collect();
        let gaps = self.spacing * self.heights.len().saturating_sub(1) as f32;
        Size::new(available.width, self.heights.iter().sum::<f32>() + gaps)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = self.column_width(frame.width);
        let mut y = frame.y;
        let columns = self.columns;
        for (row, height) in self.children.chunks_mut(columns).zip(&self.heights) {
            for (column, child) in row.iter_mut().enumerate() {
                let x = frame.x + column as f32 * (width + self.spacing);
                child.place(cx, Rect::new(x, y, width, *height));
            }
            y += height + self.spacing;
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
        Node::new(Role::List)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Picture {
    id: WidgetId,
    image: Option<Image>,
    width: f32,
    height: f32,
    description: String,
    rounded: bool,
    frame: Rect,
}

impl Picture {
    pub fn new(image: Option<Image>, width: f32, height: f32, description: &str) -> Self {
        Self {
            id: WidgetId::next(),
            image,
            width,
            height,
            description: description.to_owned(),
            rounded: false,
            frame: Rect::default(),
        }
    }

    pub fn fitted(image: Image, max_width: f32, max_height: f32, description: &str) -> Self {
        let scale = (max_width / image.width().max(1) as f32)
            .min(max_height / image.height().max(1) as f32)
            .min(1.0);
        let width = (image.width() as f32 * scale).round();
        let height = (image.height() as f32 * scale).round();
        Self {
            rounded: true,
            ..Self::new(Some(image), width, height, description)
        }
    }
}

fn paint_image(cx: &Context, image: Option<&Image>, rect: Rect, items: &mut Vec<Item>) {
    match image {
        Some(image) => items.push(Item::Image {
            rect,
            image: image.clone(),
        }),
        None => items.push(Item::Fill {
            shape: Shape::rounded(rect, Radius::L),
            paint: cx.palette.well.into(),
        }),
    }
}

impl Widget for Picture {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        let scale = (available.width / self.width.max(1.0)).min(1.0);
        Size::new(self.width * scale, self.height * scale)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let scale = (self.frame.width / self.width.max(1.0)).min(1.0);
        let rect = Rect::new(
            self.frame.x,
            self.frame.y,
            self.width * scale,
            self.height * scale,
        );
        if !self.rounded {
            paint_image(cx, self.image.as_ref(), rect, items);
            return;
        }
        let mut clipped = Vec::new();
        paint_image(cx, self.image.as_ref(), rect, &mut clipped);
        items.push(Item::Clip {
            shape: Shape::rounded(rect, Radius::L),
            items: clipped,
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(rect, Radius::L),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Image);
        node.set_label(self.description.clone());
        node
    }
}

type TextKey = (f32, Color, f32);

pub struct AppRow {
    id: WidgetId,
    icon: Option<Image>,
    icon_size: f32,
    name: String,
    summary: String,
    on_open: Box<dyn FnMut()>,
    action: Vec<Box<dyn Widget>>,
    action_size: Size,
    texts: Option<(TextKey, Paragraph, Paragraph)>,
    hovered: bool,
    pressed: bool,
    frame: Rect,
}

impl AppRow {
    pub fn new(
        icon: Option<Image>,
        icon_size: f32,
        name: &str,
        summary: &str,
        action: Option<Box<dyn Widget>>,
        on_open: impl FnMut() + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            icon,
            icon_size,
            name: name.to_owned(),
            summary: summary.to_owned(),
            on_open: Box::new(on_open),
            action: action.into_iter().collect(),
            action_size: Size::new(0.0, 0.0),
            texts: None,
            hovered: false,
            pressed: false,
            frame: Rect::default(),
        }
    }

    fn icon_rect(&self) -> Rect {
        Rect::new(
            self.frame.x + ROW_PADDING,
            self.frame.y + ((self.frame.height - self.icon_size) / 2.0).round(),
            self.icon_size,
            self.icon_size,
        )
    }
}

impl Widget for AppRow {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.action_size = self
            .action
            .iter_mut()
            .map(|action| action.measure(cx, available))
            .next()
            .unwrap_or(Size::new(0.0, 0.0));
        let used = 2.0 * ROW_PADDING
            + self.icon_size
            + ROW_GAP
            + match self.action_size.width > 0.0 {
                true => self.action_size.width + ROW_GAP,
                false => 0.0,
            };
        let room = (available.width - used).max(0.0);
        let key = (room, cx.palette.ink, cx.text.scale());
        let (name, summary) = match self.texts.take().filter(|(kept, ..)| *kept == key) {
            Some((_, name, summary)) => (name, summary),
            None => (
                cx.text.fitted(
                    &self.name,
                    TextStyle::new(NAME_SIZE, cx.palette.ink).weight(TextStyle::SEMIBOLD),
                    room,
                ),
                cx.text.fitted(
                    &self.summary,
                    TextStyle::new(SUMMARY_SIZE, cx.palette.ink2),
                    room,
                ),
            ),
        };
        let height = (name.height() + summary.height())
            .max(self.icon_size)
            .max(self.action_size.height)
            + 2.0 * ROW_PADDING;
        self.texts = Some((key, name, summary));
        Size::new(available.width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let size = self.action_size;
        if let Some(action) = self.action.first_mut() {
            action.place(
                cx,
                Rect::new(
                    frame.x + frame.width - ROW_PADDING - size.width,
                    frame.y + ((frame.height - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                ),
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, Radius::M);
        if self.hovered || self.pressed {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.well.into(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
        paint_image(cx, self.icon.as_ref(), self.icon_rect(), items);
        if let Some((_, name, summary)) = &self.texts {
            let x = self.frame.x + ROW_PADDING + self.icon_size + ROW_GAP;
            let top = self.frame.y
                + ((self.frame.height - name.height() - summary.height()) / 2.0).round();
            items.push(Item::Text {
                origin: Point::new(x, top),
                paragraph: name.clone(),
            });
            items.push(Item::Text {
                origin: Point::new(x, top + name.height()),
                paragraph: summary.clone(),
            });
        }
        for action in &self.action {
            action.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ListItem);
        node.set_label(self.name.clone());
        node.set_description(self.summary.clone());
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerEnter(_) => self.hovered = true,
            Event::PointerLeave => {
                self.hovered = false;
                self.pressed = false;
            }
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerMove(pointer) if self.pressed => {
                self.pressed = self.frame.contains(pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let open = self.pressed && self.frame.contains(pointer.position);
                self.pressed = false;
                if open {
                    (self.on_open)();
                }
            }
            Event::Key {
                key: Key::Space | Key::Enter,
                ..
            } => (self.on_open)(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            (self.on_open)();
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.action
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.action
    }
}
