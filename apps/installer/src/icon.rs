use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Size, Widget, WidgetId};
use bande_toolkit::{Image, Item, Rect};

pub struct Icon {
    id: WidgetId,
    image: Image,
    side: f32,
    frame: Rect,
}

impl Icon {
    pub fn new(image: Image, side: f32) -> Self {
        Self {
            id: WidgetId::next(),
            image,
            side,
            frame: Rect::default(),
        }
    }
}

impl Widget for Icon {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(self.side, self.side)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Image {
            rect: Rect::new(self.frame.x, self.frame.y, self.side, self.side),
            image: self.image.clone(),
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Image)
    }
}
