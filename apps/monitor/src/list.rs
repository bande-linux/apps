use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId, TreeUpdate};
use unbound_toolkit::ui::{Context, Cursor, Event, FreeLayout, Sort, Table, Ui, Widget, WidgetId};
use unbound_toolkit::{Item, Palette, Rect, TextSystem};

use crate::apps::GroupKey;
use crate::view::Request;

pub(crate) fn group_id(key: &GroupKey) -> String {
    match key {
        GroupKey::App(id) => format!("app:{id}"),
        GroupKey::Process { pid, start_ticks } => format!("group:{pid}:{start_ticks}"),
        GroupKey::Kernel => "kernel".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Signal {
    Selection(Vec<String>),
    Activate(Vec<String>),
    Sort(Sort),
    Tab(usize),
    Search(String),
    ClearSearch,
    Stop,
    Inspect,
    Choice(String),
}

pub(crate) type Signals = Rc<RefCell<Vec<Signal>>>;

pub(crate) fn sender(signals: &Signals) -> impl Fn(Signal) + 'static {
    let signals = signals.clone();
    move |signal| signals.borrow_mut().push(signal)
}

pub(crate) struct List {
    ui: Ui,
    host: WidgetId,
    table: WidgetId,
    signals: Signals,
}

impl List {
    pub(crate) fn new(table: Table, palette: Palette, text: TextSystem) -> Self {
        let signals: Signals = Rc::new(RefCell::new(Vec::new()));
        let (selection, activate, sort) = (sender(&signals), sender(&signals), sender(&signals));
        let table = table
            .on_selection(move |keys| selection(Signal::Selection(keys.to_vec())))
            .on_activate(move |keys| activate(Signal::Activate(keys.to_vec())))
            .on_sort(move |order| sort(Signal::Sort(order)));
        let table_id = Widget::id(&table);
        let mut host = FreeLayout::new();
        host.insert(table, Rect::default());
        let host_id = Widget::id(&host);
        let mut list = Self {
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            host: host_id,
            table: table_id,
            signals,
        };
        list.focus();
        list
    }

    pub(crate) fn signals(&self) -> &Signals {
        &self.signals
    }

    pub(crate) fn table(&mut self) -> Option<&mut Table> {
        self.ui.widget_as::<Table>(self.table)
    }

    pub(crate) fn shown_table(&self) -> Option<&Table> {
        self.ui.widget_ref::<Table>(self.table)
    }

    pub(crate) fn insert(&mut self, widget: impl Widget + 'static) -> WidgetId {
        self.ui
            .widget_as::<FreeLayout>(self.host)
            .map_or_else(WidgetId::next, |host| host.insert(widget, Rect::default()))
    }

    pub(crate) fn replace(&mut self, old: WidgetId, widget: impl Widget + 'static) -> WidgetId {
        let frame = self.frame_of(old);
        let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) else {
            return old;
        };
        host.remove(old);
        host.insert(widget, frame)
    }

    pub(crate) fn frame_of(&self, id: WidgetId) -> Rect {
        self.ui
            .widget_ref::<FreeLayout>(self.host)
            .and_then(|host| host.frame_of(id))
            .unwrap_or_default()
    }

    pub(crate) fn place(&mut self, id: WidgetId, frame: Rect) {
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.set_frame(id, frame);
        }
    }

    pub(crate) fn frame(&self) -> Rect {
        self.frame_of(self.table)
    }

    pub(crate) fn set_frame(&mut self, frame: Rect) {
        self.place(self.table, frame);
    }

    pub(crate) fn focus_widget(&mut self, id: WidgetId) {
        self.ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: id.into(),
            data: None,
        });
    }

    pub(crate) fn update<T: Widget + 'static>(
        &mut self,
        id: WidgetId,
        update: impl FnOnce(&mut T, &Context),
    ) {
        self.ui.update_widget(id, update);
    }

    pub(crate) fn focus(&mut self) {
        self.focus_widget(self.table);
    }

    pub(crate) fn focused(&self) -> Option<WidgetId> {
        self.ui.focus()
    }

    pub(crate) fn blur(&mut self) {
        self.ui.clear_focus();
    }

    pub(crate) fn present_dialog(&mut self, dialog: impl Widget + 'static) {
        self.ui.present_dialog(dialog);
    }

    pub(crate) fn has_dialog(&self) -> bool {
        self.ui.has_sheet()
    }

    pub(crate) fn set_palette(&mut self, palette: Palette) {
        self.ui.context().palette = palette;
    }

    pub(crate) fn text(&mut self) -> &mut TextSystem {
        &mut self.ui.context().text
    }

    pub(crate) fn handle(&mut self, event: &Event) -> bool {
        self.ui.handle(event)
    }

    pub(crate) fn perform(&mut self, request: &ActionRequest) -> bool {
        self.ui.perform(request)
    }

    pub(crate) fn menu_action(&mut self, action: &str) -> bool {
        self.ui.menu_action(action)
    }

    pub(crate) fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    pub(crate) fn pasted(&mut self, text: &str) -> bool {
        self.ui.pasted(text)
    }

    pub(crate) fn take_signals(&mut self) -> Vec<Signal> {
        std::mem::take(&mut *self.signals.borrow_mut())
    }

    pub(crate) fn take_requests(&mut self) -> Vec<Request> {
        let mut requests = Vec::new();
        if let Some((menu, at)) = self.ui.take_menu_request() {
            requests.push(Request::Menu(menu, at));
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            requests.push(Request::Clipboard(request));
        }
        if self.ui.take_emoji_picker_request() {
            requests.push(Request::EmojiPicker);
        }
        requests
    }

    pub(crate) fn paint(&mut self, width: u32, height: u32) -> (Vec<Item>, Vec<Item>) {
        self.ui.paint_layers(width, height)
    }

    pub(crate) fn motion(&mut self) -> (std::time::Instant, bool) {
        let cx = self.ui.context();
        (cx.now(), cx.reduce_motion)
    }

    pub(crate) fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub(crate) fn cursor(&self) -> Cursor {
        self.ui.cursor()
    }

    pub(crate) fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub(crate) fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }
}
