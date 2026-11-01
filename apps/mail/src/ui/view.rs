use std::cell::RefCell;
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use bande_toolkit::ui::{
    ClipboardRequest, Context, Cursor, Event, FreeLayout, Key, Menu, MenuItem, Modifiers, Pointer,
    PointerButton, ScrollState, Ui, Widget, WidgetId, shortcut_pressed,
};
use bande_toolkit::{
    Appearance, Header, Palette, Point, Rect, SHEET_INSET, TextSystem, WINDOW_HEADER_HEIGHT,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use zeroize::Zeroizing;

use crate::L10N;
use crate::compose::{Draft, Outgoing, forward, reply, signed};
use crate::discover::Settings;
use crate::error::Result;
use crate::imap::{Flag, Role};
use crate::message::{Content, Envelope};
use crate::net::Network;
use crate::secrets::Keyring;
use crate::settings::MailSettings;
use crate::setup::{Outcome, Request as SetupRequest, check};
use crate::store::{Account, AccountId, Mailbox, MessageId, Store, Summary};
use crate::ui::format::full_date;
use crate::ui::reader::{Layout, Reading, Remote};
use crate::ui::setup_sheet::{AccountSheet, Action, Field, Mode, Shared, SheetState};
use crate::ui::sidebar::{Place, Row, rows};
use crate::ui::symbols::Symbols;
use crate::ui::toolbar::{Signal, Signals, Toolbar};
use crate::worker::{Command, Event as MailEvent, Mailroom, Status, Worker};

pub const SHORTCUTS: [(&str, &str); 12] = [
    ("compose", "Ctrl+N"),
    ("get-mail", "Shift+Ctrl+N"),
    ("find", "Ctrl+F"),
    ("reply", "Ctrl+R"),
    ("reply-all", "Shift+Ctrl+R"),
    ("forward", "Shift+Ctrl+F"),
    ("toggle-read", "Shift+Ctrl+U"),
    ("toggle-flag", "Shift+Ctrl+L"),
    ("archive", "Shift+Ctrl+A"),
    ("junk", "Shift+Ctrl+J"),
    ("delete", "Delete"),
    ("send", "Shift+Ctrl+D"),
];
pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const LIST_WIDTH: f32 = 340.0;
pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const NOTIFIED_AT_ONCE: usize = 3;
const MAX_PICTURE_BYTES: usize = 10 << 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Compose(Box<Outgoing>),
    OpenLink(String),
    OpenAttachment(PathBuf),
    Notify {
        summary: String,
        body: String,
        sound: bool,
    },
    NotSent {
        error: String,
        outgoing: Box<Outgoing>,
    },
    ShowMenu(Menu, Point),
    Clipboard(ClipboardRequest),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Place(usize),
    Message(MessageId),
    Link(String),
    Attachment(usize),
    LoadRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Sidebar,
    List,
}

pub(crate) struct SetupSheet {
    pub(crate) state: Shared,
    pub(crate) mode: Mode,
    pub(crate) password_field: WidgetId,
}

pub struct MailView {
    pub(crate) store: Store,
    store_path: PathBuf,
    pub(crate) accounts: Vec<(Account, Vec<Mailbox>)>,
    workers: Vec<Worker>,
    pub(crate) statuses: Vec<(AccountId, Status)>,
    mailroom: Mailroom,
    network: Network,
    pub(crate) sidebar: Vec<Row>,
    pub(crate) place: Place,
    pub(crate) messages: Vec<Summary>,
    pub(crate) selected: Option<MessageId>,
    pub(crate) reading: Option<Reading>,
    pub(crate) layout: Option<(f32, Layout)>,
    pub(crate) search: String,
    pub(crate) ui: Ui,
    pub(crate) host: WidgetId,
    pub(crate) toolbar: Toolbar,
    pub(crate) signals: Signals,
    pub(crate) palette: Palette,
    pub(crate) symbols: Symbols,
    pub(crate) size: (f32, f32),
    pub(crate) sheet_inset: f32,
    pub(crate) scrolls: [ScrollState; 3],
    pub(crate) hits: Vec<(Rect, Hit)>,
    pub(crate) hover: Option<Hit>,
    pub(crate) focus: Pane,
    pub(crate) zone: TimeZone,
    pub(crate) now: Option<Timestamp>,
    requests: Vec<Request>,
    setup: Option<SetupSheet>,
    pressed_ui: bool,
    pub(crate) settings: MailSettings,
}

fn keyring_password(email: String, host: String) -> crate::worker::PasswordSource {
    Box::new(move || Keyring::session_bus()?.password(&email, &host))
}

