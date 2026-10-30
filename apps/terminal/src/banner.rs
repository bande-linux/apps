use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role, TreeId, TreeUpdate};
use unbound_toolkit::ui::{
    Button, ClipboardRequest, Context, Cursor, Event, Key, Modifiers, Size, TextField, Ui, Widget,
    WidgetId,
};
use unbound_toolkit::{
    Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;

pub const BANNER_HEIGHT: f32 = 36.0;
const FIELD_WIDTH: f32 = 240.0;
const MIN_FIELD_WIDTH: f32 = 80.0;
const STATUS_ROOM: f32 = 80.0;
const MARGIN: f32 = 8.0;
const SPACING: f32 = 6.0;
const STATUS_SIZE: f32 = 12.0;
const MESSAGE_SIZE: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    QueryChanged,
    Older,
    Newer,
    Confirm,
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Find,
    Confirm,
}

struct Model {
    kind: Kind,
    query: String,
    status: Option<String>,
    actions: Vec<BannerAction>,
    area: Rect,
}

type Shared = Rc<RefCell<Model>>;

struct BannerView {
    id: WidgetId,
    model: Shared,
    leading: usize,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    status: Option<Paragraph>,
    frame: Rect,
}

impl BannerView {
    fn bar(&self) -> Rect {
        let area = self.model.borrow().area;
        Rect::new(area.x, area.y, area.width, BANNER_HEIGHT)
    }

    fn status_origin(&self, bar: Rect, status: &Paragraph) -> Point {
        let x = match self.leading.checked_sub(1) {
            Some(last) => {
                let after = self.children[last].frame();
                after.x + after.width + 2.0 * SPACING
            }
            None => bar.x + MARGIN + SPACING,
        };
        Point::new(x, (bar.y + (bar.height - status.height()) / 2.0).round())
    }
}

