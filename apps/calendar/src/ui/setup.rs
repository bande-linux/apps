use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId};

use unbound_toolkit::ui::{
    Button, Event, Key, Label, Modifiers, PopUpButton, SecureField, TEXT_FIELD_HEIGHT, TextField,
    Ui, WidgetId,
};

use super::panel::{Line, Panel, Width};
use super::{L10N, Signal, Signals, sender};

const WIDTH: f32 = 440.0;
const LABEL_WIDTH: f32 = 96.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CalDav,
    Subscription,
}

pub struct Setup {
    pub kind: Kind,
    pub user: String,
    pub server: String,
    pub password: Option<String>,
    password_field: Option<WidgetId>,
    collecting: bool,
    note: WidgetId,
    cancel: WidgetId,
    panel: WidgetId,
    dismissed: Rc<Cell<bool>>,
    pub busy: bool,
}

impl Setup {
    pub fn build(signals: &Signals) -> (Panel, Self) {
        let mut panel = Panel::new(Some(&L10N.tr("account-title")), WIDTH, LABEL_WIDTH).sheet();
        let send = sender(signals);
        let kinds = [L10N.tr("account-caldav"), L10N.tr("account-subscription")];
        let kind = panel.add(PopUpButton::new(
            &L10N.tr("account-kind"),
            &kinds,
            0,
            move |index| send(Signal::SetupKind(index)),
        ));
        panel.line(Line::new(
            Some(&L10N.tr("account-kind")),
            vec![(kind, Width::Fill)],
            ROW,
        ));
        let send = sender(signals);
        let user = panel.add(TextField::new("jane@example.com", "", move |text| {
            send(Signal::SetupUser(text.to_owned()))
        }));
        panel.line(Line::new(
            Some(&L10N.tr("account-user")),
            vec![(user, Width::Fill)],
            ROW,
        ));
        let (send, submit) = (sender(signals), sender(signals));
        let password = panel.add(SecureField::new("", 13.0, move |secret| {
            send(Signal::SetupPassword(secret.to_owned()));
            submit(Signal::SetupSubmit);
        }));
        panel.line(Line::new(
            Some(&L10N.tr("account-password")),
            vec![(password, Width::Fill)],
            ROW,
        ));
        let send = sender(signals);
        let server = panel.add(TextField::new(
            &L10N.tr("account-server-placeholder"),
            "",
            move |text| send(Signal::SetupServer(text.to_owned())),
        ));
        panel.line(Line::new(
            Some(&L10N.tr("account-server")),
            vec![(server, Width::Fill)],
            ROW,
        ));
        let note = panel.add(Label::new(""));
        panel.line(Line::new(None, vec![(note, Width::Fill)], 18.0));
        let send = sender(signals);
        let cancel =
            panel.add(Button::new(&L10N.tr("cancel"), move || send(Signal::SetupCancel)).cancel());
        let send = sender(signals);
        let sign_in = panel
            .add(Button::new(&L10N.tr("sign-in"), move || send(Signal::SetupSubmit)).primary());
        panel.line(
            Line::new(
                None,
                vec![(cancel, Width::Fixed(96.0)), (sign_in, Width::Fixed(96.0))],
                ROW,
            )
            .trailing(),
        );
        let setup = Self {
            kind: Kind::CalDav,
            user: String::new(),
            server: String::new(),
            password: None,
            password_field: panel.id_of(password),
            collecting: false,
            note: panel.id_of(note).unwrap_or_else(WidgetId::next),
            cancel: panel.id_of(cancel).unwrap_or_else(WidgetId::next),
            panel: unbound_toolkit::ui::Widget::id(&panel),
            dismissed: panel.dismisser(),
            busy: false,
        };
        (panel, setup)
    }

    pub fn apply(&mut self, signal: &Signal) {
        match signal {
            Signal::SetupKind(0) => self.kind = Kind::CalDav,
            Signal::SetupKind(_) => self.kind = Kind::Subscription,
            Signal::SetupUser(text) => self.user = text.trim().to_owned(),
            Signal::SetupServer(text) => self.server = text.trim().to_owned(),
            Signal::SetupPassword(secret) => {
                self.password = Some(secret.clone());
                self.collecting = false;
            }
            _ => {}
        }
    }

    pub fn collect_password(&mut self, ui: &mut Ui) -> bool {
        let Some(field) = self.password_field.filter(|_| !self.collecting) else {
            return false;
        };
        self.collecting = true;
        ui.set_focus(field);
        ui.handle(&Event::Key {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        true
    }

    pub fn show_error(&mut self, ui: &mut Ui, message: &str) {
        self.busy = false;
        self.password = None;
        self.show_note(ui, message);
    }

    pub fn show_note(&mut self, ui: &mut Ui, message: &str) {
        let note = self.note;
        let replacement = Label::new(message);
        let id = unbound_toolkit::ui::Widget::id(&replacement);
        if let Some(panel) = ui.widget_as::<Panel>(self.panel)
            && panel.replace(note, replacement)
        {
            self.note = id;
        }
    }

    pub fn dismiss(&self) {
        self.dismissed.set(true);
    }

    pub fn close(&self, ui: &mut Ui) {
        self.dismiss();
        ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: self.cancel.into(),
            data: None,
        });
    }
}
