use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{ActionRequest, TreeUpdate};
use bande_toolkit::ui::{
    Alert, ClipboardRequest, Context, Cursor, Event, FreeLayout, Key, Menu, MenuItem, Modifiers,
    Pointer, PointerButton, ScrollState, TextField, Ui, Widget, WidgetId, shortcut_pressed,
};
use bande_toolkit::{
    Appearance, Header, Palette, Point, Rect, SHEET_INSET, TextSystem, WINDOW_HEADER_HEIGHT,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use zeroize::Zeroizing;

use crate::L10N;
use crate::compose::{Draft, Original, Outgoing, forward, reply, signed};
use crate::discover::Settings;
use crate::error::Result;
use crate::imap::{Flag, Role};
use crate::message::{Content, Envelope};
use crate::net::Network;
use crate::secrets::Keyring;
use crate::settings::{MailSettings, SortBy};
use crate::setup::{Outcome, Request as SetupRequest, check};
use crate::store::{Account, AccountId, Mailbox, MailboxId, MessageId, RuleAction, Store, Summary};
use crate::ui::format::full_date;
use crate::ui::reader::{Entry, Layout, Reading, Remote, Respond, Target, Thread};
use crate::ui::setup_sheet::{AccountSheet, Action, Field, Mode, Shared, SheetState};
use crate::ui::sidebar::{Place, Row, rows};
use crate::ui::symbols::Symbols;
use crate::ui::toolbar::{Signal, Signals, Toolbar};
use crate::worker::{Command, Event as MailEvent, MailboxChange, Mailroom, Status, Worker};
use bande_app_settings::{Stored, settings_folder};

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
const UNDO_TRIES: u8 = 3;
const MOVED_IN_KEPT: Duration = Duration::from_secs(10 * 60);
const UNDO_LEVELS: usize = 20;
const DRAG_THRESHOLD: f32 = 6.0;
const MAX_PICTURE_BYTES: usize = 10 << 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Compose(Box<Outgoing>),
    OpenLink(String),
    OpenAttachment(PathBuf),
    SaveAttachment(String, Vec<u8>),
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
    Conversation(MessageId),
    Reader(Target),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Sidebar,
    List,
}

#[derive(Debug, Default)]
pub(crate) struct Conversations {
    pub(crate) earlier: HashMap<MessageId, (String, Vec<Summary>)>,
    pub(crate) children: HashSet<MessageId>,
    expanded: HashSet<String>,
}

impl Conversations {
    fn group(&mut self, all: Vec<Summary>) -> Vec<Summary> {
        self.earlier.clear();
        self.children.clear();
        let mut order: Vec<Vec<Summary>> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for summary in all {
            match summary
                .thread
                .clone()
                .and_then(|key| index.get(&key).copied())
            {
                Some(group) => order[group].push(summary),
                None => {
                    if let Some(key) = summary.thread.clone() {
                        index.insert(key, order.len());
                    }
                    order.push(vec![summary]);
                }
            }
        }
        let mut shown = Vec::new();
        for mut group in order {
            let head = group.remove(0);
            let key = head.thread.clone().unwrap_or_default();
            if !group.is_empty() {
                if self.expanded.contains(&key) {
                    self.children.extend(group.iter().map(|summary| summary.id));
                    shown.push(head.clone());
                    shown.extend(group.iter().cloned());
                } else {
                    shown.push(head.clone());
                }
                self.earlier.insert(head.id, (key, group));
            } else {
                shown.push(head);
            }
        }
        shown
    }

    pub(crate) fn collapsed(&self, id: MessageId) -> Option<&[Summary]> {
        self.earlier
            .get(&id)
            .filter(|(key, _)| !self.expanded.contains(key))
            .map(|(_, earlier)| earlier.as_slice())
    }

    pub(crate) fn count(&self, id: MessageId) -> Option<usize> {
        self.earlier.get(&id).map(|(_, earlier)| earlier.len() + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drag {
    pub(crate) from: Point,
    pub(crate) at: Point,
    pub(crate) moving: bool,
    pub(crate) over: Option<usize>,
    collapse: Option<MessageId>,
}

#[derive(Debug, Clone)]
struct Moved {
    message_id: String,
    from: Mailbox,
    to: MailboxId,
    tries: u8,
}

pub(crate) struct SetupSheet {
    pub(crate) state: Shared,
    pub(crate) mode: Mode,
    pub(crate) password_field: WidgetId,
}

fn open_files() -> HashSet<PathBuf> {
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return HashSet::new();
    };
    processes
        .filter_map(|entry| entry.ok())
        .filter(|process| {
            process
                .file_name()
                .to_str()
                .is_some_and(|name| name.parse::<u32>().is_ok())
        })
        .filter_map(|process| std::fs::read_dir(process.path().join("fd")).ok())
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|fd| std::fs::read_link(fd.path()).ok())
        .collect()
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
    pub(crate) also: Vec<MessageId>,
    pub(crate) thread: Option<Thread>,
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
    pub(crate) settings: MailSettings,
    opened: Vec<((PathBuf, usize), PathBuf, std::time::SystemTime)>,
    held: Vec<(AccountId, Command)>,
    undo: Vec<Vec<Moved>>,
    undoing: Vec<Moved>,
    moved_in: HashMap<(MailboxId, String), Instant>,
    pub(crate) drag: Option<Drag>,
    pub(crate) conversations: Conversations,
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
            also: Vec::new(),
            thread: None,
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
            settings,
            opened: Vec::new(),
            held: Vec::new(),
            undo: Vec::new(),
            undoing: Vec::new(),
            moved_in: HashMap::new(),
            drag: None,
            conversations: Conversations::default(),
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

    pub fn settings(&self) -> &MailSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: MailSettings) {
        let regroup = settings.conversations != self.settings.conversations;
        let reread = (settings.plain_text, settings.fixed_width)
            != (self.settings.plain_text, self.settings.fixed_width);
        self.settings = settings;
        self.layout = None;
        if regroup {
            self.reload();
        }
        if reread {
            let expanded = self
                .thread
                .iter()
                .flat_map(|thread| &thread.entries)
                .filter(|entry| entry.reading.is_some())
                .map(|entry| entry.summary.id)
                .collect::<Vec<_>>();
            for id in expanded {
                self.read_entry(id);
            }
        }
    }

