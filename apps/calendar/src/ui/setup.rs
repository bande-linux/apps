use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId};

use bande_toolkit::ui::{
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    ICloud,
    Fastmail,
    Google,
    Nextcloud,
    Other,
    Subscription,
}

impl Preset {
    const ALL: [Self; 6] = [
        Self::ICloud,
        Self::Fastmail,
        Self::Google,
        Self::Nextcloud,
        Self::Other,
        Self::Subscription,
    ];

    fn title(self) -> String {
        L10N.tr(match self {
            Self::ICloud => "account-icloud",
            Self::Fastmail => "account-fastmail",
            Self::Google => "account-google",
            Self::Nextcloud => "account-nextcloud",
            Self::Other => "account-caldav",
            Self::Subscription => "account-subscription",
        })
    }

    pub fn hint(self) -> String {
        L10N.tr(match self {
            Self::ICloud => "account-icloud-hint",
            Self::Fastmail => "account-fastmail-hint",
            Self::Google => "account-google-hint",
            Self::Nextcloud => "account-nextcloud-hint",
            Self::Other => "account-caldav-hint",
            Self::Subscription => "account-subscription-hint",
        })
    }

    fn kind(self) -> Kind {
        match self {
            Self::Google | Self::Subscription => Kind::Subscription,
            _ => Kind::CalDav,
        }
    }

    fn server(self) -> Option<&'static str> {
        match self {
            Self::ICloud => Some("caldav.icloud.com"),
            Self::Fastmail => Some("caldav.fastmail.com"),
            _ => None,
        }
    }
}

pub struct Setup {
    pub kind: Kind,
    pub preset: Preset,
    pub account: Option<String>,
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
        let kinds: Vec<String> = Preset::ALL.iter().map(|preset| preset.title()).collect();
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
        let user = panel.add(
            TextField::new("jane@example.com", "", move |text| {
                send(Signal::SetupUser(text.to_owned()))
            })
            .without_text_checking(),
        );
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
        let server = panel.add(
            TextField::new(&L10N.tr("account-server-placeholder"), "", move |text| {
                send(Signal::SetupServer(text.to_owned()))
            })
            .without_text_checking(),
        );
        panel.line(Line::new(
            Some(&L10N.tr("account-server")),
            vec![(server, Width::Fill)],
            ROW,
        ));
        let note = panel.add(Label::new(&Preset::ICloud.hint()));
        panel.line(Line::new(None, vec![(note, Width::Fill)], 36.0));
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
            preset: Preset::ICloud,
            account: None,
            user: String::new(),
            server: String::new(),
            password: None,
            password_field: panel.id_of(password),
            collecting: false,
            note: panel.id_of(note).unwrap_or_else(WidgetId::next),
            cancel: panel.id_of(cancel).unwrap_or_else(WidgetId::next),
            panel: bande_toolkit::ui::Widget::id(&panel),
            dismissed: panel.dismisser(),
            busy: false,
        };
        (panel, setup)
    }

    pub fn sign_in(signals: &Signals, account: &str, name: &str, user: &str) -> (Panel, Self) {
        let title = L10N.tr_args("sign-in-title", [("name", name.to_owned().into())]);
        let mut panel = Panel::new(Some(&title), WIDTH, LABEL_WIDTH).sheet();
        let message = panel.add(Label::new(
            &L10N.tr_args("sign-in-message", [("user", user.to_owned().into())]),
        ));
        panel.line(Line::new(None, vec![(message, Width::Fill)], 36.0));
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
            preset: Preset::Other,
            account: Some(account.to_owned()),
            user: user.to_owned(),
            server: String::new(),
            password: None,
            password_field: panel.id_of(password),
            collecting: false,
            note: panel.id_of(note).unwrap_or_else(WidgetId::next),
            cancel: panel.id_of(cancel).unwrap_or_else(WidgetId::next),
            panel: bande_toolkit::ui::Widget::id(&panel),
            dismissed: panel.dismisser(),
            busy: false,
        };
        (panel, setup)
    }

    pub fn apply(&mut self, signal: &Signal) {
        match signal {
            Signal::SetupKind(index) => {
                self.preset = Preset::ALL.get(*index).copied().unwrap_or(Preset::Other);
                self.kind = self.preset.kind();
            }
            Signal::SetupUser(text) => self.user = text.trim().to_owned(),
            Signal::SetupServer(text) => self.server = text.trim().to_owned(),
            Signal::SetupPassword(secret) => {
                self.password = Some(secret.clone());
                self.collecting = false;
            }
            _ => {}
        }
    }

    pub fn password_field(&self) -> Option<WidgetId> {
        self.password_field
    }

    pub fn server(&self) -> String {
        match (self.server.is_empty(), self.preset.server()) {
            (true, Some(server)) => server.to_owned(),
            _ => self.server.clone(),
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
        let id = bande_toolkit::ui::Widget::id(&replacement);
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