impl MailView {
    pub fn new(
        store_path: PathBuf,
        settings: MailSettings,
        palette: Palette,
        text: TextSystem,
    ) -> Result<Self> {
        let store = Store::open(&store_path)?;
        let signals: Signals = Rc::new(RefCell::new(Vec::new()));
        let host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let mut view = Self {
            store,
            store_path,
            accounts: Vec::new(),
            workers: Vec::new(),
            statuses: Vec::new(),
            mailroom: Mailroom::new()?,
            network: Network::default(),
            sidebar: Vec::new(),
            place: Place::All(Role::Inbox),
            messages: Vec::new(),
            selected: None,
            reading: None,
            layout: None,
            search: String::new(),
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            host: host_id,
            toolbar: Toolbar::default(),
            signals,
            palette,
            symbols: Symbols::default(),
            size: (0.0, 0.0),
            sheet_inset: SHEET_INSET as f32,
            scrolls: Default::default(),
            hits: Vec::new(),
            hover: None,
            focus: Pane::List,
            zone: TimeZone::system(),
            now: None,
            requests: Vec::new(),
            setup: None,
            pressed_ui: false,
            settings,
        };
        view.insert_toolbar();
        view.ui.set_sheet_top(TOOLBAR_HEIGHT);
        view.reload();
        Ok(view)
    }

    pub fn start(&mut self) {
        for account in self.accounts() {
            self.spawn_worker(account);
        }
        if self.accounts.is_empty() {
            self.present_setup(Mode::Add);
        }
    }

    fn spawn_worker(&mut self, account: Account) {
        let source = keyring_password(account.email.clone(), account.imap.host.clone());
        self.workers.push(Worker::spawn(
            account,
            self.store_path.clone(),
            source,
            self.network.clone(),
            self.mailroom.notifier(),
        ));
    }

    pub fn set_settings(&mut self, settings: MailSettings) {
        self.settings = settings;
        self.layout = None;
    }

    pub fn accounts_changed(&mut self) {
        let stored = self.store.accounts().unwrap_or_default();
        let known = self.accounts();
        let connects_alike = |a: &Account, b: &Account| {
            (&a.email, &a.user, &a.imap, &a.smtp) == (&b.email, &b.user, &b.imap, &b.smtp)
        };
        let restarted = stored
            .iter()
            .filter(|account| {
                !known
                    .iter()
                    .any(|old| old.id == account.id && connects_alike(old, account))
            })
            .cloned()
            .collect::<Vec<_>>();
        self.workers.retain(|worker| {
            stored.iter().any(|account| account.id == worker.account)
                && !restarted.iter().any(|account| account.id == worker.account)
        });
        self.statuses
            .retain(|(id, _)| stored.iter().any(|account| account.id == *id));
        let same = restarted.is_empty() && stored.len() == known.len();
        for account in restarted {
            self.spawn_worker(account);
        }
        match same {
            true => self.refresh_sidebar(),
            false => self.reload(),
        }
    }

    pub fn has_accounts(&self) -> bool {
        !self.accounts.is_empty()
    }

    pub fn accounts(&self) -> Vec<Account> {
        self.accounts
            .iter()
            .map(|(account, _)| account.clone())
            .collect()
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.mailroom.fd()]
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn messages(&self) -> &[Summary] {
        &self.messages
    }

    pub fn selected(&self) -> Option<MessageId> {
        self.selected
    }

    pub fn place(&self) -> &Place {
        &self.place
    }

    pub fn sidebar(&self) -> &[Row] {
        &self.sidebar
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.layout = None;
    }

    pub fn set_window_active(&mut self, active: bool) -> bool {
        self.ui.set_window_active(active)
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.ui.context().palette = palette;
        self.layout = None;
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        self.sheet_inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
    }

    pub fn title(&self) -> String {
        self.sidebar
            .iter()
            .find_map(|row| match row {
                Row::Place { place, title, .. } if *place == self.place => Some(title.clone()),
                _ => None,
            })
            .unwrap_or_else(|| L10N.tr("app-name"))
    }

    pub fn header(&self) -> Header {
        self.toolbar_header()
    }

    pub fn cursor(&self) -> Cursor {
        match self.hover {
            Some(Hit::Link(_) | Hit::Attachment(_) | Hit::LoadRemote) => Cursor::Pointer,
            _ => self.ui.cursor(),
        }
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = self.ui.perform(request);
        self.after_ui();
        handled
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        let handled = self.ui.pasted(text);
        self.after_ui();
        handled
    }

    pub fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    pub(crate) fn account(&self, id: AccountId) -> Option<&Account> {
        self.accounts
            .iter()
            .map(|(account, _)| account)
            .find(|account| account.id == id)
    }

    fn worker(&self, account: AccountId) -> Option<&Worker> {
        self.workers.iter().find(|worker| worker.account == account)
    }

    pub fn send_command(&self, account: AccountId, command: Command) {
        if let Some(worker) = self.worker(account) {
            worker.send(command);
        }
    }

    fn flush(&self, accounts: &[AccountId]) {
        for account in accounts {
            self.send_command(*account, Command::Flush);
        }
    }

    fn refresh_sidebar(&mut self) {
        let accounts = self.store.accounts().unwrap_or_default();
        self.accounts = accounts
            .into_iter()
            .map(|account| {
                let mailboxes = self.store.mailboxes(account.id).unwrap_or_default();
                (account, mailboxes)
            })
            .collect();
        let store = &self.store;
        self.sidebar = rows(&self.accounts, |scope| store.unread(scope).unwrap_or(0));
    }

