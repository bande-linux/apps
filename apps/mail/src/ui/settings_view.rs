use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_app_settings::{Stored, settings_folder};
use bande_toolkit::ui::{
    Alert, Axis, Button, Context, Form, FormLine, FreeLayout, Label, PopUpButton, SEGMENTED_HEIGHT,
    ScrollView, SegmentedControl, Stack, TextField, TextView, Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Header, Rect, Scene, TextSystem, Window, WindowEvent};

use crate::L10N;
use crate::discover::Settings as Servers;
use crate::net::{Network, Security, Server};
use crate::secrets::Keyring;
use crate::settings::{Change, MAX_PREVIEW_LINES, MailSettings, announce};
use crate::setup::{Outcome, Request as SetupRequest, check};
use crate::store::{Account, AccountId, Rule, RuleAction, RuleField, Store};
use crate::ui::view::TOOLBAR_HEIGHT;
use crate::worker::{Event as MailEvent, Mailroom};

pub const SETTINGS_SIZE: (u32, u32) = (620, 620);
const TAB_WIDTH: f32 = 92.0;
const SECURITIES: [Security; 3] = [Security::Tls, Security::StartTls, Security::Plain];
const SIGNATURE_LINES: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Accounts,
    Viewing,
    Composing,
    Rules,
}

const TABS: [Tab; 5] = [
    Tab::General,
    Tab::Accounts,
    Tab::Viewing,
    Tab::Composing,
    Tab::Rules,
];

impl Tab {
    fn title(self) -> String {
        L10N.tr(match self {
            Self::General => "settings-general",
            Self::Accounts => "settings-accounts",
            Self::Viewing => "settings-viewing",
            Self::Composing => "settings-composing",
            Self::Rules => "settings-rules",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Tab(usize),
    Account(usize),
    Settings,
    Identity,
    Enabled(bool),
    SaveServers,
    AddAccount,
    RemoveAccount,
    Remove(AccountId),
    AddRule,
    RemoveRule(i64),
}

#[derive(Debug, Clone, Default)]
struct Editing {
    name: String,
    signature: String,
    user: String,
    incoming: (String, String, usize),
    outgoing: (String, String, usize),
}

impl Editing {
    fn of(account: &Account) -> Self {
        let server = |server: &Server| {
            (
                server.host.clone(),
                server.port.to_string(),
                SECURITIES
                    .iter()
                    .position(|security| *security == server.security)
                    .unwrap_or_default(),
            )
        };
        Self {
            name: account.name.clone(),
            signature: account.signature.clone(),
            user: account.user.clone(),
            incoming: server(&account.imap),
            outgoing: server(&account.smtp),
        }
    }

    fn servers(&self) -> Option<Servers> {
        let server = |(host, port, security): &(String, String, usize)| {
            let host = host.trim();
            Some(Server {
                host: (!host.is_empty()).then(|| host.to_owned())?,
                port: port.trim().parse().ok().filter(|port| *port > 0)?,
                security: SECURITIES.get(*security).copied()?,
            })
        };
        let user = self.user.trim();
        Some(Servers {
            user: (!user.is_empty()).then(|| user.to_owned())?,
            imap: server(&self.incoming)?,
            smtp: server(&self.outgoing)?,
        })
    }
}

#[derive(Default)]
struct State {
    settings: MailSettings,
    editing: Editing,
    signals: Vec<Signal>,
    rule: (usize, String, usize),
}

type Shared = Rc<RefCell<State>>;

fn sender(state: &Shared, signal: Signal) -> impl Fn() + 'static {
    let state = Rc::clone(state);
    move || state.borrow_mut().signals.push(signal.clone())
}

fn toggle(state: &Shared, label: &str, on: bool, set: fn(&mut MailSettings, bool)) -> Toggle {
    let state = Rc::clone(state);
    Toggle::new(label, on, move |on| {
        let mut state = state.borrow_mut();
        set(&mut state.settings, on);
        state.signals.push(Signal::Settings);
    })
}

fn edited(
    state: &Shared,
    signal: Option<Signal>,
    set: fn(&mut Editing, &str),
) -> impl FnMut(&str) + 'static {
    let state = Rc::clone(state);
    move |text| {
        let mut state = state.borrow_mut();
        set(&mut state.editing, text);
        if let Some(signal) = &signal
            && !state.signals.contains(signal)
        {
            state.signals.push(signal.clone());
        }
    }
}

fn field(state: &Shared, text: &str, placeholder: &str, set: fn(&mut Editing, &str)) -> TextField {
    TextField::new(&L10N.tr(placeholder), text, edited(state, None, set))
}

fn security(state: &Shared, chosen: usize, set: fn(&mut Editing, usize)) -> PopUpButton {
    let names = SECURITIES
        .iter()
        .map(|security| {
            L10N.tr(match security {
                Security::Tls => "security-tls",
                Security::StartTls => "security-starttls",
                Security::Plain => "security-none",
            })
        })
        .collect::<Vec<_>>();
    let state = Rc::clone(state);
    PopUpButton::new(
        &L10N.tr("settings-security"),
        &names,
        chosen,
        move |index| {
            set(&mut state.borrow_mut().editing, index);
        },
    )
}

fn rule_field(field: RuleField) -> &'static str {
    match field {
        RuleField::From => "rule-from",
        RuleField::To => "rule-to",
        RuleField::Subject => "rule-subject",
    }
}

