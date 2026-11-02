use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Size, Widget, WidgetId};
use bande_toolkit::{Item, Paragraph, Point, Rect, TextAlign, TextRole, TextStyle};

const LABEL_WIDTH: f32 = 96.0;
const LABEL_GAP: f32 = 12.0;
const ROW_GAP: f32 = 8.0;
const NOTE_GAP: f32 = 12.0;
const STRIP_GAP: f32 = 8.0;
const BAND_PADDING: f32 = 10.0;

pub struct Field {
    pub label: String,
    pub control: Box<dyn Widget>,
    pub note: String,
}

impl Field {
    pub fn new(label: &str, control: impl Widget + 'static) -> Self {
        Self {
            label: label.to_owned(),
            control: Box::new(control),
            note: String::new(),
        }
    }

    pub fn note(self, note: &str) -> Self {
        Self {
            note: note.to_owned(),
            ..self
        }
    }
}

struct Line {
    label: Option<Paragraph>,
    note: Option<Paragraph>,
    height: f32,
    control: Size,
}

pub struct Fields {
    id: WidgetId,
    labels: Vec<(String, String)>,
    children: Vec<Box<dyn Widget>>,
    lines: Vec<Line>,
    texts: Vec<(Point, Paragraph)>,
    frame: Rect,
}

impl Fields {
    pub fn new(fields: Vec<Field>) -> Self {
        let mut labels = Vec::new();
        let mut children = Vec::new();
        for field in fields {
            labels.push((field.label, field.note));
            children.push(field.control);
        }
        Self {
            id: WidgetId::next(),
            labels,
            children,
            lines: Vec::new(),
            texts: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn label_width(&self) -> f32 {
        match self.labels.iter().any(|(label, _)| !label.is_empty()) {
            true => LABEL_WIDTH + LABEL_GAP,
            false => 0.0,
        }
    }
}

fn paragraph(cx: &mut Context, text: &str, role: TextRole, width: f32) -> Option<Paragraph> {
    (!text.is_empty()).then(|| {
        let ink = match role {
            TextRole::Body => cx.palette.ink,
            _ => cx.palette.ink2,
        };
        cx.text.paragraph(
            text,
            TextStyle::role(role, ink).tabular(),
            Some(width),
            TextAlign::Start,
        )
    })
}

impl Widget for Fields {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let leading = self.label_width();
        let mut lines = Vec::new();
        for ((label, note), child) in self.labels.iter().zip(&mut self.children) {
            let label = paragraph(cx, label, TextRole::Body, LABEL_WIDTH);
            let note = paragraph(cx, note, TextRole::Callout, available.width / 3.0);
            let note_width = note
                .as_ref()
                .map_or(0.0, |note| note.width().ceil() + NOTE_GAP);
            let width = (available.width - leading - note_width).max(0.0);
            let control = child.measure(cx, Size::new(width, available.height));
            let control = Size::new(control.width.min(width), control.height);
            let height = [
                control.height,
                label.as_ref().map_or(0.0, Paragraph::height),
                note.as_ref().map_or(0.0, Paragraph::height),
            ]
            .into_iter()
            .fold(0.0, f32::max)
            .ceil();
            lines.push(Line {
                label,
                note,
                height,
                control,
            });
        }
        let height = lines.iter().map(|line| line.height).sum::<f32>()
            + ROW_GAP * lines.len().saturating_sub(1) as f32;
        self.lines = lines;
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.texts.clear();
        let leading = self.label_width();
        let mut y = frame.y;
        for (line, child) in self.lines.iter().zip(&mut self.children) {
            let middle = |height: f32| (y + (line.height - height) / 2.0).round();
            if let Some(label) = &line.label {
                self.texts
                    .push((Point::new(frame.x, middle(label.height())), label.clone()));
            }
            if let Some(note) = &line.note {
                let x = frame.x + frame.width - note.width().ceil();
                self.texts
                    .push((Point::new(x, middle(note.height())), note.clone()));
            }
            child.place(
                cx,
                Rect::new(
                    frame.x + leading,
                    middle(line.control.height),
                    line.control.width,
                    line.control.height,
                ),
            );
            y += line.height + ROW_GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Group)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Strip {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Strip {
    pub fn new(children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            children,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn set_children(&mut self, children: Vec<Box<dyn Widget>>) {
        self.children = children;
    }
}

impl Widget for Strip {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let mut sizes: Vec<Size> = self
            .children
            .iter_mut()
            .skip(1)
            .map(|child| child.measure(cx, available))
            .collect();
        let fixed = sizes.iter().map(|size| size.width + STRIP_GAP).sum::<f32>();
        let first = match self.children.first_mut() {
            Some(first) => {
                let width = (available.width - fixed).max(0.0);
                let size = first.measure(cx, Size::new(width, available.height));
                Size::new(width, size.height)
            }
            None => Size::default(),
        };
        sizes.insert(0, first);
        let height = sizes.iter().map(|size| size.height).fold(0.0, f32::max);
        self.sizes = sizes;
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut x = frame.x;
        for (child, size) in self.children.iter_mut().zip(&self.sizes) {
            let y = frame.y + ((frame.height - size.height) / 2.0).round();
            child.place(cx, Rect::new(x, y, size.width, size.height));
            x += size.width + STRIP_GAP;
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

pub struct Banded {
    id: WidgetId,
    children: [Box<dyn Widget>; 2],
    band: f32,
    frame: Rect,
}

impl Banded {
    pub fn new(band: Box<dyn Widget>, body: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: [band, body],
            band: 0.0,
            frame: Rect::default(),
        }
    }
}

impl Widget for Banded {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let inner = Size::new(
            (available.width - 2.0 * BAND_PADDING).max(0.0),
            available.height,
        );
        self.band = self.children[0].measure(cx, inner).height + 2.0 * BAND_PADDING;
        self.children[1].measure(
            cx,
            Size::new(available.width, (available.height - self.band).max(0.0)),
        );
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let band = Rect::new(frame.x, frame.y, frame.width, self.band);
        self.children[0].place(cx, band.inset(BAND_PADDING));
        self.children[1].place(
            cx,
            Rect::new(
                frame.x,
                frame.y + self.band,
                frame.width,
                (frame.height - self.band).max(0.0),
            ),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: bande_toolkit::Shape::rect(Rect::new(
                self.frame.x,
                self.frame.y + self.band - 1.0,
                self.frame.width,
                1.0,
            )),
            paint: cx.palette.hairline.into(),
        });
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