    pub fn reload(&mut self) {
        self.refresh_sidebar();
        let exists = self
            .sidebar
            .iter()
            .any(|row| matches!(row, Row::Place { place, .. } if *place == self.place));
        if !exists {
            self.place = Place::All(Role::Inbox);
        }
        self.messages = self
            .store
            .summaries(&self.place.scope(), &self.search)
            .unwrap_or_default();
        if self
            .selected
            .is_some_and(|id| !self.messages.iter().any(|summary| summary.id == id))
        {
            self.selected = None;
            self.reading = None;
            self.layout = None;
        }
    }

    pub fn choose_place(&mut self, place: Place) {
        if place == self.place {
            return;
        }
        self.place = place;
        self.selected = None;
        self.reading = None;
        self.layout = None;
        self.scrolls[1] = ScrollState::default();
        self.reload();
    }

    pub fn search_for(&mut self, text: &str) {
        self.search = text.to_owned();
        self.scrolls[1] = ScrollState::default();
        self.reload();
    }

    pub fn select(&mut self, id: MessageId) {
        let Some(summary) = self
            .messages
            .iter()
            .find(|summary| summary.id == id)
            .cloned()
        else {
            return;
        };
        self.selected = Some(id);
        self.focus = Pane::List;
        self.scrolls[2] = ScrollState::default();
        self.load_reading(&summary);
        if !summary.flags.seen {
            self.set_flag_on(&[id], Flag::Seen, true);
        }
        self.reveal_selected();
    }

    fn load_reading(&mut self, summary: &Summary) {
        let body = self.store.body(summary.id).ok().flatten();
        let reading = match body {
            Some(raw) => Reading::new(Content::parse(&raw), true),
            None => {
                let header = self
                    .store
                    .header(summary.id)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                self.send_command(summary.account, Command::Body(summary.id));
                Reading::new(
                    Content {
                        envelope: Envelope::parse(&header),
                        ..Content::default()
                    },
                    false,
                )
            }
        };
        let remote = reading.downloaded && reading.document.remote_images > 0;
        self.reading = Some(reading);
        self.layout = None;
        if remote && self.settings.load_remote {
            self.load_remote();
        }
    }

    fn selected_summary(&self) -> Option<Summary> {
        let id = self.selected?;
        self.messages
            .iter()
            .find(|summary| summary.id == id)
            .cloned()
    }

    fn set_flag_on(&mut self, ids: &[MessageId], flag: Flag, value: bool) {
        match self.store.set_flag(ids, flag, value) {
            Ok(accounts) => self.flush(&accounts),
            Err(error) => tracing::warn!("cannot change a flag: {error}"),
        }
        for summary in self
            .messages
            .iter_mut()
            .filter(|summary| ids.contains(&summary.id))
        {
            match flag {
                Flag::Seen => summary.flags.seen = value,
                Flag::Flagged => summary.flags.flagged = value,
                Flag::Answered => summary.flags.answered = value,
                Flag::Deleted => summary.flags.deleted = value,
                Flag::Draft => summary.flags.draft = value,
            }
        }
        self.refresh_sidebar();
    }

    pub fn toggle_flag(&mut self) {
        if let Some(summary) = self.selected_summary() {
            self.set_flag_on(&[summary.id], Flag::Flagged, !summary.flags.flagged);
        }
    }

    pub fn toggle_read(&mut self) {
        if let Some(summary) = self.selected_summary() {
            self.set_flag_on(&[summary.id], Flag::Seen, !summary.flags.seen);
        }
    }

    fn mailbox_with_role(&self, account: AccountId, role: Role) -> Option<Mailbox> {
        self.accounts
            .iter()
            .find(|(known, _)| known.id == account)
            .and_then(|(_, mailboxes)| {
                mailboxes
                    .iter()
                    .find(|mailbox| mailbox.role == Some(role))
                    .cloned()
            })
    }

    fn mailbox_of(&self, summary: &Summary) -> Option<Mailbox> {
        self.accounts
            .iter()
            .flat_map(|(_, mailboxes)| mailboxes)
            .find(|mailbox| mailbox.id == summary.mailbox)
            .cloned()
    }

    pub fn can_move_to(&self, role: Role) -> bool {
        self.selected_summary().is_some_and(|summary| {
            self.mailbox_with_role(summary.account, role)
                .is_some_and(|target| target.id != summary.mailbox)
        })
    }

    fn leave_selected(&mut self, id: MessageId) {
        let position = self.messages.iter().position(|summary| summary.id == id);
        self.messages.retain(|summary| summary.id != id);
        self.selected = None;
        self.reading = None;
        self.layout = None;
        let next = position
            .and_then(|position| {
                self.messages.get(position).or_else(|| {
                    position
                        .checked_sub(1)
                        .and_then(|before| self.messages.get(before))
                })
            })
            .map(|summary| summary.id);
        if let Some(next) = next {
            self.select(next);
        }
        self.refresh_sidebar();
    }