impl Widget for BannerView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = self.model.borrow().area.width;
        let bar = Size::new(width, BANNER_HEIGHT);
        let mut sizes: Vec<Size> = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, bar))
            .collect();
        let model = self.model.borrow();
        if model.kind == Kind::Find {
            let buttons: f32 = sizes.iter().skip(1).map(|size| size.width).sum();
            let room = width - 2.0 * MARGIN - buttons - SPACING * sizes.len() as f32;
            if let Some(field) = sizes.first_mut() {
                field.width = (room - STATUS_ROOM).clamp(MIN_FIELD_WIDTH, FIELD_WIDTH);
            }
        }
        self.sizes = sizes;
        let (size, color) = match model.kind {
            Kind::Find => (STATUS_SIZE, cx.palette.ink2),
            Kind::Confirm => (MESSAGE_SIZE, cx.palette.ink),
        };
        self.status = model.status.as_ref().map(|status| {
            cx.text
                .paragraph(status, TextStyle::new(size, color), None, TextAlign::Start)
        });
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let bar = self.bar();
        let centred = |height: f32| (bar.y + (bar.height - height) / 2.0).round();
        let (leading, trailing) = self.children.split_at_mut(self.leading);
        let (leading_sizes, trailing_sizes) = self.sizes.split_at(self.leading);
        let mut x = bar.x + MARGIN;
        for (child, size) in leading.iter_mut().zip(leading_sizes) {
            child.place(
                cx,
                Rect::new(x, centred(size.height), size.width, size.height),
            );
            x += size.width + SPACING;
        }
        let mut right = bar.x + bar.width - MARGIN;
        for (child, size) in trailing.iter_mut().zip(trailing_sizes).rev() {
            right -= size.width;
            child.place(
                cx,
                Rect::new(right, centred(size.height), size.width, size.height),
            );
            right -= SPACING;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let bar = self.bar();
        items.push(Item::Fill {
            shape: Shape::rect(bar),
            paint: cx.palette.sheet.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(bar.x, bar.y + bar.height - 1.0, bar.width, 1.0)),
            paint: cx.palette.hairline.into(),
        });
        if let Some(status) = &self.status {
            items.push(Item::Text {
                origin: self.status_origin(bar, status),
                paragraph: status.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let model = self.model.borrow();
        let (role, label) = match model.kind {
            Kind::Find => (Role::Toolbar, L10N.tr("find-bar")),
            Kind::Confirm => (Role::Alert, model.status.clone().unwrap_or_default()),
        };
        let mut node = Node::new(role);
        node.set_label(label);
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

fn action_button(model: &Shared, title: &str, action: BannerAction) -> Box<dyn Widget> {
    let model = model.clone();
    Box::new(Button::new(title, move || {
        model.borrow_mut().actions.push(action)
    }))
}

pub struct Banner {
    ui: Ui,
    focus: WidgetId,
    model: Shared,
}

impl Banner {
    fn new(
        palette: Palette,
        model: Shared,
        leading: usize,
        children: Vec<Box<dyn Widget>>,
        focus: WidgetId,
    ) -> Self {
        let view = BannerView {
            id: WidgetId::next(),
            model: model.clone(),
            leading,
            children,
            sizes: Vec::new(),
            status: None,
            frame: Rect::default(),
        };
        let mut banner = Self {
            ui: Ui::new(Box::new(view), Context::new(TextSystem::new(), palette)),
            focus,
            model,
        };
        banner.take_focus();
        banner
    }

    pub fn find(palette: Palette, query: &str) -> Self {
        let model: Shared = Rc::new(RefCell::new(Model {
            kind: Kind::Find,
            query: query.to_owned(),
            status: None,
            actions: Vec::new(),
            area: Rect::default(),
        }));
        let changed = model.clone();
        let field = TextField::new(&L10N.tr("find-placeholder"), query, move |text| {
            let mut model = changed.borrow_mut();
            model.query = text.to_owned();
            model.actions.push(BannerAction::QueryChanged);
        });
        let focus = field.id();
        let children = vec![
            Box::new(field) as Box<dyn Widget>,
            action_button(&model, &L10N.tr("find-previous"), BannerAction::Older),
            action_button(&model, &L10N.tr("find-next"), BannerAction::Newer),
            action_button(&model, &L10N.tr("find-done"), BannerAction::Dismiss),
        ];
        Self::new(palette, model, 3, children, focus)
    }

    pub fn confirm(palette: Palette, message: &str, confirm: &str) -> Self {
        let model: Shared = Rc::new(RefCell::new(Model {
            kind: Kind::Confirm,
            query: String::new(),
            status: Some(message.to_owned()),
            actions: Vec::new(),
            area: Rect::default(),
        }));
        let cancel = action_button(&model, &L10N.tr("cancel"), BannerAction::Dismiss);
        let confirm = action_button(&model, confirm, BannerAction::Confirm);
        let focus = confirm.id();
        Self::new(palette, model, 0, vec![cancel, confirm], focus)
    }

    pub fn take_focus(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: self.focus.into(),
            data: None,
        });
        if self.model.borrow().kind == Kind::Find {
            self.ui.handle(&Event::Key {
                key: Key::Character('a'),
                modifiers: Modifiers {
                    logo: true,
                    ..Modifiers::default()
                },
            });
        }
    }

    pub fn is_find(&self) -> bool {
        self.model.borrow().kind == Kind::Find
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.ui.context().palette = palette;
    }

    pub fn set_area(&mut self, area: Rect) {
        self.model.borrow_mut().area = area;
    }

    pub fn query(&self) -> String {
        self.model.borrow().query.clone()
    }

    pub fn set_found(&mut self, found: bool) {
        self.model.borrow_mut().status = (!found).then(|| L10N.tr("find-not-found"));
    }

    pub fn contains(&self, point: Point) -> bool {
        let area = self.model.borrow().area;
        point.x >= area.x
            && point.x < area.x + area.width
            && point.y >= area.y
            && point.y < area.y + BANNER_HEIGHT
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key { key, modifiers } = event
            && !modifiers.logo
            && !modifiers.control
        {
            let kind = self.model.borrow().kind;
            let action = match (kind, key) {
                (Kind::Find, Key::Enter) if modifiers.shift => Some(BannerAction::Newer),
                (Kind::Find, Key::Enter) => Some(BannerAction::Older),
                (Kind::Confirm, Key::Enter) => Some(BannerAction::Confirm),
                (_, Key::Escape) => Some(BannerAction::Dismiss),
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

    pub fn take_clipboard_request(&mut self) -> Option<ClipboardRequest> {
        self.ui.take_clipboard_request()
    }

    pub fn take_actions(&mut self) -> Vec<BannerAction> {
        std::mem::take(&mut self.model.borrow_mut().actions)
    }

    pub fn items(&mut self, width: u32, height: u32) -> Vec<Item> {
        self.ui.scene(width, height).items
    }

    pub fn cursor(&self) -> Cursor {
        self.ui.cursor()
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }
}
