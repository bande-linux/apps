use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId, TreeUpdate};
use unbound_toolkit::ui::{
    ClipboardRequest, Context, Cursor, Event, FreeLayout, Sort, Table, Ui, Widget, WidgetId,
};
use unbound_toolkit::{Item, Palette, Rect, TextSystem};

use crate::apps::GroupKey;

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
    Activate,
    Sort(Sort),
}

pub(crate) struct List {
    ui: Ui,
    host: WidgetId,
    table: WidgetId,
    signals: Rc<RefCell<Vec<Signal>>>,
}

impl List {
    pub(crate) fn new(table: Table, palette: Palette, text: TextSystem) -> Self {
        let signals = Rc::new(RefCell::new(Vec::new()));
        let (selection, activate, sort) = (signals.clone(), signals.clone(), signals.clone());
        let table = table
            .on_selection(move |keys| {
                selection
                    .borrow_mut()
                    .push(Signal::Selection(keys.to_vec()))
            })
            .on_activate(move |_| activate.borrow_mut().push(Signal::Activate))
            .on_sort(move |order| sort.borrow_mut().push(Signal::Sort(order)));
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

    pub(crate) fn table(&mut self) -> Option<&mut Table> {
        self.ui.widget_as::<Table>(self.table)
    }

    pub(crate) fn shown_table(&self) -> Option<&Table> {
        self.ui.widget_ref::<Table>(self.table)
    }

    pub(crate) fn frame(&self) -> Rect {
        self.ui
            .widget_ref::<FreeLayout>(self.host)
            .and_then(|host| host.frame_of(self.table))
            .unwrap_or_default()
    }

    pub(crate) fn set_frame(&mut self, frame: Rect) {
        let table = self.table;
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.set_frame(table, frame);
        }
    }

    pub(crate) fn focus(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: self.table.into(),
            data: None,
        });
    }

    pub(crate) fn blur(&mut self) {
        self.ui.clear_focus();
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

    pub(crate) fn take_signals(&mut self) -> Vec<Signal> {
        std::mem::take(&mut *self.signals.borrow_mut())
    }

    pub(crate) fn take_copy(&mut self) -> Option<String> {
        match self.ui.take_clipboard_request() {
            Some(ClipboardRequest::Copy(text)) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn paint(&mut self, width: u32, height: u32) -> Vec<Item> {
        self.ui.paint_layers(width, height).0
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
}