    pub fn move_selected(&mut self, role: Role) {
        let Some(summary) = self.selected_summary() else {
            return;
        };
        let Some(target) = self.mailbox_with_role(summary.account, role) else {
            return;
        };
        if target.id == summary.mailbox {
            return;
        }
        match self.store.move_messages(&[summary.id], &target) {
            Ok(accounts) => self.flush(&accounts),
            Err(error) => tracing::warn!("cannot move the message: {error}"),
        }
        self.leave_selected(summary.id);
    }

    pub fn delete_selected(&mut self) {
        let Some(summary) = self.selected_summary() else {
            return;
        };
        let in_trash = self
            .mailbox_of(&summary)
            .is_some_and(|mailbox| mailbox.role == Some(Role::Trash));
        let trash = self.mailbox_with_role(summary.account, Role::Trash);
        if trash.is_some() && !in_trash {
            self.move_selected(Role::Trash);
            return;
        }
        match self.store.expunge(&[summary.id]) {
            Ok(accounts) => self.flush(&accounts),
            Err(error) => tracing::warn!("cannot delete the message: {error}"),
        }
        self.leave_selected(summary.id);
    }

    fn default_account(&self) -> Option<Account> {
        let account = self
            .selected_summary()
            .map(|summary| summary.account)
            .or(match self.place {
                Place::Mailbox(id) => self
                    .accounts
                    .iter()
                    .find(|(_, mailboxes)| mailboxes.iter().any(|mailbox| mailbox.id == id))
                    .map(|(account, _)| account.id),
                _ => None,
            });
        account
            .and_then(|id| self.account(id).cloned())
            .or_else(|| self.accounts.first().map(|(account, _)| account.clone()))
    }

    fn sending_account(&self) -> Option<Account> {
        self.accounts
            .iter()
            .map(|(account, _)| account)
            .find(|account| account.email.eq_ignore_ascii_case(&self.settings.send_from))
            .cloned()
            .or_else(|| self.default_account())
    }

