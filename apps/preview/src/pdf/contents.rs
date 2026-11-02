use accesskit::{Action, ActionRequest, Node, NodeId, Role};
use bande_toolkit::ui::{
    Context, Event, Parts, PointerButton, ScrollState, Size, Widget, WidgetId,
};
use bande_toolkit::{Item, Paragraph, Point, Radius, Rect, Shape, TextStyle};

use crate::pdf::protocol::Heading;

const ROW: f32 = 26.0;
const INDENT: f32 = 12.0;
const SIDE: f32 = 10.0;
const NUMBER_WIDTH: f32 = 28.0;

pub struct Contents {
    id: WidgetId,
    frame: Rect,
    headings: Vec<Heading>,
    lines: Vec<(Paragraph, Option<Paragraph>)>,
    current: Option<usize>,
    chosen: Option<usize>,
    scroll: ScrollState,
}

impl Default for Contents {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Contents {
    pub fn new(headings: Vec<Heading>) -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            headings,
            lines: Vec::new(),
            current: None,
            chosen: None,
            scroll: ScrollState::default(),
        }
    }

    pub fn take_chosen(&mut self) -> Option<usize> {
        self.chosen.take()
    }

    fn choose(&mut self, index: usize) -> bool {
        let Some(page) = self.headings.get(index).and_then(|heading| heading.page) else {
            return false;
        };
        self.current = Some(index);
        self.chosen = Some(page);
        true
    }
}

impl Widget for Contents {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        let resized = frame.width != self.frame.width;
        self.frame = frame;
        if resized || self.lines.len() != self.headings.len() {
            let title = TextStyle::new(12.0, cx.palette.ink);
            let number = TextStyle::new(11.0, cx.palette.ink2);
            self.lines = self
                .headings
                .iter()
                .map(|heading| {
                    let indent = f32::from(heading.level.min(6)) * INDENT;
                    let width = (frame.width - 2.0 * SIDE - indent - NUMBER_WIDTH).max(20.0);
                    let page = heading.page.map(|page| {
                        cx.text
                            .fitted(&(page + 1).to_string(), number, NUMBER_WIDTH)
                    });
                    (cx.text.fitted(&heading.title, title, width), page)
                })
                .collect();
        }
        self.scroll
            .set_extent(self.headings.len() as f32 * ROW, frame.height);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        let first = (offset / ROW).floor().max(0.0) as usize;
        let last = (((offset + self.frame.height) / ROW).ceil() as usize + 1).min(self.lines.len());
        let mut content = Vec::new();
        for index in first..last {
            let y = self.frame.y + index as f32 * ROW - offset;
            if self.current == Some(index) {
                content.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(self.frame.x + 4.0, y, self.frame.width - 8.0, ROW),
                        Radius::S,
                    ),
                    paint: cx.palette.accent_soft.into(),
                });
            }
            let (Some((title, page)), Some(heading)) =
                (self.lines.get(index), self.headings.get(index))
            else {
                continue;
            };
            let indent = f32::from(heading.level.min(6)) * INDENT;
            content.push(Item::Text {
                origin: Point::new(
                    self.frame.x + SIDE + indent,
                    (y + (ROW - title.height()) / 2.0).round(),
                ),
                paragraph: title.clone(),
            });
            if let Some(page) = page {
                content.push(Item::Text {
                    origin: Point::new(
                        self.frame.x + self.frame.width - SIDE - page.width(),
                        (y + (ROW - page.height()) / 2.0).round(),
                    ),
                    paragraph: page.clone(),
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
        let mut node = Node::new(Role::Tree);
        self.scroll.describe(cx, &mut node);
        node
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<(NodeId, Node)> = self
            .headings
            .iter()
            .enumerate()
            .map(|(index, heading)| {
                let mut node = Node::new(Role::TreeItem);
                node.set_label(heading.title.clone());
                node.set_level(usize::from(heading.level) + 1);
                if heading.page.is_some() {
                    node.add_action(Action::Click);
                }
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
            _ => false,
        }
    }
}