    fn keep_settings(&mut self) {
        if let Some(folder) = settings_folder()
            && let Err(error) = self.settings.save(&folder)
        {
            tracing::warn!("cannot save the settings: {error}");
        }
        self.reload();
    }

    pub fn toggle_conversation(&mut self, id: MessageId) -> bool {
        let head = self
            .conversations
            .earlier
            .iter()
            .find(|(head, (_, earlier))| {
                **head == id || earlier.iter().any(|summary| summary.id == id)
            })
            .map(|(head, (key, _))| (*head, key.clone()));
        let Some((head, key)) = head else {
            return false;
        };
        if !self.conversations.expanded.remove(&key) {
            self.conversations.expanded.insert(key);
        } else if self.selected != Some(head) && self.is_chosen(id) {
            self.select(head);
        }
        self.reload();
        true
    }

    pub fn accounts_changed(&mut self) {
        let stored = self
            .store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|account| account.enabled)
            .collect::<Vec<_>>();
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
        self.release_held();
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
        match &self.hover {
            Some(Hit::Conversation(_)) => Cursor::Pointer,
            Some(Hit::Reader(target)) if !matches!(target, Target::Card(_)) => Cursor::Pointer,
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
            .filter(|account| account.enabled)
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
        let mut all = self
            .store
            .summaries(&self.place.scope(), &self.search)
            .unwrap_or_default();
        match self.settings.sort {
            SortBy::Date => {}
            SortBy::From => all.sort_by_cached_key(|summary| summary.sender.shown().to_lowercase()),
            SortBy::Subject => all.sort_by_cached_key(|summary| summary.subject.to_lowercase()),
        }
        if self.settings.ascending {
            all.reverse();
        }
        self.messages = match self.settings.conversations {
            true => self.conversations.group(all),
            false => {
                self.conversations = Conversations {
                    expanded: std::mem::take(&mut self.conversations.expanded),
                    ..Conversations::default()
                };
                all
            }
        };
        let messages = &self.messages;
        self.also
            .retain(|id| messages.iter().any(|summary| summary.id == *id));
        if self
            .selected
            .is_some_and(|id| !self.messages.iter().any(|summary| summary.id == id))
        {
            self.selected = None;
            self.also.clear();
            self.thread = None;
            self.layout = None;
        }
        self.refresh_thread();
    }

    fn conversation_of(&self, summary: &Summary) -> Vec<Summary> {
        match self.settings.conversations {
            true => self
                .store
                .conversation(summary)
                .unwrap_or_else(|_| vec![summary.clone()]),
            false => vec![summary.clone()],
        }
    }

    fn refresh_thread(&mut self) {
        let Some(thread) = &self.thread else {
            return;
        };
        let Some(focused) = thread
            .entry(thread.focus)
            .map(|entry| entry.summary.clone())
        else {
            return;
        };
        let messages = self.conversation_of(&focused);
        let Some(thread) = self.thread.as_mut() else {
            return;
        };
        let same = messages.len() == thread.entries.len()
            && messages
                .iter()
                .zip(&thread.entries)
                .all(|(summary, entry)| summary.id == entry.summary.id);
        if same {
            for (summary, entry) in messages.into_iter().zip(&mut thread.entries) {
                entry.summary = summary;
            }
            return;
        }
        let mut old = std::mem::take(&mut thread.entries);
        thread.entries = messages
            .into_iter()
            .map(
                |summary| match old.iter().position(|entry| entry.summary.id == summary.id) {
                    Some(index) => Entry {
                        summary,
                        ..old.swap_remove(index)
                    },
                    None => Entry {
                        summary,
                        reading: None,
                        quotes: false,
                    },
                },
            )
            .collect();
        if thread.entry(thread.focus).is_none()
            && let Some(last) = thread.entries.last()
        {
            thread.focus = last.summary.id;
        }
        self.layout = None;
    }

    pub fn choose_place(&mut self, place: Place) {
        if place == self.place {
            return;
        }
        self.place = place;
        self.selected = None;
        self.also.clear();
        self.thread = None;
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
        self.also.clear();
        self.focus = Pane::List;
        self.scrolls[2] = ScrollState::default();
        let unseen = self.open_thread(&summary);
        if !unseen.is_empty() {
            self.set_flag_on(&unseen, Flag::Seen, true);
        }
        self.reveal_selected();
    }

    fn select_more(&mut self, id: MessageId, range: bool) {
        let Some(anchor) = self.selected else {
            return self.select(id);
        };
        let position = |id: MessageId| self.messages.iter().position(|summary| summary.id == id);
        if range {
            let (Some(from), Some(to)) = (position(anchor), position(id)) else {
                return;
            };
            self.also = self.messages[from.min(to)..=from.max(to)]
                .iter()
                .map(|summary| summary.id)
                .filter(|known| *known != anchor)
                .collect();
        } else if id == anchor {
            if self.also.is_empty() {
                return;
            }
            self.selected = Some(self.also.remove(0));
        } else if let Some(index) = self.also.iter().position(|known| *known == id) {
            self.also.remove(index);
        } else {
            self.also.push(id);
        }
        self.focus = Pane::List;
        match (self.also.is_empty(), self.selected_summary()) {
            (true, Some(summary)) => {
                self.scrolls[2] = ScrollState::default();
                self.open_thread(&summary);
            }
            _ => {
                self.thread = None;
                self.layout = None;
            }
        }
    }

    pub(crate) fn is_chosen(&self, id: MessageId) -> bool {
        self.selected == Some(id) || self.also.contains(&id)
    }

    pub(crate) fn chosen(&self) -> Vec<Summary> {
        self.messages
            .iter()
            .filter(|summary| self.is_chosen(summary.id))
            .flat_map(|summary| {
                std::iter::once(summary.clone()).chain(
                    self.conversations
                        .collapsed(summary.id)
                        .unwrap_or_default()
                        .iter()
                        .cloned(),
                )
            })
            .collect()
    }

