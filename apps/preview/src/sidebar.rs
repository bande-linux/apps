use accesskit::{Action, ActionRequest, Node, NodeId, Role};
use bande_toolkit::ui::{
    Context, Event, Key, Parts, PointerButton, ScrollState, Size, Widget, WidgetId,
};
use bande_toolkit::{Image, Item, Paragraph, Point, Radius, Rect, Shape, TextStyle};

pub const SIDEBAR_WIDTH: f32 = 168.0;
const PICTURE: f32 = 104.0;
const LABEL_HEIGHT: f32 = 22.0;
const ROW: f32 = PICTURE + LABEL_HEIGHT + 14.0;
const SELECTION_PAD: f32 = 5.0;

pub struct Thumb {
    pub name: String,
    pub image: Option<Image>,
}

pub struct Sidebar {
    id: WidgetId,
    frame: Rect,
    thumbs: Vec<Thumb>,
    labels: Vec<Paragraph>,
    selected: usize,
    chosen: Option<usize>,
    scroll: ScrollState,
    reveal: bool,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            thumbs: Vec::new(),
            labels: Vec::new(),
            selected: 0,
            chosen: None,
            scroll: ScrollState::default(),
            reveal: true,
        }
    }

    pub fn set_thumbs(&mut self, thumbs: Vec<Thumb>) {
        self.thumbs = thumbs;
        self.labels.clear();
        self.selected = self.selected.min(self.thumbs.len().saturating_sub(1));
        self.reveal = true;
    }

    pub fn set_image(&mut self, index: usize, image: Image) {
        if let Some(thumb) = self.thumbs.get_mut(index) {
            thumb.image = Some(image);
        }
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index;
        self.reveal = true;
    }

    pub fn take_chosen(&mut self) -> Option<usize> {
        self.chosen.take()
    }

    pub fn visible_range(&self, cx: &Context) -> std::ops::Range<usize> {
        let offset = self.scroll.offset(cx);
        let first = (offset / ROW).floor().max(0.0) as usize;
        let last = ((offset + self.frame.height) / ROW).ceil() as usize + 1;
        first.min(self.thumbs.len())..last.min(self.thumbs.len())
    }

    fn row(&self, index: usize, offset: f32) -> Rect {
        Rect::new(
            self.frame.x,
            self.frame.y + index as f32 * ROW - offset,
            self.frame.width,
            ROW,
        )
    }

    fn choose(&mut self, index: usize) -> bool {
        if index >= self.thumbs.len() {
            return false;
        }
        self.selected = index;
        self.chosen = Some(index);
        self.reveal = true;
        true
    }
}

impl Widget for Sidebar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        if self.labels.len() != self.thumbs.len() {
            let style = TextStyle::new(11.0, cx.palette.ink);
            let width = frame.width - 16.0;
            self.labels = self
                .thumbs
                .iter()
                .map(|thumb| cx.text.fitted(&thumb.name, style, width))
                .collect();
        }
        self.scroll
            .set_extent(self.thumbs.len() as f32 * ROW, frame.height);
        if std::mem::take(&mut self.reveal) {
            let top = self.selected as f32 * ROW;
            self.scroll.reveal(cx, top, top + ROW);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        let mut content = Vec::new();
        for index in self.visible_range(cx) {
            let row = self.row(index, offset);
            let box_rect = Rect::new(
                row.x + (row.width - PICTURE) / 2.0,
                row.y + 6.0,
                PICTURE,
                PICTURE,
            );
            if index == self.selected {
                content.push(Item::Fill {
                    shape: Shape::rounded(box_rect.inset(-SELECTION_PAD), Radius::M),
                    paint: cx.palette.accent_soft.into(),
                });
            }
            if let Some(image) = self
                .thumbs
                .get(index)
                .and_then(|thumb| thumb.image.as_ref())
            {
                let (width, height) = (image.width() as f32, image.height() as f32);
                let scale = (PICTURE / width).min(PICTURE / height);
                let (shown_width, shown_height) = (width * scale, height * scale);
                content.push(Item::Image {
                    rect: Rect::new(
                        (box_rect.x + (PICTURE - shown_width) / 2.0).round(),
                        (box_rect.y + (PICTURE - shown_height) / 2.0).round(),
                        shown_width.round(),
                        shown_height.round(),
                    ),
                    image: image.clone(),
                });
            }
            if let Some(label) = self.labels.get(index) {
                content.push(Item::Text {
                    origin: Point::new(
                        (row.x + (row.width - label.width()) / 2.0).round(),
                        box_rect.y + PICTURE + SELECTION_PAD + 4.0,
                    ),
                    paragraph: label.clone(),
                });
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.frame),
            items: content,
        });
        self.scroll.paint_scrollbar(cx, self.frame, items);
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::ListBox);
        self.scroll.describe(cx, &mut node);
        node
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<(NodeId, Node)> = self
            .thumbs
            .iter()
            .enumerate()
            .map(|(index, thumb)| {
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(thumb.name.clone());
                node.set_selected(index == self.selected);
                node.add_action(Action::Click);
                (self.id.part(index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            focus: None,
            nodes,
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        request.action == Action::Click && self.choose(part as usize)
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        if self.scroll.pointer(cx, self.frame, event) {
            return true;
        }
        match event {
            Event::Scroll(scroll) => self.scroll.scroll(cx, scroll),
            Event::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && self.frame.contains(pointer.position) =>
            {
                let offset = self.scroll.offset(cx);
                let index = ((pointer.position.y - self.frame.y + offset) / ROW).floor();
                index >= 0.0 && self.choose(index as usize)
            }
            Event::Key { key, modifiers } if !modifiers.control && !modifiers.alt => match key {
                Key::Up => self.selected > 0 && self.choose(self.selected - 1),
                Key::Down => self.choose(self.selected + 1),
                _ => false,
            },
            _ => false,
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}