fn heading(message: &str) -> FormLine {
    FormLine::Heading(L10N.tr(message))
}

fn row(message: &str, control: impl Widget + 'static) -> FormLine {
    FormLine::Row(L10N.tr(message), Box::new(control))
}

fn note(message: &str) -> FormLine {
    FormLine::Wide(Box::new(Label::new(&L10N.tr(message)).secondary()))
}

fn buttons(buttons: Vec<Button>) -> FormLine {
    FormLine::Wide(Box::new(
        Stack::new(
            Axis::Horizontal,
            8.0,
            0.0,
            buttons
                .into_iter()
                .map(|button| Box::new(button) as Box<dyn Widget>)
                .collect(),
        )
        .trailing(),
    ))
}

pub struct SettingsView {
    ui: Ui,
    host: WidgetId,
    tabs: WidgetId,
    content: WidgetId,
    tab: Tab,
    state: Shared,
    folder: Option<PathBuf>,
    store: Option<Store>,
    accounts: Vec<Account>,
    account: usize,
    status: Option<(String, bool)>,
    checking: Option<AccountId>,
    store_path: PathBuf,
    mailroom: Option<Mailroom>,
    network: Network,
    width: f32,
}

impl SettingsView {
    pub fn new(store_path: PathBuf, appearance: Appearance, text: TextSystem) -> Self {
        let folder = settings_folder();
        let settings = folder
            .as_deref()
            .map(MailSettings::load)
            .unwrap_or_default();
        let state: Shared = Rc::new(RefCell::new(State {
            settings,
            ..State::default()
        }));
        let titles = TABS.iter().map(|tab| tab.title()).collect::<Vec<_>>();
        let choose = Rc::clone(&state);
        let tabs =
            SegmentedControl::new(&L10N.tr("settings-title"), &titles, Some(0), move |index| {
                choose.borrow_mut().signals.push(Signal::Tab(index))
            })
            .fill_equally();
        let mut host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let tabs = host.insert_in_header(tabs, Rect::default());
        let content = host.insert(Label::new(""), Rect::default());
        let mut ui = Ui::new(Box::new(host), Context::new(text, appearance.palette()));
        ui.set_appearance(appearance);
        ui.set_sheet_top(TOOLBAR_HEIGHT);
        let store = Store::open(&store_path)
            .inspect_err(|error| tracing::warn!("cannot read the accounts: {error}"))
            .ok();
        let mut view = Self {
            ui,
            host: host_id,
            tabs,
            content,
            tab: Tab::General,
            state,
            folder,
            accounts: Vec::new(),
            store,
            account: 0,
            status: None,
            checking: None,
            store_path: store_path.clone(),
            mailroom: Mailroom::new()
                .inspect_err(|error| tracing::warn!("cannot check servers: {error}"))
                .ok(),
            network: Network::default(),
            width: 0.0,
        };
        view.load_accounts();
        view.rebuild();
        view
    }

