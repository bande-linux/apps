use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId};
use unbound_toolkit::ui::{Button, PopUpButton, TEXT_FIELD_HEIGHT, TextField, Ui, WidgetId};

use super::panel::{Line, Panel, Width};
use super::{L10N, Signal, Signals, sender};

const WIDTH: f32 = 380.0;
const LABEL_WIDTH: f32 = 72.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    New(Vec<String>),
    Rename(usize),
}

pub struct Naming {
    pub target: Target,
    pub name: String,
    pub account: usize,
    pub field: Option<WidgetId>,
    cancel: WidgetId,
    dismissed: Rc<Cell<bool>>,
}

impl Naming {
    pub fn build(
        signals: &Signals,
        target: Target,
        name: &str,
        accounts: &[String],
        chosen: usize,
    ) -> (Panel, Self) {
        let title = match target {
            Target::New(_) => L10N.tr("new-calendar-title"),
            Target::Rename(_) => L10N.tr("rename-calendar-title"),
        };
        let mut panel = Panel::new(Some(&title), WIDTH, LABEL_WIDTH).sheet();
        let send = sender(signals);
        let field = panel.add(TextField::new(
            &L10N.tr("calendar-name-placeholder"),
            name,
            move |text| send(Signal::NameText(text.to_owned())),
        ));
        panel.line(Line::new(
            Some(&L10N.tr("calendar-name")),
            vec![(field, Width::Fill)],
            ROW,
        ));
        if matches!(target, Target::New(_)) {
            let send = sender(signals);
            let account = panel.add(PopUpButton::new(
                &L10N.tr("calendar-account"),
                accounts,
                chosen,
                move |index| send(Signal::NameAccount(index)),
            ));
            panel.line(Line::new(
                Some(&L10N.tr("calendar-account")),
                vec![(account, Width::Fill)],
                ROW,
            ));
        }
        let send = sender(signals);
        let cancel =
            panel.add(Button::new(&L10N.tr("cancel"), move || send(Signal::NameCancel)).cancel());
        let send = sender(signals);
        let label = match target {
            Target::New(_) => L10N.tr("create"),
            Target::Rename(_) => L10N.tr("rename"),
        };
        let accept = panel.add(Button::new(&label, move || send(Signal::NameSubmit)).primary());
        panel.line(
            Line::new(
                None,
                vec![(cancel, Width::Fixed(96.0)), (accept, Width::Fixed(96.0))],
                ROW,
            )
            .separated()
            .trailing(),
        );
        let naming = Self {
            target,
            name: name.to_owned(),
            account: chosen,
            field: panel.id_of(field),
            cancel: panel.id_of(cancel).unwrap_or_else(WidgetId::next),
            dismissed: panel.dismisser(),
        };
        (panel, naming)
    }

    pub fn apply(&mut self, signal: &Signal) {
        match signal {
            Signal::NameText(text) => self.name = text.clone(),
            Signal::NameAccount(index) => self.account = *index,
            _ => {}
        }
    }

    pub fn close(&self, ui: &mut Ui) {
        self.dismissed.set(true);
        ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: self.cancel.into(),
            data: None,
        });
    }
}
