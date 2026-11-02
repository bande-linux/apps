use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::compose::{Draft, Outgoing, build, forget_unsent};
use crate::error::{Error, Result};
use crate::imap::{Flag, Role, Session};
use crate::net::{Closer, Network};
use crate::smtp;
use crate::store::{Account, AccountId, Mailbox, MailboxId, MessageId, Store};
use crate::sync::{download_bodies, fetch_body, flush, sync_folders, sync_mailbox};

const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const FULL_SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);
const IDLE_RENEW: Duration = Duration::from_secs(25 * 60);
const FIRST_RETRY: Duration = Duration::from_secs(5);
const LAST_RETRY: Duration = Duration::from_secs(5 * 60);
const BODY_ROUNDS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Syncing,
    Ready,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Sync,
    Refresh(MailboxId),
    Body(MessageId),
    Flush,
    Send(Box<Outgoing>),
    SaveDraft(Box<Outgoing>),
    Password(String),
    Mailbox(MailboxChange),
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailboxChange {
    Create(String),
    Rename(String, String),
    Delete(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(AccountId, Status),
    Changed(AccountId),
    Arrived(AccountId, Vec<MessageId>),
    Body(MessageId),
    LoginFailed(AccountId, String),
    Failed(AccountId, String),
    Sent(AccountId),
    NotSent(AccountId, String, Box<Outgoing>),
    Setup(Box<crate::setup::Outcome>),
    Pictures(MessageId, Vec<(String, crate::picture::Picture)>),
    MailboxRefused(String),
}

#[derive(Clone)]
pub struct Notifier {
    events: Sender<Event>,
    wake: Arc<UnixStream>,
}

impl Notifier {
    pub fn send(&self, event: Event) {
        if self.events.send(event).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

pub struct Mailroom {
    events: Receiver<Event>,
    awake: UnixStream,
    notifier: Notifier,
}

impl Mailroom {
    pub fn new() -> io::Result<Self> {
        let (awake, wake) = UnixStream::pair()?;
        awake.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        let (events, received) = mpsc::channel();
        Ok(Self {
            events: received,
            awake,
            notifier: Notifier {
                events,
                wake: Arc::new(wake),
            },
        })
    }

    pub fn notifier(&self) -> Notifier {
        self.notifier.clone()
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.awake.as_fd()
    }

    pub fn take(&mut self) -> Vec<Event> {
        let mut buffer = [0u8; 256];
        while matches!((&self.awake).read(&mut buffer), Ok(count) if count > 0) {}
        self.events.try_iter().collect()
    }

    pub fn wait(&mut self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

pub type PasswordSource = Box<dyn FnMut() -> Result<Option<String>> + Send>;

#[derive(Default)]
struct Watch {
    closer: Option<Closer>,
    thread: Option<thread::Thread>,
}

impl Watch {
    fn interrupt(&self) {
        if let Some(closer) = &self.closer {
            closer.close();
        }
        if let Some(thread) = &self.thread {
            thread.unpark();
        }
    }
}

pub struct Worker {
    pub account: AccountId,
    commands: Sender<Command>,
    stopping: Arc<AtomicBool>,
    idle: Arc<Mutex<Watch>>,
    running: Option<Receiver<()>>,
}

impl Worker {
    pub fn spawn(
        account: Account,
        store: PathBuf,
        password: PasswordSource,
        network: Network,
        notifier: Notifier,
    ) -> Self {
        let (commands, received) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let idle = Arc::new(Mutex::new(Watch::default()));
        let mut worker = Self {
            account: account.id,
            commands: commands.clone(),
            stopping: Arc::clone(&stopping),
            idle: Arc::clone(&idle),
            running: None,
        };
        let id = account.id;
        let (alive, running) = mpsc::channel::<()>();
        let spawned = thread::Builder::new()
            .name(format!("mail-{id}"))
            .spawn(move || {
                let _alive = alive;
                let store = match Store::open(&store) {
                    Ok(store) => store,
                    Err(error) => {
                        notifier.send(Event::Failed(id, error.to_string()));
                        return;
                    }
                };
                Runner {
                    account,
                    store,
                    session: None,
                    password: None,
                    source: password,
                    network,
                    notifier,
                    commands: received,
                    sender: commands,
                    stopping,
                    idle,
                    idling: Arc::new(AtomicBool::new(false)),
                    retry: FIRST_RETRY,
                    signed_out: false,
                }
                .run();
            });
        match spawned {
            Ok(_) => worker.running = Some(running),
            Err(error) => tracing::error!("cannot start the mail worker: {error}"),
        }
        worker.send(Command::Sync);
        worker
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    fn close_idle(&self) {
        if let Ok(idle) = self.idle.lock() {
            idle.interrupt();
        }
    }

    pub fn finish(mut self, deadline: Instant) {
        self.close_idle();
        let _ = self.commands.send(Command::Stop);
        if let Some(running) = self.running.take() {
            let _ = running.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.running.is_some() {
            self.stopping.store(true, Ordering::Relaxed);
        }
        self.close_idle();
        let _ = self.commands.send(Command::Stop);
    }
}

struct Runner {
    account: Account,
    store: Store,
    session: Option<Session>,
    password: Option<String>,
    source: PasswordSource,
    network: Network,
    notifier: Notifier,
    commands: Receiver<Command>,
    sender: Sender<Command>,
    stopping: Arc<AtomicBool>,
    idle: Arc<Mutex<Watch>>,
    idling: Arc<AtomicBool>,
    retry: Duration,
    signed_out: bool,
}

fn connect(account: &Account, password: &str, network: &Network) -> Result<Session> {
    let mut session = Session::connect(&account.imap, network)?;
    session.login(&account.user, password)?;
    Ok(session)
}

impl Runner {
    fn emit(&self, event: Event) {
        self.notifier.send(event);
    }

    fn status(&self, status: Status) {
        self.emit(Event::Status(self.account.id, status));
    }

    fn password(&mut self) -> Result<String> {
        if self.password.is_none() {
            self.password = (self.source)()?;
        }
        self.password
            .clone()
            .ok_or_else(|| Error::Login("no password is saved for this account".to_owned()))
    }

    fn session(&mut self) -> Result<&mut Session> {
        if self.session.is_none() {
            self.status(Status::Connecting);
            let password = self.password()?;
            let session = connect(&self.account, &password, &self.network)?;
            if session.has("IDLE") && !self.idling.load(Ordering::Relaxed) {
                self.start_idle(password);
            }
            self.session = Some(session);
        }
        self.session.as_mut().ok_or(Error::Closed)
    }

    fn connected(&mut self) -> Result<(&mut Session, &mut Store)> {
        self.session()?;
        match self.session.as_mut() {
            Some(session) => Ok((session, &mut self.store)),
            None => Err(Error::Closed),
        }
    }

    fn start_idle(&mut self, password: String) {
        let Ok(Some(inbox)) = self.store.mailbox_with_role(self.account.id, Role::Inbox) else {
            return;
        };
        self.idling.store(true, Ordering::Relaxed);
        let idling = Arc::clone(&self.idling);
        let idle = Idle {
            account: self.account.clone(),
            password,
            network: self.network.clone(),
            inbox,
            commands: self.sender.clone(),
            stopping: Arc::clone(&self.stopping),
            closer: Arc::clone(&self.idle),
        };
        let spawned = thread::Builder::new()
            .name(format!("mail-idle-{}", self.account.id))
            .spawn(move || {
                idle.run();
                idling.store(false, Ordering::Relaxed);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot watch for new mail: {error}");
            self.idling.store(false, Ordering::Relaxed);
        }
    }

    fn next_command(&mut self) -> Option<Command> {
        if self.signed_out {
            self.signed_out = false;
            return match self.commands.recv() {
                Ok(Command::Stop) | Err(_) => None,
                Ok(command) => Some(command),
            };
        }
        let wait = match (self.session.is_some(), self.idling.load(Ordering::Relaxed)) {
            (false, _) => self.retry,
            (true, false) => POLL_INTERVAL,
            (true, true) => FULL_SYNC_INTERVAL,
        };
        match self.commands.recv_timeout(wait) {
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => None,
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => Some(Command::Sync),
        }
    }

    fn run(mut self) {
        while let Some(mut command) = self.next_command() {
            if self.stopping.load(Ordering::Relaxed) {
                break;
            }
            if command == Command::Sync {
                while let Ok(queued) = self.commands.try_recv() {
                    match queued {
                        Command::Stop => return,
                        Command::Sync | Command::Refresh(_) | Command::Flush => {}
                        other => {
                            self.attempt(&other);
                        }
                    }
                }
            }
            if let Command::Password(password) = command {
                self.password = Some(password);
                self.session = None;
                if let Ok(idle) = self.idle.lock() {
                    idle.interrupt();
                }
                command = Command::Sync;
            }
            self.attempt(&command);
        }
        if let Some(session) = self.session.take() {
            session.logout();
        }
    }

    fn attempt(&mut self, command: &Command) {
        let mut result = self.execute(command);
        if matches!(&result, Err(error) if error.is_connection())
            && self.session.is_some()
            && !matches!(command, Command::Send(_))
        {
            self.session = None;
            result = self.execute(command);
        }
        match result {
            Ok(()) => self.retry = FIRST_RETRY,
            Err(error) => self.failed(command, error),
        }
    }

    fn failed(&mut self, command: &Command, error: Error) {
        tracing::warn!("{}: {error}", self.account.email);
        if let Command::Send(outgoing) | Command::SaveDraft(outgoing) = command {
            self.emit(Event::NotSent(
                self.account.id,
                error.to_string(),
                outgoing.clone(),
            ));
        }
        match error {
            Error::Login(text) => {
                self.password = None;
                self.session = None;
                self.signed_out = true;
                self.status(Status::Offline);
                self.emit(Event::LoginFailed(self.account.id, text));
            }
            error if error.is_connection() || matches!(error, Error::Insecure) => {
                self.session = None;
                self.retry = (self.retry * 2).min(LAST_RETRY);
                self.status(Status::Offline);
                self.emit(Event::Failed(self.account.id, error.to_string()));
            }
            error => {
                self.signed_out = matches!(error, Error::Secret(_));
                self.status(Status::Ready);
                self.emit(Event::Failed(self.account.id, error.to_string()));
            }
        }
    }

    fn execute(&mut self, command: &Command) -> Result<()> {
        match command {
            Command::Sync => self.sync_all(),
            Command::Refresh(mailbox) => {
                let mailbox = self.store.mailbox(*mailbox)?;
                self.flush()?;
                if let Some(mailbox) = mailbox {
                    self.sync_one(&mailbox)?;
                    self.bodies(&mailbox)?;
                }
                Ok(())
            }
            Command::Flush => {
                let Some(targets) = self.flush()? else {
                    return Ok(());
                };
                self.emit(Event::Changed(self.account.id));
                let mailboxes = self.store.mailboxes(self.account.id)?;
                for target in targets {
                    if let Some(mailbox) = mailboxes.iter().find(|mailbox| mailbox.path == target) {
                        self.sync_one(mailbox)?;
                    }
                }
                Ok(())
            }
            Command::Body(id) => self.body(*id),
            Command::Mailbox(change) => self.change_mailbox(change),
            Command::Send(outgoing) => self.send(outgoing),
            Command::SaveDraft(outgoing) => {
                self.save_draft(&outgoing.draft, outgoing.replaces)?;
                forget_unsent(outgoing);
                Ok(())
            }
            Command::Password(_) | Command::Stop => Ok(()),
        }
    }

    fn change_mailbox(&mut self, change: &MailboxChange) -> Result<()> {
        let session = self.session()?;
        let changed = match change {
            MailboxChange::Create(path) => session.create(path),
            MailboxChange::Rename(from, to) => session.rename(from, to),
            MailboxChange::Delete(path) => session.delete(path),
        };
        let account = self.account.id;
        match changed {
            Err(Error::Refused(text)) => self.emit(Event::MailboxRefused(text)),
            Err(error) => return Err(error),
            Ok(()) => {
                if let MailboxChange::Rename(from, to) = change {
                    let delimiter = self
                        .store
                        .mailboxes(account)?
                        .into_iter()
                        .find(|mailbox| mailbox.path == *from)
                        .and_then(|mailbox| mailbox.delimiter);
                    self.store.rename_mailbox(account, from, to, delimiter)?;
                }
            }
        }
        let (session, store) = self.connected()?;
        sync_folders(session, store, account)?;
        self.emit(Event::Changed(account));
        Ok(())
    }

    fn flush(&mut self) -> Result<Option<Vec<String>>> {
        let account = self.account.id;
        let (session, store) = self.connected()?;
        flush(session, store, account)
    }

    fn sync_one(&mut self, mailbox: &Mailbox) -> Result<()> {
        let (session, store) = self.connected()?;
        let report = sync_mailbox(session, store, mailbox)?;
        if report.changed {
            self.emit(Event::Changed(self.account.id));
        }
        if mailbox.role == Some(Role::Inbox) && !report.arrived.is_empty() {
            let unseen = self
                .store
                .summaries(&crate::store::Scope::Mailbox(mailbox.id), "")?
                .into_iter()
                .filter(|summary| report.arrived.contains(&summary.uid) && !summary.flags.seen)
                .map(|summary| summary.id)
                .collect::<Vec<_>>();
            if !unseen.is_empty() {
                self.emit(Event::Arrived(self.account.id, unseen));
            }
        }
        Ok(())
    }

    fn bodies(&mut self, mailbox: &Mailbox) -> Result<()> {
        for _ in 0..BODY_ROUNDS {
            if self.stopping.load(Ordering::Relaxed) {
                break;
            }
            let (session, store) = self.connected()?;
            if download_bodies(session, store, mailbox)? == 0 {
                break;
            }
            self.emit(Event::Changed(self.account.id));
        }
        Ok(())
    }

    fn sync_all(&mut self) -> Result<()> {
        self.session()?;
        self.status(Status::Syncing);
        self.flush()?;
        let account = self.account.id;
        let (session, store) = self.connected()?;
        sync_folders(session, store, account)?;
        self.emit(Event::Changed(self.account.id));
        let mut mailboxes = self
            .store
            .mailboxes(self.account.id)?
            .into_iter()
            .filter(|mailbox| mailbox.selectable)
            .collect::<Vec<_>>();
        mailboxes.sort_by_key(|mailbox| (mailbox.role.is_none(), mailbox.role));
        for mailbox in &mailboxes {
            if self.stopping.load(Ordering::Relaxed) {
                return Ok(());
            }
            match self.sync_one(mailbox) {
                Err(Error::Refused(text)) => {
                    tracing::info!("{} cannot be read: {text}", mailbox.path)
                }
                other => other?,
            }
        }
        if !self.idling.load(Ordering::Relaxed)
            && let Some(password) = self.password.clone()
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.has("IDLE"))
        {
            self.start_idle(password);
        }
        for mailbox in &mailboxes {
            self.bodies(mailbox)?;
        }
        self.status(Status::Ready);
        Ok(())
    }

    fn body(&mut self, id: MessageId) -> Result<()> {
        let Some(summary) = self.store.summary(id)? else {
            return Ok(());
        };
        let Some(mailbox) = self.store.mailbox(summary.mailbox)? else {
            return Ok(());
        };
        let (session, store) = self.connected()?;
        if fetch_body(session, store, &mailbox, summary.uid)? {
            self.emit(Event::Body(id));
        }
        Ok(())
    }

    fn replace_draft(&mut self, replaces: Option<MessageId>) -> Result<()> {
        if let Some(old) = replaces {
            self.store.expunge(&[old])?;
            self.flush()?;
        }
        if let Some(drafts) = self
            .store
            .mailbox_with_role(self.account.id, Role::Drafts)?
        {
            self.sync_one(&drafts)?;
            self.bodies(&drafts)?;
        }
        Ok(())
    }

    fn save_draft(&mut self, draft: &Draft, replaces: Option<MessageId>) -> Result<()> {
        let Some(drafts) = self
            .store
            .mailbox_with_role(self.account.id, Role::Drafts)?
        else {
            return Err(Error::Refused(
                "the account has no Drafts mailbox".to_owned(),
            ));
        };
        let raw = build(draft, &jiff::Zoned::now());
        let session = self.session()?;
        session.append(&drafts.path, &[Flag::Seen, Flag::Draft], &raw)?;
        self.replace_draft(replaces)
    }

    fn send(&mut self, outgoing: &Outgoing) -> Result<()> {
        let Outgoing {
            draft,
            answering,
            replaces,
            ..
        } = outgoing;
        let (answering, replaces) = (*answering, *replaces);
        let raw = build(draft, &jiff::Zoned::now());
        let recipients = draft.recipients();
        let password = self.password()?;
        smtp::send(
            &self.account.smtp,
            &self.network,
            Some((&self.account.user, &password)),
            &smtp::Envelope {
                from: &self.account.email,
                recipients: &recipients,
                message: &raw,
            },
        )?;
        forget_unsent(outgoing);
        self.emit(Event::Sent(self.account.id));
        let mut kept = self.file_sent(&raw, answering, replaces);
        if matches!(&kept, Err(error) if error.is_connection()) {
            self.session = None;
            kept = self.file_sent(&raw, answering, replaces);
        }
        if let Err(error) = kept {
            tracing::warn!("{}: sent, but cannot file it: {error}", self.account.email);
            if error.is_connection() {
                self.session = None;
            }
        }
        Ok(())
    }

    fn file_sent(
        &mut self,
        raw: &[u8],
        answering: Option<MessageId>,
        replaces: Option<MessageId>,
    ) -> Result<()> {
        let sent = self.store.mailbox_with_role(self.account.id, Role::Sent)?;
        if let Some(sent) = sent.filter(|_| !saves_sent_mail(&self.account.imap.host)) {
            match self.session()?.append(&sent.path, &[Flag::Seen], raw) {
                Err(error) if error.is_connection() => return Err(error),
                Err(error) => tracing::warn!("cannot keep a copy in {}: {error}", sent.path),
                Ok(()) => {}
            }
        }
        if let Some(original) = answering {
            self.store.set_flag(&[original], Flag::Answered, true)?;
            self.flush()?;
        }
        if let Some(sent) = self.store.mailbox_with_role(self.account.id, Role::Sent)? {
            self.sync_one(&sent)?;
            self.bodies(&sent)?;
        }
        if replaces.is_some() {
            self.replace_draft(replaces)?;
        }
        Ok(())
    }
}

fn saves_sent_mail(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "imap.gmail.com" || host.ends_with(".googlemail.com")
}

struct Idle {
    account: Account,
    password: String,
    network: Network,
    inbox: Mailbox,
    commands: Sender<Command>,
    stopping: Arc<AtomicBool>,
    closer: Arc<Mutex<Watch>>,
}

impl Idle {
    fn pause(&self, length: Duration) {
        let until = Instant::now() + length;
        while let Some(left) = until.checked_duration_since(Instant::now())
            && !self.stopping.load(Ordering::Relaxed)
        {
            thread::park_timeout(left);
        }
    }

    fn run(self) {
        if let Ok(mut watch) = self.closer.lock() {
            watch.thread = Some(thread::current());
        }
        let mut retry = FIRST_RETRY;
        while !self.stopping.load(Ordering::Relaxed) {
            let watched =
                connect(&self.account, &self.password, &self.network).and_then(|mut session| {
                    session.select(&self.inbox.path)?;
                    if let Ok(mut watch) = self.closer.lock() {
                        watch.closer = session.closer().ok();
                    }
                    if self.stopping.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    loop {
                        if session.idle(IDLE_RENEW)? {
                            retry = FIRST_RETRY;
                            if self.commands.send(Command::Refresh(self.inbox.id)).is_err() {
                                return Ok(());
                            }
                        }
                        if self.stopping.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                    }
                });
            match watched {
                Ok(()) => return,
                Err(Error::Login(_)) => return,
                Err(error) => {
                    tracing::debug!("{} stopped watching: {error}", self.account.email);
                    self.pause(retry);
                    retry = (retry * 2).min(LAST_RETRY);
                }
            }
        }
    }
}
