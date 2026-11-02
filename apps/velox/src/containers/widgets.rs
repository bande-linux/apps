use std::any::Any;

use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Cursor, Event, Size, Table, Widget, WidgetId};
use bande_toolkit::{Item, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle};

const TABLE_TEXT: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Primary,
    Secondary,
    Tertiary,
}

pub struct Text {
    id: WidgetId,
    text: String,
    role: TextRole,
    ink: Ink,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Text {
    pub fn new(text: &str, role: TextRole) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.to_owned(),
            role,
            ink: Ink::Primary,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn secondary(self) -> Self {
        Self {
            ink: Ink::Secondary,
            ..self
        }
    }

    pub fn tertiary(self) -> Self {
        Self {
            ink: Ink::Tertiary,
            ..self
        }
    }

    pub fn set_text(&mut self, text: &str) {
        text.clone_into(&mut self.text);
    }
}

impl Widget for Text {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let ink = match self.ink {
            Ink::Primary => cx.palette.ink,
            Ink::Secondary => cx.palette.ink2,
            Ink::Tertiary => cx.palette.ink3,
        };
        let paragraph = cx.text.paragraph(
            &self.text,
            TextStyle::role(self.role, ink),
            Some(available.width.max(1.0)),
            TextAlign::Start,
        );
        let size = Size::new(paragraph.width().ceil(), paragraph.height().ceil());
        self.paragraph = Some(paragraph);
        size
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

    pub fn set_main(&mut self, main: Box<dyn Widget>) {
        self.children[0] = main;
    }

    pub fn set_side(&mut self, side: Option<Box<dyn Widget>>) {
        self.children.truncate(1);
        self.children.extend(side);
    }

    pub fn has_side(&self) -> bool {
        self.children.len() > 1
    }

    fn main_width(&self) -> f32 {
        match self.has_side() {
            true => (self.frame.width - self.side - 1.0).max(0.0),
            false => self.frame.width,
        }
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
        let main = self.main_width();
        let frames = [
            Rect::new(frame.x, frame.y, main, frame.height),
            Rect::new(
                frame.x + main + 1.0,
                frame.y,
                (frame.width - main - 1.0).max(0.0),
                frame.height,
            ),
        ];
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
        if self.has_side() {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    self.frame.x + self.main_width(),
                    self.frame.y,
                    1.0,
                    self.frame.height,
                )),
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

pub struct PortLinks {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    column: Option<usize>,
    ports: Vec<Vec<u16>>,
    spots: Vec<(Rect, u16)>,
    visible: Vec<usize>,
    pressed: Option<u16>,
    on_open: Box<dyn FnMut(u16)>,
    frame: Rect,
}

pub fn port_text(ports: &[u16]) -> String {
    ports
        .iter()
        .map(|port| format!(":{port}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl PortLinks {
    pub fn new(table: Table, column: Option<usize>, on_open: impl FnMut(u16) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            child: [Box::new(table)],
            column,
            ports: Vec::new(),
            spots: Vec::new(),
            visible: Vec::new(),
            pressed: None,
            on_open: Box::new(on_open),
            frame: Rect::default(),
        }
    }

    pub fn set_ports(&mut self, ports: Vec<Vec<u16>>) {
        self.ports = ports;
    }

    pub fn set_column(&mut self, column: Option<usize>) {
        self.column = column;
    }

    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    fn spot(&self, point: Point) -> Option<u16> {
        self.spots
            .iter()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, port)| *port)
    }

    fn measure_spots(&mut self, cx: &mut Context) {
        let any: &dyn Any = self.child[0].as_ref();
        let Some(table) = any.downcast_ref::<Table>() else {
            return;
        };
        let top = self.frame.y + table.header_height();
        let bottom = self.frame.y + self.frame.height;
        self.visible = (0..table.row_count())
            .filter(|index| {
                table
                    .row_bounds(*index)
                    .is_some_and(|row| row.y + row.height > top && row.y < bottom)
            })
            .collect();
        let style = TextStyle::new(TABLE_TEXT, cx.palette.ink).tabular();
        let mut spots = Vec::new();
        let Some(column) = self.column else {
            self.spots = spots;
            return;
        };
        for index in &self.visible {
            let (Some(ports), Some(bounds)) =
                (self.ports.get(*index), table.text_bounds(*index, column))
            else {
                continue;
            };
            let mut shown = String::new();
            for port in ports {
                if !shown.is_empty() {
                    shown.push(' ');
                }
                let start = cx
                    .text
                    .paragraph(&shown, style, None, TextAlign::Start)
                    .width();
                shown.push_str(&format!(":{port}"));
                let end = cx
                    .text
                    .paragraph(&shown, style, None, TextAlign::Start)
                    .width()
                    .min(bounds.width);
                if end > start {
                    spots.push((
                        Rect::new(bounds.x + start, bounds.y, end - start, bounds.height),
                        *port,
                    ));
                }
            }
        }
        self.spots = spots;
    }
}

impl Widget for PortLinks {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.child[0].measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.child[0].place(cx, frame);
        self.measure_spots(cx);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.child[0].paint(cx, items);
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.pressed = self.spot(pointer.position);
                self.pressed.is_some()
            }
            Event::PointerUp(pointer) => {
                let pressed = self.pressed.take();
                match pressed.filter(|port| self.spot(pointer.position) == Some(*port)) {
                    Some(port) => {
                        (self.on_open)(port);
                        true
                    }
                    None => pressed.is_some(),
                }
            }
            _ => false,
        }
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        self.spot(point).map(|_| Cursor::Pointer)
    }

    fn hit_children(&self, point: Point) -> bool {
        self.pressed.is_none() && self.spot(point).is_none()
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