    pub fn compose(&mut self, mut draft: Draft) {
        let Some(account) = self.sending_account() else {
            self.present_setup(Mode::Add);
            return;
        };
        if draft.from.email.is_empty() {
            draft.from = account.address();
        }
        draft.body = match draft.body.trim_end() {
            "" => signed("", &account.signature),
            body => format!("{body}{}", signed("", &account.signature)),
        };
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: None,
            replaces: None,
        })));
    }

    pub fn respond(&mut self, all: bool) {
        let (Some(summary), Some(reading)) = (self.selected_summary(), self.reading.as_ref())
        else {
            return;
        };
        let Some(account) = self.account(summary.account) else {
            return;
        };
        let envelope = &reading.content.envelope;
        let attribution = L10N.tr_args(
            "reply-attribution",
            [
                (
                    "date",
                    envelope
                        .date
                        .map(|date| full_date(date, &self.zone))
                        .unwrap_or_default()
                        .into(),
                ),
                ("name", envelope.sender().shown().to_owned().into()),
            ],
        );
        let quote = self.settings.quote.then_some(attribution.as_str());
        let mut draft = reply(&reading.content, &account.address(), all, quote);
        draft.body = signed(&draft.body, &account.signature);
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: Some(summary.id),
            replaces: None,
        })));
    }

    pub fn continue_draft(&mut self) {
        let (Some(summary), Some(reading)) = (self.selected_summary(), self.reading.as_ref())
        else {
            return;
        };
        let is_draft = summary.flags.draft
            || self
                .mailbox_of(&summary)
                .is_some_and(|mailbox| mailbox.role == Some(Role::Drafts));
        if !is_draft || !reading.downloaded {
            return;
        }
        let Some(account) = self.account(summary.account) else {
            return;
        };
        let envelope = &reading.content.envelope;
        let draft = Draft {
            from: account.address(),
            to: envelope.to.clone(),
            cc: envelope.cc.clone(),
            bcc: Vec::new(),
            subject: envelope.subject.clone(),
            body: reading.content.text().replace("\r\n", "\n"),
            in_reply_to: envelope.references.last().cloned(),
            references: envelope.references.clone(),
            attachments: reading.content.attachments.clone(),
        };
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: None,
            replaces: Some(summary.id),
        })));
    }

    pub fn forward_selected(&mut self) {
        let (Some(summary), Some(reading)) = (self.selected_summary(), self.reading.as_ref())
        else {
            return;
        };
        let Some(account) = self.account(summary.account) else {
            return;
        };
        let envelope = &reading.content.envelope;
        let names = |addresses: &[crate::message::Address]| {
            addresses
                .iter()
                .map(crate::message::Address::full)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let heading = L10N.tr_args(
            "forward-heading",
            [
                ("from", names(&envelope.from).into()),
                (
                    "date",
                    envelope
                        .date
                        .map(|date| full_date(date, &self.zone))
                        .unwrap_or_default()
                        .into(),
                ),
                ("subject", envelope.subject.clone().into()),
                ("to", names(&envelope.to).into()),
            ],
        );
        let mut draft = forward(&reading.content, &account.address(), &heading);
        draft.body = signed(&draft.body, &account.signature);
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: None,
            replaces: None,
        })));
    }

    pub fn get_mail(&mut self) {
        for worker in &self.workers {
            worker.send(Command::Sync);
        }
    }

    fn account_for(&self, draft: &Draft) -> Option<AccountId> {
        self.accounts
            .iter()
            .map(|(account, _)| account)
            .find(|account| account.email.eq_ignore_ascii_case(&draft.from.email))
            .map(|account| account.id)
    }

    pub fn save_draft(&mut self, outgoing: Outgoing) -> bool {
        let Some(account) = self.account_for(&outgoing.draft) else {
            return false;
        };
        self.send_command(account, Command::SaveDraft(Box::new(outgoing)));
        true
    }

    pub fn send(&mut self, outgoing: Outgoing) -> bool {
        let Some(account) = self.account_for(&outgoing.draft) else {
            return false;
        };
        self.send_command(account, Command::Send(Box::new(outgoing)));
        true
    }

    pub fn finish(&mut self) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        for worker in self.workers.drain(..) {
            worker.finish(deadline);
        }
    }

    pub fn load_remote(&mut self) {
        let (Some(id), Some(reading)) = (self.selected, self.reading.as_mut()) else {
            return;
        };
        if reading.remote != Remote::Blocked {
            return;
        }
        reading.remote = Remote::Loading;
        self.layout = None;
        let sources = reading.remote_sources();
        let network = self.network.clone();
        let notifier = self.mailroom.notifier();
        let spawned = std::thread::Builder::new()
            .name("remote-content".to_owned())
            .spawn(move || {
                let pictures = sources
                    .into_iter()
                    .filter_map(|source| {
                        let bytes = crate::http::get(&source, &network, MAX_PICTURE_BYTES)
                            .inspect_err(|error| tracing::debug!("cannot load {source}: {error}"))
                            .ok()?;
                        crate::picture::decode(&bytes).map(|picture| (source, picture))
                    })
                    .collect();
                notifier.send(MailEvent::Pictures(id, pictures));
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot load remote content: {error}");
        }
    }

    pub fn open_attachment(&mut self, index: usize) {
        let Some(attachment) = self
            .reading
            .as_ref()
            .and_then(|reading| reading.content.attachments.get(index))
        else {
            return;
        };
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let user_dirs = bande_xdg::config_home()
            .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
        let folder =
            bande_xdg::user_dir(&home, user_dirs.as_deref(), bande_xdg::UserDir::Downloads);
        let name = attachment
            .name
            .replace(['/', '\\', '\0'], "_")
            .trim_start_matches('.')
            .to_owned();
        let name = match name.is_empty() {
            true => "attachment".to_owned(),
            false => name,
        };
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => {
                (stem.to_owned(), format!(".{extension}"))
            }
            _ => (name.clone(), String::new()),
        };
        let saved = std::fs::create_dir_all(&folder).and_then(|()| {
            for copy in 0..1000 {
                let path = match copy {
                    0 => folder.join(&name),
                    copy => folder.join(format!("{stem} {}{extension}", copy + 1)),
                };
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(mut file) => return file.write_all(&attachment.data).map(|()| path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(std::io::Error::other("no free file name"))
        });
        match saved {
            Ok(path) => self.requests.push(Request::OpenAttachment(path)),
            Err(error) => tracing::warn!("cannot save {}: {error}", attachment.name),
        }
    }

    pub fn engine_ready(&mut self) -> bool {
        let events = self.mailroom.take();
        let mut changed = false;
        let mut stale = false;
        for event in events {
            changed = true;
            match event {
                MailEvent::Status(account, status) => {
                    self.statuses.retain(|(known, _)| *known != account);
                    self.statuses.push((account, status));
                }
                MailEvent::Changed(_) => stale = true,
                MailEvent::Arrived(_, ids) => {
                    stale = true;
                    self.notify(&ids);
                }
                MailEvent::Body(id) => {
                    if self.selected == Some(id)
                        && let Some(summary) = self.selected_summary()
                    {
                        self.load_reading(&summary);
                    }
                    stale = true;
                }
                MailEvent::LoginFailed(account, _) => {
                    if self.setup.is_none()
                        && let Some(account) = self.account(account)
                    {
                        let email = account.email.clone();
                        self.present_setup(Mode::Password(email));
                    }
                }
                MailEvent::Failed(account, error) => {
                    tracing::warn!("account {account}: {error}");
                }
                MailEvent::Sent(_) => {}
                MailEvent::NotSent(_, error, outgoing) => {
                    self.requests.push(Request::NotSent { error, outgoing });
                }
                MailEvent::Setup(outcome) => self.setup_checked(*outcome),
                MailEvent::Pictures(id, pictures) => {
                    if self.selected == Some(id)
                        && let Some(reading) = self.reading.as_mut()
                    {
                        for (source, picture) in pictures {
                            reading.add_picture(source, picture);
                        }
                        reading.remote = Remote::Loaded;
                        self.layout = None;
                    }
                }
            }
        }
        if stale {
            self.reload();
        }
        changed
    }

    fn notify(&mut self, ids: &[MessageId]) {
        if !self.settings.notify {
            return;
        }
        let sound = self.settings.sound;
        let arrived = ids
            .iter()
            .filter_map(|id| self.store.summary(*id).ok().flatten())
            .collect::<Vec<_>>();
        if arrived.len() > NOTIFIED_AT_ONCE {
            self.requests.push(Request::Notify {
                summary: L10N.tr_args("new-messages", [("count", arrived.len().into())]),
                body: arrived
                    .iter()
                    .take(NOTIFIED_AT_ONCE)
                    .map(|summary| summary.sender.shown().to_owned())
                    .collect::<Vec<_>>()
                    .join(", "),
                sound,
            });
            return;
        }
        for summary in arrived {
            self.requests.push(Request::Notify {
                summary: summary.sender.shown().to_owned(),
                body: match summary.subject.as_str() {
                    "" => L10N.tr("no-subject"),
                    subject => subject.to_owned(),
                },
                sound,
            });
        }
    }

    pub fn present_setup(&mut self, mode: Mode) {
        let state: Shared = Rc::new(RefCell::new(SheetState::default()));
        if let Some(previous) = &self.setup {
            let previous = previous.state.borrow();
            let mut fresh = state.borrow_mut();
            fresh.values = previous.values.clone();
            fresh.password = previous.password.clone();
        }
        if let Some(previous) = self.setup.take() {
            previous.state.borrow_mut().closed = true;
        }
        let sheet = AccountSheet::new(Rc::clone(&state), mode.clone());
        let password_field = sheet.password_field();
        self.ui.present_sheet(sheet);
        self.setup = Some(SetupSheet {
            state,
            mode,
            password_field,
        });
    }

    fn close_setup(&mut self) {
        if let Some(setup) = self.setup.take() {
            setup.state.borrow_mut().closed = true;
        }
    }

    fn submit_password(&mut self, field: WidgetId) {
        let previous = self.ui.focus();
        self.ui.set_focus(field);
        self.ui.handle(&Event::Key {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        match previous {
            Some(previous) => self.ui.set_focus(previous),
            None => self.ui.clear_focus(),
        }
    }

    fn sheet_action(&mut self) {
        let Some(setup) = &self.setup else {
            return;
        };
        let (state, mode, field) = (
            Rc::clone(&setup.state),
            setup.mode.clone(),
            setup.password_field,
        );
        let action = state.borrow_mut().action.take();
        match action {
            None => {}
            Some(Action::Cancel) => self.close_setup(),
            Some(Action::SignIn) => {
                if state.borrow().busy {
                    return;
                }
                self.submit_password(field);
                state.borrow_mut().action = None;
                self.sign_in(&state, &mode);
            }
        }
    }

    fn sign_in(&mut self, state: &Shared, mode: &Mode) {
        let password = state.borrow().password.clone();
        if let Mode::Password(email) = mode {
            if password.is_empty() {
                return;
            }
            let Some(account) = self
                .accounts
                .iter()
                .map(|(account, _)| account.clone())
                .find(|account| account.email == *email)
            else {
                return;
            };
            let host = account.imap.host.clone();
            let saved = password.clone();
            std::thread::spawn(move || {
                if let Err(error) = Keyring::session_bus()
                    .and_then(|keyring| keyring.save(&account.email, &host, &saved))
                {
                    tracing::warn!("cannot save the password: {error}");
                }
            });
            self.send_command(account.id, Command::Password(password.to_string()));
            self.close_setup();
            return;
        }
        let (name, email) = {
            let state = state.borrow();
            (state.value(Field::Name), state.value(Field::Email))
        };
        let servers = match mode {
            Mode::Manual => state.borrow().servers(),
            _ => None,
        };
        let problem = if name.is_empty() || !email.contains('@') {
            Some("setup-missing")
        } else if password.is_empty() {
            Some("setup-missing-password")
        } else if *mode == Mode::Manual && servers.is_none() {
            Some("setup-missing-servers")
        } else if self
            .accounts
            .iter()
            .any(|(account, _)| account.email.eq_ignore_ascii_case(&email))
        {
            Some("setup-exists")
        } else {
            None
        };
        if let Some(problem) = problem {
            state.borrow_mut().status = Some((L10N.tr(problem), true));
            return;
        }
        {
            let mut state = state.borrow_mut();
            state.busy = true;
            state.status = Some((L10N.tr("checking-account"), false));
        }
        let request = SetupRequest {
            name,
            email,
            password: password.to_string(),
            servers,
        };
        let network = self.network.clone();
        let notifier = self.mailroom.notifier();
        std::thread::spawn(move || {
            let outcome = check(&request, &network);
            if let Outcome::Ready(settings) = &outcome
                && let Err(error) = Keyring::session_bus().and_then(|keyring| {
                    keyring.save(&request.email, &settings.imap.host, &request.password)
                })
            {
                tracing::warn!("cannot save the password: {error}");
            }
            notifier.send(MailEvent::Setup(Box::new(outcome)));
        });
    }

    fn setup_checked(&mut self, outcome: Outcome) {
        let Some(setup) = &self.setup else {
            return;
        };
        let state = Rc::clone(&setup.state);
        state.borrow_mut().busy = false;
        match outcome {
            Outcome::Ready(settings) => self.add_account(&state, settings),
            Outcome::WrongPassword(settings, _) => {
                let mut state = state.borrow_mut();
                fill_servers(&mut state, &settings);
                state.status = Some((L10N.tr("setup-wrong-password"), true));
            }
            Outcome::NeedsServers(settings, reason) => {
                if let Some(settings) = &settings {
                    fill_servers(&mut state.borrow_mut(), settings);
                }
                let manual = setup.mode == Mode::Manual;
                if !manual {
                    self.present_setup(Mode::Manual);
                }
                if let Some(setup) = &self.setup {
                    setup.state.borrow_mut().status = Some((
                        L10N.tr_args("setup-no-servers", [("reason", reason.into())]),
                        true,
                    ));
                }
            }
        }
    }

    fn add_account(&mut self, state: &Shared, settings: Settings) {
        let (name, email, password) = {
            let state = state.borrow();
            (
                state.value(Field::Name),
                state.value(Field::Email),
                state.password.clone(),
            )
        };
        let account = Account {
            id: 0,
            name,
            email,
            user: settings.user,
            imap: settings.imap,
            smtp: settings.smtp,
            signature: String::new(),
        };
        let id = match self.store.add_account(&account) {
            Ok(id) => id,
            Err(error) => {
                state.borrow_mut().status = Some((error.to_string(), true));
                return;
            }
        };
        let account = Account { id, ..account };
        let first = Zeroizing::new(password.to_string());
        let mut handed = false;
        let fallback = keyring_password(account.email.clone(), account.imap.host.clone());
        let mut fallback = Some(fallback);
        let source: crate::worker::PasswordSource = Box::new(move || {
            if !handed {
                handed = true;
                return Ok(Some(first.to_string()));
            }
            match fallback.as_mut() {
                Some(fallback) => fallback(),
                None => Ok(None),
            }
        });
        self.workers.push(Worker::spawn(
            account,
            self.store_path.clone(),
            source,
            self.network.clone(),
            self.mailroom.notifier(),
        ));
        self.close_setup();
        self.reload();
    }

    pub fn remove_account(&mut self, id: AccountId) {
        let Some(account) = self.account(id).cloned() else {
            return;
        };
        self.workers.retain(|worker| worker.account != id);
        if let Err(error) = self.store.remove_account(id) {
            tracing::warn!("cannot remove {}: {error}", account.email);
        }
        std::thread::spawn(move || {
            if let Err(error) = Keyring::session_bus()
                .and_then(|keyring| keyring.forget(&account.email, &account.imap.host))
            {
                tracing::warn!("cannot forget the password: {error}");
            }
        });
        self.reload();
    }

    pub(crate) fn after_ui(&mut self) {
        self.sheet_action();
        let signals = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in signals {
            match signal {
                Signal::GetMail => self.get_mail(),
                Signal::Compose => self.compose(Draft::default()),
                Signal::Archive => self.move_selected(Role::Archive),
                Signal::Delete => self.delete_selected(),
                Signal::Junk => self.move_selected(Role::Junk),
                Signal::Reply => self.respond(false),
                Signal::ReplyAll => self.respond(true),
                Signal::Forward => self.forward_selected(),
                Signal::Flag => self.toggle_flag(),
                Signal::Search(text) => self.search_for(&text),
            }
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(Request::Clipboard(request));
        }
        if let Some((menu, at)) = self.ui.take_menu_request() {
            self.requests.push(Request::ShowMenu(menu, at));
        }
        self.refresh_toolbar();
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let handled = self.route(event);
        self.after_ui();
        handled
    }

    fn route(&mut self, event: &Event) -> bool {
        if self.ui.has_sheet() {
            return self.ui.handle(event);
        }
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::PointerDown(pointer) => {
                if self.over_control(pointer.position) {
                    self.pressed_ui = true;
                    return self.ui.handle(event);
                }
                self.pressed_ui = false;
                self.ui.clear_focus();
                self.click(pointer)
            }
            Event::PointerUp(_) => {
                let handled = self.ui.handle(event);
                self.pressed_ui = false;
                handled
            }
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                let ui = self.ui.handle(event);
                let hover = self.hit(pointer.position);
                let changed = hover != self.hover;
                self.hover = hover;
                ui | changed
            }
            Event::PointerLeave => self.ui.handle(event) | self.hover.take().is_some(),
            Event::Scroll(scroll) => {
                self.ui.handle(event)
                    || (!self.over_control(scroll.position) && self.scroll(scroll))
            }
            _ => self.ui.handle(event),
        }
    }

    fn search_focused(&self) -> bool {
        self.ui.focus() == Some(self.toolbar.search)
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if modifiers.logo {
            return false;
        }
        if self.search_focused() {
            if key == Key::Escape {
                self.ui.clear_focus();
                return true;
            }
            return self.ui.handle(&Event::Key { key, modifiers });
        }
        let event = Event::Key { key, modifiers };
        if let Some((action, _)) = SHORTCUTS
            .iter()
            .find(|(_, shortcut)| shortcut_pressed(&event, shortcut))
        {
            return self.menu_action(action);
        }
        let plain = !modifiers.control;
        match key {
            Key::Up if plain => self.step(-1),
            Key::Down if plain => self.step(1),
            Key::Backspace | Key::Delete => {
                self.delete_selected();
                true
            }
            Key::Space if plain => self.page_reader(),
            Key::Enter if plain => {
                self.continue_draft();
                true
            }
            _ => self.ui.handle(&event),
        }
    }

    fn step(&mut self, by: isize) -> bool {
        if self.messages.is_empty() {
            return false;
        }
        let current = self
            .selected
            .and_then(|id| self.messages.iter().position(|summary| summary.id == id));
        let next = match current {
            Some(current) => current
                .saturating_add_signed(by)
                .min(self.messages.len() - 1),
            None => 0,
        };
        let id = self.messages[next].id;
        if Some(id) == self.selected {
            return false;
        }
        self.select(id);
        true
    }

    fn click(&mut self, pointer: &Pointer) -> bool {
        let Some(hit) = self.hit(pointer.position) else {
            return false;
        };
        let secondary = pointer.button == PointerButton::Secondary
            || (pointer.button == PointerButton::Primary && pointer.modifiers.control);
        match hit {
            Hit::Place(index) => {
                self.focus = Pane::Sidebar;
                if let Some(Row::Place { place, .. }) = self.sidebar.get(index).cloned() {
                    self.choose_place(place);
                }
            }
            Hit::Message(id) => {
                if self.selected != Some(id) {
                    self.select(id);
                }
                if pointer.count >= 2 && !secondary {
                    self.continue_draft();
                }
                if secondary {
                    let menu = self.message_menu();
                    self.requests
                        .push(Request::ShowMenu(menu, pointer.position));
                }
            }
            Hit::Link(url) => self.requests.push(Request::OpenLink(url)),
            Hit::Attachment(index) => self.open_attachment(index),
            Hit::LoadRemote => self.load_remote(),
        }
        true
    }

    pub fn message_menu(&self) -> Menu {
        let summary = self.selected_summary();
        let read = summary.as_ref().is_some_and(|summary| summary.flags.seen);
        let flagged = summary
            .as_ref()
            .is_some_and(|summary| summary.flags.flagged);
        Menu::new(vec![
            MenuItem::new(&L10N.tr("action-reply"), "reply"),
            MenuItem::new(&L10N.tr("action-reply-all"), "reply-all"),
            MenuItem::new(&L10N.tr("action-forward"), "forward"),
            MenuItem::separator(),
            MenuItem::new(
                &L10N.tr(if read {
                    "action-mark-unread"
                } else {
                    "action-mark-read"
                }),
                "toggle-read",
            ),
            MenuItem::new(
                &L10N.tr(if flagged {
                    "action-unflag"
                } else {
                    "action-flag"
                }),
                "toggle-flag",
            ),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("action-archive"), "archive")
                .enabled(self.can_move_to(Role::Archive)),
            MenuItem::new(&L10N.tr("action-junk"), "junk").enabled(self.can_move_to(Role::Junk)),
            MenuItem::new(&L10N.tr("action-delete"), "delete"),
        ])
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        match action {
            "reply" => self.respond(false),
            "reply-all" => self.respond(true),
            "forward" => self.forward_selected(),
            "toggle-read" => self.toggle_read(),
            "toggle-flag" => self.toggle_flag(),
            "archive" => self.move_selected(Role::Archive),
            "junk" => self.move_selected(Role::Junk),
            "delete" => self.delete_selected(),
            "compose" => self.compose(Draft::default()),
            "get-mail" => self.get_mail(),
            "add-account" => self.present_setup(Mode::Add),
            "find" => self.ui.set_focus(self.toolbar.search),
            action => {
                let handled = self.ui.menu_action(action);
                self.after_ui();
                return handled;
            }
        }
        self.after_ui();
        true
    }

    pub fn hit(&self, at: Point) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, hit)| hit.clone())
    }
}

fn fill_servers(state: &mut SheetState, settings: &Settings) {
    for (field, value) in [
        (Field::Incoming, settings.imap.host.clone()),
        (Field::IncomingPort, settings.imap.port.to_string()),
        (Field::Outgoing, settings.smtp.host.clone()),
        (Field::OutgoingPort, settings.smtp.port.to_string()),
        (Field::User, settings.user.clone()),
    ] {
        match state.values.iter_mut().find(|(known, _)| *known == field) {
            Some((_, known)) => *known = value,
            None => state.values.push((field, value)),
        }
    }
}
