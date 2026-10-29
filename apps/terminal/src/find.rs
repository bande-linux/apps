use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role, TreeId, TreeUpdate};
use unbound_toolkit::ui::{Button, Context, Event, Key, Size, TextField, Ui, Widget, WidgetId};
use unbound_toolkit::{
    Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;

pub const FIND_BAR_HEIGHT: f32 = 36.0;
const FIELD_WIDTH: f32 = 240.0;
const MIN_FIELD_WIDTH: f32 = 80.0;
const STATUS_ROOM: f32 = 80.0;
const MARGIN: f32 = 8.0;
const SPACING: f32 = 6.0;
const STATUS_SIZE: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindAction {
    Changed,
    Older,
    Newer,
    Close,
}

#[derive(Default)]
struct Model {
    query: String,
    actions: Vec<FindAction>,
    top: f32,
    not_found: bool,
}

type Shared = Rc<RefCell<Model>>;

struct BarView {
    id: WidgetId,
    model: Shared,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    status: Option<Paragraph>,
    frame: Rect,
}

impl BarView {
    fn bar(&self) -> Rect {
        Rect::new(
            self.frame.x,
            self.model.borrow().top,
            self.frame.width,
            FIND_BAR_HEIGHT,
        )
    }
}

impl Widget for BarView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let bar = Size::new(available.width, FIND_BAR_HEIGHT);
        let mut sizes: Vec<Size> = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, bar))
            .collect();
        let buttons: f32 = sizes.iter().skip(1).map(|size| size.width).sum();
        let room = available.width - 2.0 * MARGIN - buttons - SPACING * sizes.len() as f32;
        if let Some(field) = sizes.first_mut() {
            field.width = (room - STATUS_ROOM).clamp(MIN_FIELD_WIDTH, FIELD_WIDTH);
        }
        self.sizes = sizes;
        self.status = self.model.borrow().not_found.then(|| {
            cx.text.paragraph(
                &L10N.tr("find-not-found"),
                TextStyle::new(STATUS_SIZE, cx.palette.label_muted),
                None,
                TextAlign::Start,
            )
        });
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let bar = self.bar();
        let (done, rest) = self.children.split_last_mut().unzip();
        let mut x = bar.x + MARGIN;
        let centred = |height: f32| (bar.y + (bar.height - height) / 2.0).round();
        if let Some(rest) = rest {
            for (child, size) in rest.iter_mut().zip(&self.sizes) {
                child.place(
                    cx,
                    Rect::new(x, centred(size.height), size.width, size.height),
                );
                x += size.width + SPACING;
            }
        }
        if let (Some(done), Some(size)) = (done, self.sizes.last()) {
            let x = bar.x + bar.width - MARGIN - size.width;
            done.place(
                cx,
                Rect::new(x, centred(size.height), size.width, size.height),
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let bar = self.bar();
        items.push(Item::Fill {
            shape: Shape::rect(bar),
            paint: cx.palette.surface_background.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(bar.x, bar.y + bar.height - 1.0, bar.width, 1.0)),
            paint: cx.palette.separator_color.into(),
        });
        for child in &self.children {
            child.paint(cx, items);
        }
        if let (Some(status), Some(after)) = (&self.status, self.children.iter().rev().nth(1)) {
            let after = after.frame();
            items.push(Item::Text {
                origin: Point::new(
                    after.x + after.width + 2.0 * SPACING,
                    (bar.y + (bar.height - status.height()) / 2.0).round(),
                ),
                paragraph: status.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Toolbar);
        node.set_label(L10N.tr("find-bar"));
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct FindBar {
    ui: Ui,
    field: WidgetId,
    model: Shared,
}

fn action_button(model: &Shared, title: &str, action: FindAction) -> Box<dyn Widget> {
    let model = model.clone();
    Box::new(Button::new(title, move || {
        model.borrow_mut().actions.push(action)
    }))
}

impl FindBar {
    pub fn new(palette: Palette, query: &str) -> Self {
        let model: Shared = Rc::new(RefCell::new(Model {
            query: query.to_owned(),
            ..Model::default()
        }));
        let changed = model.clone();
        let field = TextField::new(&L10N.tr("find-placeholder"), query, move |text| {
            let mut model = changed.borrow_mut();
            model.query = text.to_owned();
            model.actions.push(FindAction::Changed);
        });
        let field_id = field.id();
        let view = BarView {
            id: WidgetId::next(),
            model: model.clone(),
            children: vec![
                Box::new(field),
                action_button(&model, &L10N.tr("find-previous"), FindAction::Older),
                action_button(&model, &L10N.tr("find-next"), FindAction::Newer),
                action_button(&model, &L10N.tr("find-done"), FindAction::Close),
            ],
            sizes: Vec::new(),
            status: None,
            frame: Rect::default(),
        };
        let mut bar = Self {
            ui: Ui::new(Box::new(view), Context::new(TextSystem::new(), palette)),
            field: field_id,
            model,
        };
        bar.focus_field();
        bar
    }

    pub fn focus_field(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: self.field.into(),
            data: None,
        });
        self.ui.handle(&Event::Key {
            key: Key::Character('a'),
            modifiers: unbound_toolkit::ui::Modifiers {
                logo: true,
                ..Default::default()
            },
        });
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.ui.context().palette = palette;
    }

    pub fn set_top(&mut self, top: f32) {
        self.model.borrow_mut().top = top;
    }

    pub fn query(&self) -> String {
        self.model.borrow().query.clone()
    }

    pub fn set_found(&mut self, found: bool) {
        self.model.borrow_mut().not_found = !found;
    }

    pub fn contains(&self, point: Point) -> bool {
        let top = self.model.borrow().top;
        point.y >= top && point.y < top + FIND_BAR_HEIGHT
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key { key, modifiers } = event
            && !modifiers.logo
            && !modifiers.control
        {
            let action = match key {
                Key::Enter if modifiers.shift => Some(FindAction::Newer),
                Key::Enter => Some(FindAction::Older),
                Key::Escape => Some(FindAction::Close),
                _ => None,
            };
            if let Some(action) = action {
                self.model.borrow_mut().actions.push(action);
                return true;
            }
        }
        self.ui.handle(event)
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        self.ui.perform(request)
    }

    pub fn take_actions(&mut self) -> Vec<FindAction> {
        std::mem::take(&mut self.model.borrow_mut().actions)
    }

    pub fn items(&mut self, width: u32, height: u32) -> Vec<Item> {
        self.ui.scene(width, height).items
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }
}