    fn load_accounts(&mut self) -> bool {
        let accounts = self
            .store
            .as_ref()
            .and_then(|store| store.accounts().ok())
            .unwrap_or_default();
        if accounts == self.accounts {
            return false;
        }
        self.accounts = accounts;
        self.choose_account(self.account.min(self.accounts.len().saturating_sub(1)));
        true
    }

    fn choose_account(&mut self, index: usize) {
        self.account = index;
        self.status = None;
        self.state.borrow_mut().editing = self
            .accounts
            .get(index)
            .map(Editing::of)
            .unwrap_or_default();
    }

    fn rebuild(&mut self) {
        let form = match self.tab {
            Tab::General => self.general(),
            Tab::Accounts => self.accounts_form(),
            Tab::Viewing => self.viewing(),
            Tab::Composing => self.composing(),
            Tab::Rules => self.rules(),
        };
        let content = self.content;
        let replaced = self
            .ui
            .widget_as::<FreeLayout>(self.host)
            .and_then(|host| host.replace(content, ScrollView::new(form)));
        if let Some(content) = replaced {
            self.content = content;
        }
    }

    fn general(&self) -> Form {
        let settings = self.state.borrow().settings.clone();
        Form::new(
            &Tab::General.title(),
            vec![
                heading("settings-new-messages"),
                row(
                    "settings-notify",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-notify"),
                        settings.notify,
                        |settings, on| settings.notify = on,
                    ),
                ),
                row(
                    "settings-sound",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-sound"),
                        settings.sound,
                        |settings, on| settings.sound = on,
                    ),
                ),
                heading("settings-default-reader"),
                note("settings-default-reader-note"),
            ],
        )
    }

    fn viewing(&self) -> Form {
        let settings = self.state.borrow().settings.clone();
        let lines = (0..=MAX_PREVIEW_LINES)
            .map(|lines| match lines {
                0 => L10N.tr("settings-preview-none"),
                lines => L10N.tr_args("settings-preview-lines", [("lines", lines.into())]),
            })
            .collect::<Vec<_>>();
        let state = Rc::clone(&self.state);
        let preview = PopUpButton::new(
            &L10N.tr("settings-preview"),
            &lines,
            usize::from(settings.preview_lines),
            move |index| {
                let mut state = state.borrow_mut();
                state.settings.preview_lines = u8::try_from(index).unwrap_or(MAX_PREVIEW_LINES);
                state.signals.push(Signal::Settings);
            },
        );
        Form::new(
            &Tab::Viewing.title(),
            vec![
                heading("settings-message-list"),
                row("settings-preview", preview),
                row(
                    "settings-conversations",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-conversations"),
                        settings.conversations,
                        |settings, on| settings.conversations = on,
                    ),
                ),
                heading("settings-message"),
                row(
                    "settings-plain-text",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-plain-text"),
                        settings.plain_text,
                        |settings, on| settings.plain_text = on,
                    ),
                ),
                row(
                    "settings-fixed-width",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-fixed-width"),
                        settings.fixed_width,
                        |settings, on| settings.fixed_width = on,
                    ),
                ),
                heading("settings-privacy"),
                row(
                    "settings-load-remote",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-load-remote"),
                        settings.load_remote,
                        |settings, on| settings.load_remote = on,
                    ),
                ),
                note("settings-load-remote-note"),
            ],
        )
    }

    fn rule_actions(&self) -> Vec<(String, RuleAction)> {
        let mut actions = vec![
            (L10N.tr("rule-flag"), RuleAction::Flag),
            (L10N.tr("rule-read"), RuleAction::Read),
        ];
        let Some(store) = &self.store else {
            return actions;
        };
        for account in &self.accounts {
            for mailbox in store.mailboxes(account.id).unwrap_or_default() {
                if mailbox.selectable {
                    actions.push((
                        L10N.tr_args(
                            "rule-move",
                            [
                                ("mailbox", mailbox.path.clone().into()),
                                ("account", account.email.clone().into()),
                            ],
                        ),
                        RuleAction::Move(mailbox.id),
                    ));
                }
            }
        }
        actions
    }

    fn rules(&self) -> Form {
        let fields = RuleField::ALL
            .iter()
            .map(|field| L10N.tr(rule_field(*field)))
            .collect::<Vec<_>>();
        let actions = self.rule_actions();
        let mut lines = vec![heading("settings-rules-list")];
        let rules = self
            .store
            .as_ref()
            .and_then(|store| store.rules().ok())
            .unwrap_or_default();
        if rules.is_empty() {
            lines.push(note("settings-no-rules"));
        }
        for rule in rules {
            let action = actions
                .iter()
                .find(|(_, action)| *action == rule.action)
                .map(|(title, _)| title.clone())
                .unwrap_or_default();
            let description = L10N.tr_args(
                "rule-description",
                [
                    ("field", L10N.tr(rule_field(rule.field)).into()),
                    ("pattern", rule.pattern.clone().into()),
                    ("action", action.into()),
                ],
            );
            lines.push(FormLine::Row(
                description,
                Box::new(Button::new(
                    &L10N.tr("button-remove"),
                    sender(&self.state, Signal::RemoveRule(rule.id)),
                )),
            ));
        }
        let (field, pattern, action) = self.state.borrow().rule.clone();
        let state = Rc::clone(&self.state);
        let field = PopUpButton::new(&L10N.tr("rule-if"), &fields, field, move |index| {
            state.borrow_mut().rule.0 = index;
        });
        let state = Rc::clone(&self.state);
        let pattern = TextField::new("", &pattern, move |text| {
            state.borrow_mut().rule.1 = text.to_owned();
        });
        let titles = actions
            .iter()
            .map(|(title, _)| title.clone())
            .collect::<Vec<_>>();
        let state = Rc::clone(&self.state);
        let action = PopUpButton::new(&L10N.tr("rule-then"), &titles, action, move |index| {
            state.borrow_mut().rule.2 = index;
        });
        lines.extend([
            heading("settings-new-rule"),
            row("rule-if", field),
            row("rule-contains", pattern),
            row("rule-then", action),
            buttons(vec![Button::new(
                &L10N.tr("rule-add"),
                sender(&self.state, Signal::AddRule),
            )]),
            note("settings-rules-note"),
        ]);
        Form::new(&Tab::Rules.title(), lines)
    }

    fn add_rule(&mut self) {
        let (field, pattern, action) = self.state.borrow().rule.clone();
        let actions = self.rule_actions();
        let (Some(store), Some(field), Some((_, action))) =
            (&self.store, RuleField::ALL.get(field), actions.get(action))
        else {
            return;
        };
        if pattern.trim().is_empty() {
            return;
        }
        let rule = Rule {
            id: 0,
            field: *field,
            pattern: pattern.trim().to_owned(),
            action: *action,
        };
        match store.add_rule(&rule) {
            Ok(()) => self.state.borrow_mut().rule.1.clear(),
            Err(error) => tracing::warn!("cannot add the rule: {error}"),
        }
    }

    fn composing(&self) -> Form {
        let settings = self.state.borrow().settings.clone();
        let mut choices = vec![L10N.tr("settings-send-from-mailbox")];
        choices.extend(self.accounts.iter().map(|account| account.email.clone()));
        let chosen = self
            .accounts
            .iter()
            .position(|account| account.email.eq_ignore_ascii_case(&settings.send_from))
            .map_or(0, |index| index + 1);
        let state = Rc::clone(&self.state);
        let emails = self
            .accounts
            .iter()
            .map(|account| account.email.clone())
            .collect::<Vec<_>>();
        let send_from = PopUpButton::new(
            &L10N.tr("settings-send-from"),
            &choices,
            chosen,
            move |index| {
                let mut state = state.borrow_mut();
                state.settings.send_from = index
                    .checked_sub(1)
                    .and_then(|index| emails.get(index).cloned())
                    .unwrap_or_default();
                state.signals.push(Signal::Settings);
            },
        );
        Form::new(
            &Tab::Composing.title(),
            vec![
                heading("settings-addressing"),
                row("settings-send-from", send_from),
                heading("settings-responding"),
                row(
                    "settings-quote",
                    toggle(
                        &self.state,
                        &L10N.tr("settings-quote"),
                        settings.quote,
                        |settings, on| settings.quote = on,
                    ),
                ),
                note("settings-signature-note"),
            ],
        )
    }

    fn accounts_form(&self) -> Form {
        let add = Button::new(
            &L10N.tr("menu-add-account"),
            sender(&self.state, Signal::AddAccount),
        );
        let Some(account) = self.accounts.get(self.account) else {
            return Form::new(
                &Tab::Accounts.title(),
                vec![note("settings-no-accounts"), buttons(vec![add])],
            );
        };
        let editing = self.state.borrow().editing.clone();
        let emails = self
            .accounts
            .iter()
            .map(|account| account.email.clone())
            .collect::<Vec<_>>();
        let choose = Rc::clone(&self.state);
        let chooser = PopUpButton::new(
            &L10N.tr("settings-account"),
            &emails,
            self.account,
            move |index| choose.borrow_mut().signals.push(Signal::Account(index)),
        );
        let remove = Button::new(
            &L10N.tr("settings-remove-account"),
            sender(&self.state, Signal::RemoveAccount),
        );
        let name = TextField::new(
            &L10N.tr("placeholder-name"),
            &editing.name,
            edited(&self.state, Some(Signal::Identity), |editing, text| {
                editing.name = text.to_owned();
            }),
        );
        let signature = TextView::new(
            &L10N.tr("settings-signature-placeholder"),
            &editing.signature,
            edited(&self.state, Some(Signal::Identity), |editing, text| {
                editing.signature = text.to_owned();
            }),
        )
        .visible_lines(SIGNATURE_LINES);
        let mut save = Button::new(
            &L10N.tr("settings-save-servers"),
            sender(&self.state, Signal::SaveServers),
        );
        save.set_enabled(self.checking.is_none());
        let mut lines = vec![
            row("settings-account", chooser),
            buttons(vec![add, remove]),
            heading("settings-account-information"),
            row("settings-enabled", {
                let state = Rc::clone(&self.state);
                Toggle::new(&L10N.tr("settings-enabled"), account.enabled, move |on| {
                    state.borrow_mut().signals.push(Signal::Enabled(on));
                })
            }),
            row("settings-email", Label::new(&account.email)),
            row("settings-name", name),
            heading("settings-signature"),
            FormLine::Wide(Box::new(signature)),
            heading("settings-server-settings"),
            row(
                "settings-user",
                field(
                    &self.state,
                    &editing.user,
                    "placeholder-user",
                    |editing, text| {
                        editing.user = text.to_owned();
                    },
                ),
            ),
            row(
                "settings-incoming",
                field(
                    &self.state,
                    &editing.incoming.0,
                    "placeholder-incoming",
                    |editing, text| editing.incoming.0 = text.to_owned(),
                ),
            ),
            row(
                "settings-port",
                field(
                    &self.state,
                    &editing.incoming.1,
                    "placeholder-port",
                    |editing, text| editing.incoming.1 = text.to_owned(),
                ),
            ),
            row(
                "settings-security",
                security(&self.state, editing.incoming.2, |editing, index| {
                    editing.incoming.2 = index;
                }),
            ),
            row(
                "settings-outgoing",
                field(
                    &self.state,
                    &editing.outgoing.0,
                    "placeholder-outgoing",
                    |editing, text| editing.outgoing.0 = text.to_owned(),
                ),
            ),
            row(
                "settings-port",
                field(
                    &self.state,
                    &editing.outgoing.1,
                    "placeholder-port",
                    |editing, text| editing.outgoing.1 = text.to_owned(),
                ),
            ),
            row(
                "settings-security",
                security(&self.state, editing.outgoing.2, |editing, index| {
                    editing.outgoing.2 = index;
                }),
            ),
        ];
        if let Some((status, _)) = &self.status {
            lines.push(FormLine::Wide(Box::new(Label::new(status).secondary())));
        }
        lines.push(buttons(vec![save]));
        Form::new(&Tab::Accounts.title(), lines)
    }

    fn save_settings(&self) {
        let settings = self.state.borrow().settings.clone();
        let Some(folder) = &self.folder else {
            return;
        };
        match std::fs::create_dir_all(folder).and_then(|()| settings.save(folder)) {
            Ok(()) => announce(Change::Settings),
            Err(error) => tracing::warn!("cannot save {}: {error}", MailSettings::FILE),
        }
    }

    fn save_identity(&mut self) {
        let editing = self.state.borrow().editing.clone();
        let (Some(store), Some(account)) = (&self.store, self.accounts.get_mut(self.account))
        else {
            return;
        };
        account.name = editing.name.trim().to_owned();
        account.signature = editing.signature;
        match store.update_account(account) {
            Ok(()) => announce(Change::Accounts),
            Err(error) => tracing::warn!("cannot save {}: {error}", account.email),
        }
    }

    fn save_servers(&mut self) {
        let (Some(account), Some(mailroom)) = (self.accounts.get(self.account), &self.mailroom)
        else {
            return;
        };
        let Some(servers) = self.state.borrow().editing.servers() else {
            self.status = Some((L10N.tr("setup-missing-servers"), true));
            return;
        };
        if (&servers.user, &servers.imap, &servers.smtp)
            == (&account.user, &account.imap, &account.smtp)
        {
            return;
        }
        self.status = Some((L10N.tr("checking-account"), false));
        self.checking = Some(account.id);
        let (id, email, old_host) = (account.id, account.email.clone(), account.imap.host.clone());
        let (network, store_path) = (self.network.clone(), self.store_path.clone());
        let notifier = mailroom.notifier();
        let spawned = std::thread::Builder::new()
            .name("check-servers".to_owned())
            .spawn(move || {
                let outcome = match Keyring::session_bus()
                    .and_then(|keyring| Ok((keyring.password(&email, &old_host)?, keyring)))
                {
                    Ok((Some(password), keyring)) => {
                        let request = SetupRequest {
                            name: String::new(),
                            email: email.clone(),
                            password,
                            servers: Some(servers.clone()),
                        };
                        match check(&request, &network) {
                            Outcome::Ready(checked) => {
                                let kept = Store::open(&store_path)
                                    .and_then(|store| store.update_servers(id, &servers))
                                    .and_then(|()| match servers.imap.host == old_host {
                                        true => Ok(()),
                                        false => keyring
                                            .save(&email, &servers.imap.host, &request.password)
                                            .and_then(|()| keyring.forget(&email, &old_host)),
                                    });
                                match kept {
                                    Ok(()) => {
                                        announce(Change::Accounts);
                                        Outcome::Ready(checked)
                                    }
                                    Err(error) => {
                                        Outcome::NeedsServers(Some(servers), error.to_string())
                                    }
                                }
                            }
                            outcome => outcome,
                        }
                    }
                    Ok((None, _)) => {
                        Outcome::WrongPassword(servers, "no saved password".to_owned())
                    }
                    Err(error) => Outcome::NeedsServers(Some(servers), error.to_string()),
                };
                notifier.send(MailEvent::Setup(Box::new(outcome)));
            });
        if let Err(error) = spawned {
            self.checking = None;
            self.status = Some((error.to_string(), true));
        }
    }

    fn checked(&mut self, outcome: Outcome) {
        let Some(id) = self.checking.take() else {
            return;
        };
        self.status = Some(match outcome {
            Outcome::Ready(servers) => {
                if let Some(account) = self.accounts.iter_mut().find(|account| account.id == id) {
                    account.user = servers.user;
                    account.imap = servers.imap;
                    account.smtp = servers.smtp;
                }
                (L10N.tr("settings-servers-saved"), false)
            }
            Outcome::WrongPassword(..) => (L10N.tr("setup-wrong-password"), true),
            Outcome::NeedsServers(_, reason) => (
                L10N.tr_args("setup-no-servers", [("reason", reason.into())]),
                true,
            ),
        });
    }

    fn confirm_removal(&mut self) {
        let Some(account) = self.accounts.get(self.account) else {
            return;
        };
        let (state, id) = (Rc::clone(&self.state), account.id);
        let alert = Alert::new(
            &L10N.tr_args("remove-title", [("email", account.email.clone().into())]),
            &L10N.tr("remove-message"),
            move |choice| {
                if choice == "remove" {
                    state.borrow_mut().signals.push(Signal::Remove(id));
                }
            },
        )
        .destructive(&L10N.tr("button-remove"), "remove")
        .cancel(&L10N.tr("button-cancel"), "cancel");
        self.ui.present_sheet(alert);
    }

    fn settle(&mut self) -> bool {
        let signals = std::mem::take(&mut self.state.borrow_mut().signals);
        let mut rebuild = false;
        for signal in signals {
            match signal {
                Signal::Tab(index) => {
                    self.tab = TABS.get(index).copied().unwrap_or(Tab::General);
                    rebuild = true;
                }
                Signal::Account(index) => {
                    self.choose_account(index);
                    rebuild = true;
                }
                Signal::Settings => self.save_settings(),
                Signal::Identity => self.save_identity(),
                Signal::Enabled(on) => {
                    if let Some(account) = self.accounts.get_mut(self.account) {
                        account.enabled = on;
                    }
                    self.save_identity();
                }
                Signal::SaveServers => {
                    self.save_servers();
                    rebuild = true;
                }
                Signal::AddAccount => announce(Change::AddAccount),
                Signal::AddRule => {
                    self.add_rule();
                    rebuild = true;
                }
                Signal::RemoveRule(id) => {
                    if let Some(store) = &self.store
                        && let Err(error) = store.remove_rule(id)
                    {
                        tracing::warn!("cannot remove the rule: {error}");
                    }
                    rebuild = true;
                }
                Signal::RemoveAccount => self.confirm_removal(),
                Signal::Remove(id) => {
                    announce(Change::Remove(id));
                    self.accounts.retain(|account| account.id != id);
                    self.choose_account(0);
                    rebuild = true;
                }
            }
        }
        if rebuild {
            self.rebuild();
        }
        rebuild
    }

    fn layout(&mut self, width: f32, height: f32) {
        self.width = width;
        let tabs_width = TAB_WIDTH * TABS.len() as f32;
        let tabs = Rect::new(
            ((width - tabs_width) / 2.0).round(),
            ((TOOLBAR_HEIGHT - SEGMENTED_HEIGHT) / 2.0).round(),
            tabs_width,
            SEGMENTED_HEIGHT,
        );
        let content = Rect::new(
            0.0,
            TOOLBAR_HEIGHT,
            width,
            (height - TOOLBAR_HEIGHT).max(0.0),
        );
        let (tabs_id, content_id) = (self.tabs, self.content);
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.set_frame(tabs_id, tabs);
            host.set_frame(content_id, content);
        }
    }

    fn header(&self) -> Header {
        let tabs_width = TAB_WIDTH * TABS.len() as f32;
        Header {
            title_x: self.width,
            items: vec![Rect::new(
                ((self.width - tabs_width) / 2.0).round(),
                0.0,
                tabs_width,
                TOOLBAR_HEIGHT,
            )],
            tabs: Vec::new(),
        }
    }

    pub fn render(&mut self, width: u32, height: u32) -> Scene {
        self.layout(width as f32, height as f32);
        let mut scene = self.ui.scene(width, height);
        scene.background = self.ui.context().palette.sheet;
        scene
    }
}

impl View for SettingsView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let scene = self.render(width, height);
        window.set_header(self.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(&mut self.ui, window, event);
        self.settle() || handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
        if !active {
            return;
        }
        let stored = self
            .folder
            .as_deref()
            .map(MailSettings::load)
            .unwrap_or_default();
        let changed = stored != self.state.borrow().settings;
        if changed {
            self.state.borrow_mut().settings = stored;
        }
        if self.load_accounts() || changed {
            self.rebuild();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.mailroom.iter().map(Mailroom::fd).collect()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        let events = self
            .mailroom
            .as_mut()
            .map(Mailroom::take)
            .unwrap_or_default();
        for event in events {
            if let MailEvent::Setup(outcome) = event {
                self.checked(*outcome);
            }
        }
        self.rebuild();
        true
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