    fn reading_of(&self, summary: &Summary) -> Reading {
        let body = self.store.body(summary.id).ok().flatten();
        match body {
            Some(raw) => Reading::new(Content::parse(&raw), true, &self.settings),
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
                    &self.settings,
                )
            }
        }
    }

    fn open_thread(&mut self, summary: &Summary) -> Vec<MessageId> {
        let messages = self.conversation_of(summary);
        let shown = messages
            .iter()
            .filter(|message| message.id == summary.id || !message.flags.seen)
            .map(|message| (message.id, message.flags.seen))
            .collect::<Vec<_>>();
        self.thread = Some(Thread {
            entries: messages
                .into_iter()
                .map(|summary| Entry {
                    summary,
                    reading: None,
                    quotes: false,
                })
                .collect(),
            focus: summary.id,
            reveal: true,
        });
        for (id, _) in &shown {
            self.read_entry(*id);
        }
        shown
            .into_iter()
            .filter(|(_, seen)| !seen)
            .map(|(id, _)| id)
            .collect()
    }

    fn read_entry(&mut self, id: MessageId) {
        let Some(summary) = self
            .thread
            .as_ref()
            .and_then(|thread| thread.entry(id))
            .map(|entry| entry.summary.clone())
        else {
            return;
        };
        let reading = self.reading_of(&summary);
        let remote = reading.downloaded
            && reading.document.remote_images > 0
            && (self.settings.load_remote
                || self
                    .store
                    .trusts_remote(&reading.content.envelope.sender().email)
                    .unwrap_or(false));
        if let Some(entry) = self.thread.as_mut().and_then(|thread| thread.entry_mut(id)) {
            entry.reading = Some(reading);
        }
        self.layout = None;
        if remote {
            self.load_remote(id);
        }
    }

    fn focus_entry(&mut self, id: MessageId) {
        if let Some(thread) = self.thread.as_mut()
            && thread.focus != id
        {
            thread.focus = id;
            self.layout = None;
        }
    }

    fn toggle_entry(&mut self, id: MessageId) {
        let Some(thread) = self.thread.as_mut() else {
            return;
        };
        let Some(entry) = thread.entry_mut(id) else {
            return;
        };
        if entry.reading.take().is_none() {
            let unseen = !entry.summary.flags.seen;
            self.focus_entry(id);
            self.read_entry(id);
            if unseen {
                self.set_flag_on(&[id], Flag::Seen, true);
            }
            return;
        }
        if thread.focus == id
            && let Some(open) = thread
                .entries
                .iter()
                .rev()
                .find(|entry| entry.reading.is_some())
        {
            thread.focus = open.summary.id;
        }
        self.layout = None;
    }

    fn focused(&self) -> Option<(&Summary, &Reading)> {
        self.thread.as_ref().and_then(Thread::focused)
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
        let earlier = self
            .conversations
            .earlier
            .values_mut()
            .flat_map(|(_, earlier)| earlier.iter_mut());
        for summary in self
            .messages
            .iter_mut()
            .chain(earlier)
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

    fn toggle(&mut self, flag: Flag, is_set: fn(&Summary) -> bool) {
        let chosen = self.chosen();
        let Some(first) = self.selected_summary().or_else(|| chosen.first().cloned()) else {
            return;
        };
        let ids = chosen.iter().map(|summary| summary.id).collect::<Vec<_>>();
        self.set_flag_on(&ids, flag, !is_set(&first));
    }

    pub fn toggle_flag(&mut self) {
        self.toggle(Flag::Flagged, |summary| summary.flags.flagged);
    }

    pub fn mark_all_read(&mut self) {
        let unread = self
            .store
            .summaries(&self.place.scope(), "")
            .unwrap_or_default()
            .into_iter()
            .filter(|summary| !summary.flags.seen)
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        if !unread.is_empty() {
            self.set_flag_on(&unread, Flag::Seen, true);
        }
    }

    pub fn toggle_read(&mut self) {
        self.toggle(Flag::Seen, |summary| summary.flags.seen);
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
        self.chosen().iter().any(|summary| {
            self.mailbox_with_role(summary.account, role)
                .is_some_and(|target| target.id != summary.mailbox)
        })
    }

    fn leave_selected(&mut self, ids: &[MessageId]) {
        if ids.is_empty() {
            return;
        }
        let position = self
            .messages
            .iter()
            .position(|summary| ids.contains(&summary.id));
        self.messages.retain(|summary| !ids.contains(&summary.id));
        self.selected = None;
        self.also.clear();
        self.thread = None;
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

    fn move_each(&mut self, moves: Vec<(MessageId, Mailbox)>) -> Vec<MessageId> {
        let mut moved = Vec::new();
        let mut done = Vec::new();
        let mut undo = Vec::new();
        for (id, target) in &moves {
            if let Ok(Some(message_id)) = self.store.message_id(*id) {
                self.expect_arrival(target, message_id);
            }
            if let (Some(from), Ok(Some(message_id))) = (
                self.store
                    .summary(*id)
                    .ok()
                    .flatten()
                    .and_then(|summary| self.mailbox_of(&summary)),
                self.store.message_id(*id),
            ) {
                undo.push(Moved {
                    message_id,
                    from,
                    to: target.id,
                    tries: 0,
                });
            }
        }
        if !undo.is_empty() {
            if self.undo.len() == UNDO_LEVELS {
                self.undo.remove(0);
            }
            self.undo.push(undo);
        }
        for (_, target) in &moves {
            if done.contains(&target.id) {
                continue;
            }
            done.push(target.id);
            let ids = moves
                .iter()
                .filter(|(_, known)| known.id == target.id)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            match self.store.move_messages(&ids, target) {
                Ok(accounts) => {
                    self.flush(&accounts);
                    moved.extend(ids);
                }
                Err(error) => tracing::warn!("cannot move the messages: {error}"),
            }
        }
        moved
    }

    pub fn move_selected(&mut self, role: Role) {
        let moves = self
            .chosen()
            .into_iter()
            .filter_map(|summary| {
                let target = self.mailbox_with_role(summary.account, role)?;
                (target.id != summary.mailbox).then_some((summary.id, target))
            })
            .collect();
        let moved = self.move_each(moves);
        self.leave_selected(&moved);
    }

    pub fn move_targets(&self) -> Vec<(String, MailboxId)> {
        let Some(summary) = self.selected_summary() else {
            return Vec::new();
        };
        self.accounts
            .iter()
            .filter(|(account, _)| account.id == summary.account)
            .flat_map(|(_, mailboxes)| mailboxes)
            .filter(|mailbox| mailbox.selectable && mailbox.id != summary.mailbox)
            .map(|mailbox| {
                let name = match mailbox.delimiter {
                    Some(delimiter) => mailbox.path.replace(delimiter, " / "),
                    None => mailbox.path.clone(),
                };
                let name = match mailbox.role {
                    Some(Role::Inbox) => L10N.tr("mailbox-inbox"),
                    _ => name,
                };
                (name, mailbox.id)
            })
            .collect()
    }

    fn move_selected_to(&mut self, target: &Mailbox) {
        let moves = self
            .chosen()
            .into_iter()
            .filter(|summary| target.id != summary.mailbox && target.account == summary.account)
            .map(|summary| (summary.id, target.clone()))
            .collect();
        let moved = self.move_each(moves);
        self.leave_selected(&moved);
    }

    pub fn undo(&mut self) {
        let moves = self.undo.pop().unwrap_or_default();
        self.put_back(moves);
    }

    fn expect_arrival(&mut self, mailbox: &Mailbox, message_id: String) {
        let now = Instant::now();
        self.moved_in
            .retain(|_, since| now.duration_since(*since) < MOVED_IN_KEPT);
        if mailbox.role == Some(Role::Inbox) {
            self.moved_in.insert((mailbox.id, message_id), now);
        }
    }

    fn expected_arrival(&mut self, id: MessageId) -> bool {
        let (Ok(Some(summary)), Ok(Some(message_id))) =
            (self.store.summary(id), self.store.message_id(id))
        else {
            return false;
        };
        self.moved_in
            .remove(&(summary.mailbox, message_id))
            .is_some_and(|since| since.elapsed() < MOVED_IN_KEPT)
    }

    fn put_back(&mut self, moves: Vec<Moved>) {
        let mut waiting = Vec::new();
        for moved in moves {
            match self.store.find_message(moved.to, &moved.message_id) {
                Ok(Some(id)) => {
                    let from = match self.store.mailbox(moved.from.id) {
                        Ok(Some(from)) => from,
                        Ok(None) => continue,
                        Err(error) => {
                            tracing::warn!("cannot put the message back: {error}");
                            continue;
                        }
                    };
                    self.expect_arrival(&from, moved.message_id.clone());
                    match self.store.move_messages(&[id], &from) {
                        Ok(accounts) => self.flush(&accounts),
                        Err(error) => tracing::warn!("cannot put the message back: {error}"),
                    }
                }
                Ok(None) if moved.tries < UNDO_TRIES => {
                    if moved.tries == 0 && !waiting.contains(&moved.to) {
                        waiting.push(moved.to);
                        self.send_command(moved.from.account, Command::Refresh(moved.to));
                    }
                    self.undoing.push(Moved {
                        tries: moved.tries + 1,
                        ..moved
                    });
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot put the message back: {error}"),
            }
        }
        self.reload();
    }

    pub fn delete_selected(&mut self) {
        let mut moves = Vec::new();
        let mut expunged = Vec::new();
        for summary in self.chosen() {
            let in_trash = self
                .mailbox_of(&summary)
                .is_some_and(|mailbox| mailbox.role == Some(Role::Trash));
            match self.mailbox_with_role(summary.account, Role::Trash) {
                Some(trash) if !in_trash => moves.push((summary.id, trash)),
                _ => expunged.push(summary.id),
            }
        }
        let mut gone = self.move_each(moves);
        if !expunged.is_empty() {
            match self.store.expunge(&expunged) {
                Ok(accounts) => {
                    self.flush(&accounts);
                    gone.extend(expunged);
                }
                Err(error) => tracing::warn!("cannot delete the messages: {error}"),
            }
        }
        self.leave_selected(&gone);
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
            unsent: None,
        })));
    }

    pub fn respond(&mut self, all: bool) {
        let Some((summary, reading)) = self.focused() else {
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
            answering: Some(Original {
                id: summary.id,
                message_id: envelope.message_id.clone().unwrap_or_default(),
            }),
            replaces: None,
            unsent: None,
        })));
    }

    pub fn continue_draft(&mut self) {
        let Some((summary, reading)) = self.focused() else {
            return;
        };
        let is_draft = summary.flags.draft
            || self
                .mailbox_of(summary)
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
            bcc: envelope.bcc.clone(),
            subject: envelope.subject.clone(),
            body: reading.content.text().replace("\r\n", "\n"),
            in_reply_to: envelope.references.last().cloned(),
            references: envelope.references.clone(),
            attachments: reading.content.attachments.clone(),
        };
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: envelope.references.last().and_then(|parent| {
                let id = self.store.locate(summary.account, parent, None).ok()??;
                Some(Original {
                    id,
                    message_id: parent.clone(),
                })
            }),
            replaces: Some(Original {
                id: summary.id,
                message_id: envelope.message_id.clone().unwrap_or_default(),
            }),
            unsent: None,
        })));
    }

    pub fn forward_selected(&mut self) {
        let Some((summary, reading)) = self.focused() else {
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
            unsent: None,
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

    pub fn recover(&self, mut outgoing: Outgoing) -> Outgoing {
        let account = self.account_for(&outgoing.draft);
        let located = |original: Option<Original>, role: Option<Role>| {
            let original = original?;
            if original.message_id.is_empty() {
                return None;
            }
            if self.store.message_id(original.id).ok().flatten().as_ref()
                == Some(&original.message_id)
            {
                return Some(original);
            }
            let id = self
                .store
                .locate(account?, &original.message_id, role)
                .ok()
                .flatten()?;
            Some(Original { id, ..original })
        };
        outgoing.answering = located(outgoing.answering.take(), None);
        outgoing.replaces = located(outgoing.replaces.take(), Some(Role::Drafts));
        outgoing
    }

    pub fn save_draft(&mut self, outgoing: Outgoing) -> bool {
        self.dispatch(Command::SaveDraft(Box::new(outgoing)))
    }

    pub fn send(&mut self, outgoing: Outgoing) -> bool {
        self.dispatch(Command::Send(Box::new(outgoing)))
    }

    fn dispatch(&mut self, command: Command) -> bool {
        let (Command::Send(outgoing) | Command::SaveDraft(outgoing)) = &command else {
            return false;
        };
        if let Some(account) = self.account_for(&outgoing.draft) {
            self.send_command(account, command);
            return true;
        }
        let email = outgoing.draft.from.email.clone();
        let Some(account) = self
            .store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .find(|account| account.email.eq_ignore_ascii_case(&email))
        else {
            return false;
        };
        self.held.push((account.id, command));
        self.tell_held(&[account.id]);
        true
    }

    fn release_held(&mut self) {
        let stored = self.store.accounts().unwrap_or_default();
        for (account, command) in std::mem::take(&mut self.held) {
            if self.worker(account).is_some() {
                self.send_command(account, command);
            } else if stored.iter().any(|known| known.id == account) {
                self.held.push((account, command));
            } else if let Command::Send(outgoing) | Command::SaveDraft(outgoing) = command {
                self.requests.push(Request::NotSent {
                    error: L10N.tr("account-gone"),
                    outgoing,
                });
            }
        }
    }

    fn tell_held(&mut self, accounts: &[AccountId]) {
        let stored = self.store.accounts().unwrap_or_default();
        for account in stored.iter().filter(|known| accounts.contains(&known.id)) {
            let count = self.held.iter().filter(|(id, _)| *id == account.id).count();
            self.ui.present_sheet(
                Alert::new(
                    &L10N.tr_args("held-title", [("count", count.into())]),
                    &L10N.tr_args("held-message", [("email", account.email.clone().into())]),
                    |_| {},
                )
                .primary(&L10N.tr("button-ok"), "ok"),
            );
        }
    }

    pub fn finish(&mut self) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        for worker in self.workers.drain(..) {
            worker.finish(deadline);
        }
        self.remove_unedited_attachments();
    }

    pub fn load_remote(&mut self, id: MessageId) {
        let Some(reading) = self
            .thread
            .as_mut()
            .and_then(|thread| thread.entry_mut(id))
            .and_then(|entry| entry.reading.as_mut())
        else {
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

    fn attachment_name(&self, index: usize) -> Option<String> {
        let attachment = self.focused()?.1.content.attachments.get(index)?;
        let name = attachment
            .name
            .replace(['/', '\\', '\0'], "_")
            .trim_start_matches('.')
            .to_owned();
        Some(match name.is_empty() {
            true => "attachment".to_owned(),
            false => name,
        })
    }

    fn attachment_data(&self, index: usize) -> Option<Vec<u8>> {
        self.focused()
            .and_then(|(_, reading)| reading.content.attachments.get(index))
            .map(|attachment| attachment.data.clone())
    }

    pub fn open_attachment(&mut self, index: usize) {
        let (Some(summary), Some(name), Some(data)) = (
            self.focused().map(|(summary, _)| summary.clone()),
            self.attachment_name(index),
            self.attachment_data(index),
        ) else {
            return;
        };
        let Some(folder) = bande_xdg::cache_home().map(|cache| {
            cache
                .join("bande-mail/attachments")
                .join(format!("{}-{}", summary.mailbox, summary.uid))
        }) else {
            return;
        };
        let key = (folder.clone(), index);
        let ours = self
            .opened
            .iter()
            .find(|(known, ..)| *known == key)
            .map(|(_, path, _)| path.clone());
        let path = folder.join(&name);
        let kept = ours.or_else(|| {
            std::fs::read(&path)
                .is_ok_and(|kept| kept == data)
                .then_some(path)
        });
        if let Some(path) = kept {
            self.requests.push(Request::OpenAttachment(path));
            return;
        }
        let written = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&folder)
            .and_then(|()| write_new(&folder, &name, &data));
        match written {
            Ok(path) => {
                if let Ok(modified) =
                    std::fs::metadata(&path).and_then(|metadata| metadata.modified())
                {
                    self.opened.push((key, path.clone(), modified));
                }
                self.requests.push(Request::OpenAttachment(path));
            }
            Err(error) => tracing::warn!("cannot open {name}: {error}"),
        }
    }

    pub fn save_attachment(&mut self, index: usize) {
        let (Some(name), Some(data)) = (self.attachment_name(index), self.attachment_data(index))
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
        let saved =
            std::fs::create_dir_all(&folder).and_then(|()| write_new(&folder, &name, &data));
        if let Err(error) = saved {
            tracing::warn!("cannot save {name}: {error}");
        }
    }

    fn remove_unedited_attachments(&mut self) {
        let unedited = self
            .opened
            .drain(..)
            .filter(|(_, path, written)| {
                std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .is_ok_and(|modified| modified == *written)
            })
            .map(|(_, path, _)| path)
            .collect::<Vec<_>>();
        if unedited.is_empty() {
            return;
        }
        let in_use = open_files();
        for path in unedited {
            let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !in_use.contains(&real)
                && std::fs::remove_file(&path).is_ok()
                && let Some(folder) = path.parent()
            {
                let _ = std::fs::remove_dir(folder);
            }
        }
    }

    pub fn engine_ready(&mut self) -> bool {
        let events = self.mailroom.take();
        let mut changed = false;
        let mut stale = false;
        let mut held = Vec::new();
        for event in events {
            changed |= !matches!(event, MailEvent::Synced(..));
            match event {
                MailEvent::Status(account, status) => {
                    self.statuses.retain(|(known, _)| *known != account);
                    self.statuses.push((account, status));
                }
                MailEvent::Changed(_) => stale = true,
                MailEvent::Synced(account, mailbox) => {
                    let (moves, waiting) = std::mem::take(&mut self.undoing)
                        .into_iter()
                        .partition(|moved| moved.from.account == account && moved.to == mailbox);
                    self.undoing = waiting;
                    if !moves.is_empty() {
                        changed = true;
                        self.put_back(moves);
                    }
                }
                MailEvent::Arrived(account, mut ids) => {
                    stale = true;
                    ids.retain(|id| !self.expected_arrival(*id));
                    let left = self.apply_rules(account, ids);
                    self.notify(&left);
                }
                MailEvent::Body(id) => {
                    if self
                        .thread
                        .as_ref()
                        .and_then(|thread| thread.entry(id))
                        .is_some_and(|entry| entry.reading.is_some())
                    {
                        self.read_entry(id);
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
                MailEvent::MailboxRefused(reason) => {
                    self.ui.present_sheet(
                        Alert::new(&L10N.tr("mailbox-refused"), &reason, |_| {})
                            .primary(&L10N.tr("button-ok"), "ok"),
                    );
                }
                MailEvent::NotSent(_, error, outgoing) => {
                    self.requests.push(Request::NotSent { error, outgoing });
                }
                MailEvent::Held(account, command) => {
                    if self.worker(account).is_some() {
                        self.send_command(account, *command);
                    } else {
                        self.held.push((account, *command));
                        held.push(account);
                    }
                }
                MailEvent::Setup(outcome) => self.setup_checked(*outcome),
                MailEvent::Pictures(id, pictures) => {
                    if let Some(reading) = self
                        .thread
                        .as_mut()
                        .and_then(|thread| thread.entry_mut(id))
                        .and_then(|entry| entry.reading.as_mut())
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
        if !held.is_empty() {
            held.dedup();
            self.tell_held(&held);
        }
        if stale {
            self.reload();
        }
        changed
    }

    fn apply_rules(&mut self, account: AccountId, mut ids: Vec<MessageId>) -> Vec<MessageId> {
        for rule in self.store.rules().unwrap_or_default() {
            if ids.is_empty() {
                break;
            }
            let matched = self.store.matching(&ids, &rule).unwrap_or_default();
            if matched.is_empty() {
                continue;
            }
            match rule.action {
                RuleAction::Move(target) => {
                    let Some(target) = self
                        .mailbox(target)
                        .filter(|target| target.account == account)
                    else {
                        continue;
                    };
                    match self.store.move_messages(&matched, &target) {
                        Ok(accounts) => self.flush(&accounts),
                        Err(error) => tracing::warn!("cannot apply a rule: {error}"),
                    }
                    ids.retain(|id| !matched.contains(id));
                }
                RuleAction::Flag => self.set_flag_on(&matched, Flag::Flagged, true),
                RuleAction::Read => {
                    self.set_flag_on(&matched, Flag::Seen, true);
                    ids.retain(|id| !matched.contains(id));
                }
            }
        }
        ids
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
            .store
            .accounts()
            .unwrap_or_default()
            .iter()
            .any(|account| account.email.eq_ignore_ascii_case(&email))
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
                let message = match app_password_hint(&settings.imap.host) {
                    Some(hint) => format!("{} {}", L10N.tr("setup-wrong-password"), L10N.tr(hint)),
                    None => L10N.tr("setup-wrong-password"),
                };
                state.status = Some((message, true));
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
            enabled: true,
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
        let Some(account) = self
            .store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .find(|account| account.id == id)
        else {
            return;
        };
        self.workers.retain(|worker| worker.account != id);
        if let Err(error) = self.store.remove_account(id) {
            tracing::warn!("cannot remove {}: {error}", account.email);
        }
        self.release_held();
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
                signal @ (Signal::NewMailbox(..)
                | Signal::RenameMailbox(..)
                | Signal::DeleteMailbox(_)) => self.change_mailboxes(signal),
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
                    return self.ui.handle(event);
                }
                self.ui.clear_focus();
                self.click(pointer)
            }
            Event::PointerUp(_) if self.drag.is_some() => {
                self.drop_messages();
                true
            }
            Event::PointerUp(_) => self.ui.handle(event),
            Event::PointerMove(pointer) if self.drag.is_some() => self.drag_to(pointer.position),
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                let ui = self.ui.handle(event);
                let hover = self.hit(pointer.position);
                let changed = hover != self.hover;
                self.hover = hover;
                ui | changed
            }
            Event::PointerLeave => {
                self.ui.handle(event) | self.hover.take().is_some() | self.drag.take().is_some()
            }
            Event::Scroll(scroll) => {
                self.ui.handle(event)
                    || (!self.over_control(scroll.position) && self.scroll(scroll))
            }
            _ => self.ui.handle(event),
        }
    }

    fn drop_target(&self, index: usize) -> bool {
        match self.sidebar.get(index) {
            Some(Row::Place { place, .. }) if *place == self.place => false,
            Some(Row::Place {
                place: Place::Mailbox(id),
                ..
            }) => self
                .accounts
                .iter()
                .flat_map(|(_, mailboxes)| mailboxes)
                .any(|mailbox| mailbox.id == *id && mailbox.selectable),
            Some(Row::Place {
                place: Place::All(role),
                ..
            }) => *role != Role::Drafts && *role != Role::Sent,
            _ => false,
        }
    }

    fn drag_to(&mut self, at: Point) -> bool {
        let over = match self.hit(at) {
            Some(Hit::Place(index)) if self.drop_target(index) => Some(index),
            _ => None,
        };
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        let moving =
            drag.moving || (at.x - drag.from.x).abs() + (at.y - drag.from.y).abs() > DRAG_THRESHOLD;
        let changed = moving && (drag.at != at || drag.over != over || !drag.moving);
        *drag = Drag {
            at,
            moving,
            over: over.filter(|_| moving),
            ..*drag
        };
        changed
    }

    fn drop_messages(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let (false, Some(id)) = (drag.moving, drag.collapse) {
            self.select(id);
        }
        let (true, Some(index)) = (drag.moving, drag.over) else {
            return;
        };
        match self.sidebar.get(index).cloned() {
            Some(Row::Place {
                place: Place::Mailbox(id),
                ..
            }) => {
                let target = self
                    .accounts
                    .iter()
                    .flat_map(|(_, mailboxes)| mailboxes)
                    .find(|mailbox| mailbox.id == id)
                    .cloned();
                if let Some(target) = target {
                    self.move_selected_to(&target);
                }
            }
            Some(Row::Place {
                place: Place::All(Role::Trash),
                ..
            }) => self.delete_selected(),
            Some(Row::Place {
                place: Place::All(role),
                ..
            }) => self.move_selected(role),
            _ => {}
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
            Key::Character('z' | 'Z') if modifiers.control && !modifiers.shift => {
                self.undo();
                true
            }
            Key::Character('a' | 'A') if modifiers.control && !modifiers.shift => {
                self.select_all();
                true
            }
            Key::Up if plain && modifiers.shift => self.extend(-1),
            Key::Down if plain && modifiers.shift => self.extend(1),
            Key::Right | Key::Left if plain && !modifiers.shift => {
                let expanded = self.selected.is_some_and(|id| {
                    self.conversations.children.contains(&id)
                        || self
                            .conversations
                            .earlier
                            .get(&id)
                            .is_some_and(|(key, _)| self.conversations.expanded.contains(key))
                });
                match (key == Key::Right, expanded, self.selected) {
                    (true, false, Some(id)) | (false, true, Some(id)) => {
                        self.toggle_conversation(id)
                    }
                    _ => false,
                }
            }
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

    fn select_all(&mut self) {
        let Some(first) = self
            .selected
            .or_else(|| self.messages.first().map(|summary| summary.id))
        else {
            return;
        };
        if self.selected.is_none() {
            self.select(first);
        }
        self.also = self
            .messages
            .iter()
            .map(|summary| summary.id)
            .filter(|id| *id != first)
            .collect();
        if !self.also.is_empty() {
            self.thread = None;
            self.layout = None;
        }
    }

    fn extend(&mut self, by: isize) -> bool {
        let chosen = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, summary)| self.is_chosen(summary.id))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let (Some(first), Some(last)) = (chosen.first(), chosen.last()) else {
            return self.step(by);
        };
        let next = match by < 0 {
            true => first.checked_sub(1),
            false => Some(last + 1).filter(|next| *next < self.messages.len()),
        };
        let Some(id) = next.map(|next| self.messages[next].id) else {
            return false;
        };
        self.select_more(id, false);
        true
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
        let secondary = pointer.button == PointerButton::Secondary;
        match hit {
            Hit::Place(index) => {
                self.focus = Pane::Sidebar;
                if let Some(Row::Place { place, .. }) = self.sidebar.get(index).cloned() {
                    if secondary && let Some(menu) = self.mailbox_menu(&place) {
                        self.requests
                            .push(Request::ShowMenu(menu, pointer.position));
                    } else {
                        self.choose_place(place);
                    }
                }
            }
            Hit::Message(id)
                if !secondary && (pointer.modifiers.control || pointer.modifiers.shift) =>
            {
                self.select_more(id, pointer.modifiers.shift);
            }
            Hit::Message(id) => {
                let collapse = !secondary && self.is_chosen(id) && !self.also.is_empty();
                if !self.is_chosen(id) {
                    self.select(id);
                }
                if !secondary && pointer.count == 1 {
                    self.drag = Some(Drag {
                        from: pointer.position,
                        at: pointer.position,
                        moving: false,
                        over: None,
                        collapse: collapse.then_some(id),
                    });
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
            Hit::Conversation(id) => {
                self.toggle_conversation(id);
            }
            Hit::Reader(target) => self.click_reader(target, pointer),
        }
        true
    }

    fn click_reader(&mut self, target: Target, pointer: &Pointer) {
        let secondary = pointer.button == PointerButton::Secondary;
        match target {
            Target::Card(id) => self.focus_entry(id),
            Target::Expand(id) => self.toggle_entry(id),
            Target::Quotes(id) => {
                self.focus_entry(id);
                if let Some(entry) = self.thread.as_mut().and_then(|thread| thread.entry_mut(id)) {
                    entry.quotes = !entry.quotes;
                    self.layout = None;
                }
            }
            Target::Respond(id, respond) => {
                self.focus_entry(id);
                match respond {
                    Respond::Reply => self.respond(false),
                    Respond::ReplyAll => self.respond(true),
                    Respond::Forward => self.forward_selected(),
                }
            }
            Target::Link(url) => self.requests.push(Request::OpenLink(url)),
            Target::Attachment(id, index) if secondary => {
                self.focus_entry(id);
                let menu = Menu::new(vec![
                    MenuItem::new(
                        &L10N.tr("attachment-open"),
                        &format!("attachment-open:{index}"),
                    ),
                    MenuItem::new(
                        &L10N.tr("attachment-save-as"),
                        &format!("attachment-save-as:{index}"),
                    ),
                    MenuItem::new(
                        &L10N.tr("attachment-save"),
                        &format!("attachment-save:{index}"),
                    ),
                ]);
                self.requests
                    .push(Request::ShowMenu(menu, pointer.position));
            }
            Target::Attachment(id, index) => {
                self.focus_entry(id);
                self.open_attachment(index);
            }
            Target::LoadRemote(id) => {
                let sender = self
                    .thread
                    .as_ref()
                    .and_then(|thread| thread.entry(id))
                    .and_then(|entry| entry.reading.as_ref())
                    .map(|reading| reading.content.envelope.sender().email)
                    .filter(|email| !email.is_empty());
                if let Some(email) = sender
                    && let Err(error) = self.store.trust_remote(&email)
                {
                    tracing::warn!("cannot remember {email}: {error}");
                }
                self.load_remote(id);
            }
        }
    }

    fn mailbox(&self, id: MailboxId) -> Option<Mailbox> {
        self.accounts
            .iter()
            .flat_map(|(_, mailboxes)| mailboxes)
            .find(|mailbox| mailbox.id == id)
            .cloned()
    }

    pub fn current_account(&self) -> Option<AccountId> {
        match self.place {
            Place::Mailbox(id) => self.mailbox(id).map(|mailbox| mailbox.account),
            _ => None,
        }
        .or_else(|| self.accounts.first().map(|(account, _)| account.id))
    }

    fn mailbox_menu(&self, place: &Place) -> Option<Menu> {
        let Place::Mailbox(id) = place else {
            return None;
        };
        let mailbox = self.mailbox(*id)?;
        let own = mailbox.role.is_none();
        Some(Menu::new(vec![
            MenuItem::new(
                &L10N.tr("menu-new-mailbox"),
                &format!("mailbox-new:{}", mailbox.account),
            ),
            MenuItem::new(
                &L10N.tr("menu-rename-mailbox"),
                &format!("mailbox-rename:{id}"),
            )
            .enabled(own),
            MenuItem::new(
                &L10N.tr("menu-delete-mailbox"),
                &format!("mailbox-delete:{id}"),
            )
            .enabled(own),
        ]))
    }

    fn ask_mailbox_name(
        &mut self,
        title: &str,
        name: &str,
        done: impl Fn(String) -> Signal + 'static,
    ) {
        let typed = Rc::new(RefCell::new(name.to_owned()));
        let field = {
            let typed = Rc::clone(&typed);
            TextField::new(&L10N.tr("mailbox-name-placeholder"), name, move |text| {
                *typed.borrow_mut() = text.to_owned();
            })
        };
        let signals = Rc::clone(&self.signals);
        let alert = Alert::new(title, "", move |choice| {
            let name = typed.borrow().trim().to_owned();
            if choice == "ok" && !name.is_empty() {
                signals.borrow_mut().push(done(name));
            }
        })
        .accessory(field)
        .primary(&L10N.tr("button-ok"), "ok")
        .cancel(&L10N.tr("button-cancel"), "cancel");
        self.ui.present_sheet(alert);
    }

    fn mailbox_action(&mut self, verb: &str, id: i64) {
        match verb {
            "mailbox-new" => {
                self.ask_mailbox_name(&L10N.tr("new-mailbox-title"), "", move |name| {
                    Signal::NewMailbox(id, name)
                });
            }
            "mailbox-rename" => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                let title = L10N.tr_args(
                    "rename-mailbox-title",
                    [("name", mailbox.name.clone().into())],
                );
                self.ask_mailbox_name(&title, &mailbox.name, move |name| {
                    Signal::RenameMailbox(id, name)
                });
            }
            "mailbox-delete" => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                let signals = Rc::clone(&self.signals);
                let alert = Alert::new(
                    &L10N.tr_args("delete-mailbox-title", [("name", mailbox.name.into())]),
                    &L10N.tr("delete-mailbox-message"),
                    move |choice| {
                        if choice == "delete" {
                            signals.borrow_mut().push(Signal::DeleteMailbox(id));
                        }
                    },
                )
                .destructive(&L10N.tr("action-delete"), "delete")
                .cancel(&L10N.tr("button-cancel"), "cancel");
                self.ui.present_sheet(alert);
            }
            _ => {}
        }
    }

    fn change_mailboxes(&mut self, signal: Signal) {
        let (account, change) = match signal {
            Signal::NewMailbox(account, name) => {
                let prefix = self.namespace_prefix(account);
                (account, MailboxChange::Create(format!("{prefix}{name}")))
            }
            Signal::RenameMailbox(id, name) => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                let parent = mailbox
                    .delimiter
                    .and_then(|delimiter| {
                        mailbox
                            .path
                            .rsplit_once(delimiter)
                            .map(|(parent, _)| format!("{parent}{delimiter}"))
                    })
                    .unwrap_or_default();
                (
                    mailbox.account,
                    MailboxChange::Rename(mailbox.path, format!("{parent}{name}")),
                )
            }
            Signal::DeleteMailbox(id) => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                (mailbox.account, MailboxChange::Delete(mailbox.path))
            }
            _ => return,
        };
        self.send_command(account, Command::Mailbox(change));
    }

    fn namespace_prefix(&self, account: AccountId) -> String {
        let Some((_, mailboxes)) = self.accounts.iter().find(|(known, _)| known.id == account)
        else {
            return String::new();
        };
        let inbox = mailboxes
            .iter()
            .find(|mailbox| mailbox.role == Some(Role::Inbox));
        let Some(prefix) = inbox.and_then(|inbox| {
            inbox
                .delimiter
                .map(|delimiter| format!("{}{delimiter}", inbox.path))
        }) else {
            return String::new();
        };
        let others = mailboxes
            .iter()
            .filter(|mailbox| mailbox.role != Some(Role::Inbox))
            .collect::<Vec<_>>();
        match !others.is_empty()
            && others
                .iter()
                .all(|mailbox| mailbox.path.starts_with(&prefix))
        {
            true => prefix,
            false => String::new(),
        }
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
            MenuItem::submenu(
                &L10N.tr("menu-move-to"),
                Menu::new(
                    self.move_targets()
                        .into_iter()
                        .map(|(name, id)| MenuItem::new(&name, &format!("move-to:{id}")))
                        .collect(),
                ),
            ),
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
            "mark-all-read" => self.mark_all_read(),
            "bcc-field" => {
                self.settings.bcc_field = !self.settings.bcc_field;
                self.keep_settings();
            }
            "sort-ascending" | "sort-descending" => {
                self.settings.ascending = action == "sort-ascending";
                self.keep_settings();
            }
            action if action.starts_with("sort-") => {
                if let Some(sort) = SortBy::ALL
                    .into_iter()
                    .find(|sort| action.strip_prefix("sort-") == Some(sort.name()))
                {
                    self.settings.sort = sort;
                    self.keep_settings();
                }
            }
            "archive" => self.move_selected(Role::Archive),
            "junk" => self.move_selected(Role::Junk),
            "delete" => self.delete_selected(),
            "compose" => self.compose(Draft::default()),
            "get-mail" => self.get_mail(),
            "add-account" => self.present_setup(Mode::Add),
            "new-mailbox" => {
                if let Some(account) = self.current_account() {
                    self.mailbox_action("mailbox-new", account);
                }
            }
            "find" => self.ui.set_focus(self.toolbar.search),
            action if action.starts_with("move-to:") => {
                let target = action
                    .strip_prefix("move-to:")
                    .and_then(|id| id.parse::<MailboxId>().ok())
                    .and_then(|id| {
                        self.accounts
                            .iter()
                            .flat_map(|(_, mailboxes)| mailboxes)
                            .find(|mailbox| mailbox.id == id)
                            .cloned()
                    });
                if let Some(target) = target {
                    self.move_selected_to(&target);
                }
            }
            action if action.starts_with("mailbox-") => {
                if let Some((verb, id)) = action
                    .split_once(':')
                    .and_then(|(verb, id)| Some((verb, id.parse().ok()?)))
                {
                    self.mailbox_action(verb, id);
                }
            }
            action if action.starts_with("attachment-") => {
                let parsed = action
                    .split_once(':')
                    .and_then(|(verb, index)| Some((verb, index.parse::<usize>().ok()?)));
                match parsed {
                    Some(("attachment-open", index)) => self.open_attachment(index),
                    Some(("attachment-save", index)) => self.save_attachment(index),
                    Some(("attachment-save-as", index)) => {
                        if let (Some(name), Some(data)) =
                            (self.attachment_name(index), self.attachment_data(index))
                        {
                            self.requests.push(Request::SaveAttachment(name, data));
                        }
                    }
                    _ => {}
                }
            }
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

fn write_new(folder: &std::path::Path, name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for copy in 0..1000 {
        let path = match copy {
            0 => folder.join(name),
            copy => folder.join(format!("{stem} {}{extension}", copy + 1)),
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => return file.write_all(data).map(|()| path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("no free file name"))
}

fn app_password_hint(host: &str) -> Option<&'static str> {
    let host = host.to_ascii_lowercase();
    let provider = |domains: &[&str]| {
        domains
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    };
    if provider(&["gmail.com", "googlemail.com"]) {
        Some("hint-google")
    } else if provider(&["outlook.com", "office365.com", "hotmail.com", "live.com"]) {
        Some("hint-microsoft")
    } else if provider(&["me.com", "mac.com", "icloud.com"]) {
        Some("hint-icloud")
    } else if provider(&["yahoo.com", "aol.com"]) {
        Some("hint-yahoo")
    } else if provider(&["fastmail.com", "messagingengine.com"]) {
        Some("hint-fastmail")
    } else {
        None
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
