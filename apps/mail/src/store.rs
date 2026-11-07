use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::error::Result;
use crate::imap::{Fetched, Flag, Flags, Folder, Role};
use crate::message::{Address, Content, Envelope};
use crate::net::{Security, Server};

pub type AccountId = i64;
pub type MailboxId = i64;
pub type MessageId = i64;

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    user TEXT NOT NULL,
    imap_host TEXT NOT NULL,
    imap_port INTEGER NOT NULL,
    imap_security TEXT NOT NULL,
    smtp_host TEXT NOT NULL,
    smtp_port INTEGER NOT NULL,
    smtp_security TEXT NOT NULL,
    signature TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS mailboxes (
    id INTEGER PRIMARY KEY,
    account INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    delimiter TEXT,
    role TEXT,
    selectable INTEGER NOT NULL,
    uid_validity INTEGER,
    uid_next INTEGER,
    UNIQUE (account, path)
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    mailbox INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL,
    message_id TEXT,
    subject TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    sender_email TEXT NOT NULL,
    recipients TEXT NOT NULL,
    date INTEGER NOT NULL,
    size INTEGER NOT NULL,
    seen INTEGER NOT NULL,
    flagged INTEGER NOT NULL,
    answered INTEGER NOT NULL,
    deleted INTEGER NOT NULL,
    draft INTEGER NOT NULL,
    preview TEXT NOT NULL DEFAULT '',
    text TEXT NOT NULL DEFAULT '',
    attachments INTEGER NOT NULL DEFAULT 0,
    header BLOB NOT NULL,
    body BLOB,
    thread TEXT,
    color INTEGER NOT NULL DEFAULT 0,
    UNIQUE (mailbox, uid)
);
CREATE INDEX IF NOT EXISTS messages_by_date ON messages (mailbox, date DESC);
CREATE INDEX IF NOT EXISTS messages_by_thread ON messages (thread);
CREATE TABLE IF NOT EXISTS search_outbox (
    change_id INTEGER PRIMARY KEY AUTOINCREMENT,
    message INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS search_added AFTER INSERT ON messages BEGIN
    INSERT INTO search_outbox (message) VALUES (new.id);
END;
CREATE TRIGGER IF NOT EXISTS search_changed AFTER UPDATE OF subject, sender_name, text ON messages BEGIN
    INSERT INTO search_outbox (message) VALUES (new.id);
END;
CREATE TRIGGER IF NOT EXISTS search_removed AFTER DELETE ON messages BEGIN
    INSERT INTO search_outbox (message) VALUES (old.id);
END;
CREATE TABLE IF NOT EXISTS rules (
    id INTEGER PRIMARY KEY,
    every INTEGER NOT NULL DEFAULT 1,
    action TEXT NOT NULL,
    mailbox INTEGER REFERENCES mailboxes(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS rule_conditions (
    id INTEGER PRIMARY KEY,
    rule INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    pattern TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS remote_senders (
    email TEXT PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS pending (
    id INTEGER PRIMARY KEY,
    account INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    uids TEXT NOT NULL,
    action TEXT NOT NULL,
    flag TEXT,
    value INTEGER,
    target TEXT
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    pub user: String,
    pub imap: Server,
    pub smtp: Server,
    pub signature: String,
    pub enabled: bool,
}

impl Account {
    pub fn address(&self) -> Address {
        Address {
            name: self.name.clone(),
            email: self.email.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub id: MailboxId,
    pub account: AccountId,
    pub path: String,
    pub name: String,
    pub delimiter: Option<char>,
    pub role: Option<Role>,
    pub selectable: bool,
    pub unread: u32,
}

impl Mailbox {
    pub fn depth(&self) -> usize {
        self.delimiter
            .map_or(0, |delimiter| self.path.matches(delimiter).count())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchContent {
    pub subject: String,
    pub sender_name: String,
    pub sender_email: String,
    pub recipients: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchChange {
    pub change: i64,
    pub message: MessageId,
    pub content: Option<SearchContent>,
}

pub struct Summary {
    pub id: MessageId,
    pub mailbox: MailboxId,
    pub account: AccountId,
    pub uid: u32,
    pub subject: String,
    pub sender: Address,
    pub date: i64,
    pub preview: String,
    pub flags: Flags,
    pub attachments: bool,
    pub downloaded: bool,
    pub thread: Option<String>,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Mailbox(MailboxId),
    AllOf(Role),
    Flagged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Flag(Flag, bool),
    Move(String),
    Expunge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleField {
    From,
    To,
    Subject,
}

impl RuleField {
    pub const ALL: [Self; 3] = [Self::From, Self::To, Self::Subject];

    pub fn name(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::To => "to",
            Self::Subject => "subject",
        }
    }

    fn columns(self, slot: usize) -> String {
        match self {
            Self::From => format!(
                "(sender_name LIKE ?{slot} ESCAPE '\\' OR sender_email LIKE ?{slot} ESCAPE '\\')"
            ),
            Self::To => format!("recipients LIKE ?{slot} ESCAPE '\\'"),
            Self::Subject => format!("subject LIKE ?{slot} ESCAPE '\\'"),
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleAction {
    Move(MailboxId),
    Flag,
    Read,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: i64,
    pub every: bool,
    pub conditions: Vec<(RuleField, String)>,
    pub action: RuleAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    pub mailbox: String,
    pub uid_validity: u32,
    pub uids: Vec<u32>,
    pub action: Action,
}

pub struct Store {
    connection: Connection,
}

fn security(name: String) -> Security {
    Security::from_name(&name).unwrap_or(Security::Tls)
}

fn account_from(row: &Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
        user: row.get(3)?,
        imap: Server {
            host: row.get(4)?,
            port: row.get(5)?,
            security: security(row.get(6)?),
        },
        smtp: Server {
            host: row.get(7)?,
            port: row.get(8)?,
            security: security(row.get(9)?),
        },
        signature: row.get(10)?,
        enabled: row.get(11)?,
    })
}

const SUMMARY_COLUMNS: &str = "m.id, m.mailbox, b.account, m.uid, m.subject, m.sender_name, \
    m.sender_email, m.date, m.preview, m.seen, m.flagged, m.answered, m.deleted, m.draft, \
    m.attachments, m.body IS NOT NULL, m.thread, m.size, m.color";

fn summary_from(row: &Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: row.get(0)?,
        mailbox: row.get(1)?,
        account: row.get(2)?,
        uid: row.get(3)?,
        subject: row.get(4)?,
        sender: Address {
            name: row.get(5)?,
            email: row.get(6)?,
        },
        date: row.get(7)?,
        preview: row.get(8)?,
        flags: Flags {
            seen: row.get(9)?,
            flagged: row.get(10)?,
            answered: row.get(11)?,
            deleted: row.get(12)?,
            draft: row.get(13)?,
            color: row.get(18)?,
        },
        attachments: row.get(14)?,
        downloaded: row.get(15)?,
        thread: row.get(16)?,
        size: row.get(17)?,
    })
}

const ENABLED: &str = " AND b.account IN (SELECT id FROM accounts WHERE enabled = 1)";

fn thread_of(envelope: &Envelope) -> Option<String> {
    envelope
        .references
        .first()
        .or(envelope.message_id.as_ref())
        .cloned()
}

const REPLY_PREFIXES: [&str; 13] = [
    "re", "fw", "fwd", "aw", "sv", "vs", "wg", "tr", "rif", "antw", "odp", "res", "r",
];
const TOPIC_WINDOW: i64 = 30 * 24 * 60 * 60;

fn topic(subject: &str) -> Option<String> {
    let mut rest = subject.trim();
    let mut prefixed = false;
    while let Some((token, after)) = rest.split_once(':') {
        let word = token
            .trim_end_matches(|character: char| {
                character.is_ascii_digit() || matches!(character, '[' | ']' | '(' | ')' | ' ')
            })
            .to_lowercase();
        if !REPLY_PREFIXES.contains(&word.as_str()) {
            break;
        }
        prefixed = true;
        rest = after.trim_start();
    }
    let rest = rest.trim().to_lowercase();
    (prefixed && !rest.is_empty()).then_some(rest)
}

fn thread_by_topic(
    transaction: &rusqlite::Transaction<'_>,
    mailbox: MailboxId,
    envelope: &Envelope,
    date: i64,
) -> Result<Option<String>> {
    let Some(wanted) = topic(&envelope.subject).filter(|_| envelope.references.is_empty()) else {
        return Ok(None);
    };
    let mut statement = transaction.prepare(
        "SELECT m.subject, m.thread FROM messages m JOIN mailboxes b ON b.id = m.mailbox \
         WHERE b.account = (SELECT account FROM mailboxes WHERE id = ?1) \
         AND m.thread IS NOT NULL AND m.date BETWEEN ?2 AND ?3 \
         AND m.subject LIKE ?4 ESCAPE '\\' ORDER BY m.date DESC LIMIT 50",
    )?;
    let candidates = statement
        .query_map(
            params![mailbox, date - TOPIC_WINDOW, date, like_pattern(&wanted)],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(candidates
        .into_iter()
        .find(|(subject, _)| {
            let subject = subject.trim().to_lowercase();
            subject == wanted || topic(&subject).as_deref() == Some(wanted.as_str())
        })
        .map(|(_, thread)| thread))
}

fn search_terms(search: &str) -> Vec<(Option<RuleField>, String)> {
    let mut terms = Vec::new();
    let mut rest = search.trim();
    while !rest.is_empty() {
        let (field, after) = match rest.split_once(':') {
            Some((name, after))
                if !name.contains(char::is_whitespace)
                    && RuleField::named(&name.to_lowercase()).is_some() =>
            {
                (RuleField::named(&name.to_lowercase()), after.trim_start())
            }
            _ => (None, rest),
        };
        let (term, remaining) = match after.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
            None => after.split_once(char::is_whitespace).unwrap_or((after, "")),
        };
        if !term.trim().is_empty() {
            terms.push((field, term.to_owned()));
        }
        rest = remaining.trim_start();
    }
    terms
}

fn like_pattern(search: &str) -> String {
    let escaped = search
        .trim()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

fn joined(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn scope_filter(scope: &Scope) -> (&'static str, Box<dyn rusqlite::ToSql>) {
    match scope {
        Scope::Mailbox(id) => ("m.mailbox = ?1", Box::new(*id)),
        Scope::AllOf(role) => ("b.role = ?1", Box::new(role.name())),
        Scope::Flagged => ("m.flagged = ?1", Box::new(true)),
    }
}

impl Store {
    pub fn search_changes(&self, limit: usize) -> Result<Vec<SearchChange>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT o.change_id, o.message, m.subject, m.sender_name, m.sender_email, \
             m.recipients, m.text FROM search_outbox o LEFT JOIN messages m ON m.id = o.message \
             ORDER BY o.change_id LIMIT ?1",
        )?;
        statement
            .query_map(params![limit as i64], |row| {
                Ok(SearchChange {
                    change: row.get(0)?,
                    message: row.get(1)?,
                    content: match row.get::<_, Option<String>>(2)? {
                        Some(subject) => Some(SearchContent {
                            subject,
                            sender_name: row.get(3)?,
                            sender_email: row.get(4)?,
                            recipients: row.get(5)?,
                            text: row.get(6)?,
                        }),
                        None => None,
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()
            .map_err(Into::into)
    }

    pub fn search_changes_done(&self, last_change: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM search_outbox WHERE change_id <= ?1",
            params![last_change],
        )?;
        Ok(())
    }

    pub fn search_offer_everything(&self) -> Result<()> {
        self.connection.execute(
            "INSERT INTO search_outbox (message) SELECT id FROM messages",
            [],
        )?;
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection })
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, email, user, imap_host, imap_port, imap_security, \
             smtp_host, smtp_port, smtp_security, signature, enabled FROM accounts ORDER BY id",
        )?;
        let accounts = statement
            .query_map([], account_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(accounts)
    }

    pub fn account(&self, id: AccountId) -> Result<Option<Account>> {
        Ok(self
            .accounts()?
            .into_iter()
            .find(|account| account.id == id))
    }

    pub fn add_account(&self, account: &Account) -> Result<AccountId> {
        self.connection.execute(
            "INSERT INTO accounts (name, email, user, imap_host, imap_port, imap_security, \
             smtp_host, smtp_port, smtp_security, signature) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                account.name,
                account.email,
                account.user,
                account.imap.host,
                account.imap.port,
                account.imap.security.name(),
                account.smtp.host,
                account.smtp.port,
                account.smtp.security.name(),
                account.signature,
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn update_account(&self, account: &Account) -> Result<()> {
        self.connection.execute(
            "UPDATE accounts SET name = ?2, signature = ?3, enabled = ?4 WHERE id = ?1",
            params![account.id, account.name, account.signature, account.enabled],
        )?;
        Ok(())
    }

    pub fn update_servers(&self, id: AccountId, servers: &crate::discover::Settings) -> Result<()> {
        self.connection.execute(
            "UPDATE accounts SET user = ?2, imap_host = ?3, imap_port = ?4, imap_security = ?5, \
             smtp_host = ?6, smtp_port = ?7, smtp_security = ?8 WHERE id = ?1",
            params![
                id,
                servers.user,
                servers.imap.host,
                servers.imap.port,
                servers.imap.security.name(),
                servers.smtp.host,
                servers.smtp.port,
                servers.smtp.security.name(),
            ],
        )?;
        Ok(())
    }

    pub fn rename_mailbox(
        &self,
        account: AccountId,
        from: &str,
        to: &str,
        delimiter: Option<char>,
    ) -> Result<()> {
        let escaped = from
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let children = delimiter.map(|delimiter| format!("{escaped}{delimiter}%"));
        self.connection.execute(
            "UPDATE mailboxes SET path = ?3 || substr(path, length(?2) + 1) \
             WHERE account = ?1 AND (path = ?2 OR path LIKE ?4 ESCAPE '\\')",
            params![account, from, to, children],
        )?;
        Ok(())
    }

    pub fn remove_account(&self, id: AccountId) -> Result<()> {
        self.connection
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn mailboxes(&self, account: AccountId) -> Result<Vec<Mailbox>> {
        let mut statement = self.connection.prepare(
            "SELECT b.id, b.account, b.path, b.name, b.delimiter, b.role, b.selectable, \
             (SELECT COUNT(*) FROM messages m WHERE m.mailbox = b.id AND m.seen = 0 AND m.deleted = 0) \
             FROM mailboxes b WHERE b.account = ?1 ORDER BY b.path",
        )?;
        let mailboxes = statement
            .query_map([account], |row| {
                Ok(Mailbox {
                    id: row.get(0)?,
                    account: row.get(1)?,
                    path: row.get(2)?,
                    name: row.get(3)?,
                    delimiter: row
                        .get::<_, Option<String>>(4)?
                        .and_then(|delimiter| delimiter.chars().next()),
                    role: row
                        .get::<_, Option<String>>(5)?
                        .as_deref()
                        .and_then(Role::from_name),
                    selectable: row.get(6)?,
                    unread: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(mailboxes)
    }

    pub fn mailbox(&self, id: MailboxId) -> Result<Option<Mailbox>> {
        let account: Option<AccountId> = self
            .connection
            .query_row("SELECT account FROM mailboxes WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(account) = account else {
            return Ok(None);
        };
        Ok(self
            .mailboxes(account)?
            .into_iter()
            .find(|mailbox| mailbox.id == id))
    }

    pub fn mailbox_with_role(&self, account: AccountId, role: Role) -> Result<Option<Mailbox>> {
        Ok(self
            .mailboxes(account)?
            .into_iter()
            .find(|mailbox| mailbox.role == Some(role)))
    }

    pub fn replace_mailboxes(&mut self, account: AccountId, folders: &[Folder]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for folder in folders {
            transaction.execute(
                "INSERT INTO mailboxes (account, path, name, delimiter, role, selectable) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (account, path) DO UPDATE SET \
                 name = excluded.name, delimiter = excluded.delimiter, role = excluded.role, \
                 selectable = excluded.selectable",
                params![
                    account,
                    folder.path,
                    folder.name,
                    folder.delimiter.map(String::from),
                    folder.role.map(Role::name),
                    folder.selectable,
                ],
            )?;
        }
        let kept = folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect::<Vec<_>>();
        let known: Vec<(MailboxId, String)> = {
            let mut statement =
                transaction.prepare("SELECT id, path FROM mailboxes WHERE account = ?1")?;
            statement
                .query_map([account], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        for (id, path) in known {
            if !kept.contains(&path) {
                transaction.execute("DELETE FROM mailboxes WHERE id = ?1", [id])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn mailbox_state(&self, mailbox: MailboxId) -> Result<(Option<u32>, Option<u32>)> {
        Ok(self.connection.query_row(
            "SELECT uid_validity, uid_next FROM mailboxes WHERE id = ?1",
            [mailbox],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    pub fn set_mailbox_state(
        &self,
        mailbox: MailboxId,
        uid_validity: u32,
        uid_next: Option<u32>,
    ) -> Result<()> {
        let (known, _) = self.mailbox_state(mailbox)?;
        if known.is_some_and(|known| known != uid_validity) {
            self.connection
                .execute("DELETE FROM messages WHERE mailbox = ?1", [mailbox])?;
        }
        self.connection.execute(
            "UPDATE mailboxes SET uid_validity = ?2, uid_next = ?3 WHERE id = ?1",
            params![mailbox, uid_validity, uid_next],
        )?;
        Ok(())
    }

    pub fn known_uids(&self, mailbox: MailboxId) -> Result<Vec<u32>> {
        let mut statement = self
            .connection
            .prepare("SELECT uid FROM messages WHERE mailbox = ?1 ORDER BY uid")?;
        let uids = statement
            .query_map([mailbox], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(uids)
    }

    pub fn insert_headers(&mut self, mailbox: MailboxId, fetched: &[Fetched]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for message in fetched {
            let header = message.header.clone().unwrap_or_default();
            let envelope = Envelope::parse(&header);
            let sender = envelope.sender();
            let recipients = envelope
                .to
                .iter()
                .chain(&envelope.cc)
                .map(Address::full)
                .collect::<Vec<_>>()
                .join(", ");
            let flags = message.flags.unwrap_or_default();
            let date = envelope.date.or(message.internal_date).unwrap_or_default();
            let thread = match thread_by_topic(&transaction, mailbox, &envelope, date)? {
                Some(thread) => Some(thread),
                None => thread_of(&envelope),
            };
            transaction.execute(
                "INSERT INTO messages (mailbox, uid, message_id, subject, sender_name, sender_email, \
                 recipients, date, size, seen, flagged, answered, deleted, draft, header, thread, \
                 color) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
                 ON CONFLICT (mailbox, uid) DO NOTHING",
                params![
                    mailbox,
                    message.uid,
                    envelope.message_id,
                    envelope.subject,
                    sender.name,
                    sender.email,
                    recipients,
                    date,
                    message
                        .size
                        .and_then(|size| i64::try_from(size).ok())
                        .unwrap_or_default(),
                    flags.seen,
                    flags.flagged,
                    flags.answered,
                    flags.deleted,
                    flags.draft,
                    header,
                    thread,
                    flags.color,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn update_flags(&mut self, mailbox: MailboxId, fetched: &[Fetched]) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let mut changed = 0;
        for message in fetched {
            let Some(flags) = message.flags else {
                continue;
            };
            changed += transaction.execute(
                "UPDATE messages SET seen = ?3, flagged = ?4, answered = ?5, deleted = ?6, \
                 draft = ?7, color = ?8 WHERE mailbox = ?1 AND uid = ?2 \
                 AND (seen, flagged, answered, deleted, draft, color) \
                 IS NOT (?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    mailbox,
                    message.uid,
                    flags.seen,
                    flags.flagged,
                    flags.answered,
                    flags.deleted,
                    flags.draft,
                    flags.color
                ],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn remove_uids(&mut self, mailbox: MailboxId, uids: &[u32]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for uid in uids {
            transaction.execute(
                "DELETE FROM messages WHERE mailbox = ?1 AND uid = ?2",
                params![mailbox, uid],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn missing_bodies(
        &self,
        mailbox: MailboxId,
        max_size: i64,
        limit: i64,
    ) -> Result<Vec<u32>> {
        let mut statement = self.connection.prepare(
            "SELECT uid FROM messages WHERE mailbox = ?1 AND body IS NULL AND size <= ?2 \
             ORDER BY date DESC LIMIT ?3",
        )?;
        let uids = statement
            .query_map(params![mailbox, max_size, limit], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(uids)
    }

    pub fn store_body(&self, mailbox: MailboxId, uid: u32, raw: &[u8]) -> Result<()> {
        let content = Content::parse(raw);
        let text = content.text();
        self.connection.execute(
            "UPDATE messages SET body = ?3, preview = ?4, text = ?5, attachments = ?6 \
             WHERE mailbox = ?1 AND uid = ?2",
            params![
                mailbox,
                uid,
                raw,
                crate::message::preview(&text),
                text,
                !content.attachments.is_empty()
            ],
        )?;
        Ok(())
    }

    pub fn summaries(&self, scope: &Scope, search: &str) -> Result<Vec<Summary>> {
        let (filter, scope_value) = scope_filter(scope);
        let terms = search_terms(search);
        let search_filter = terms
            .iter()
            .enumerate()
            .map(|(index, (field, _))| {
                let slot = index + 2;
                match field {
                    Some(field) => format!(" AND {}", field.columns(slot)),
                    None => format!(
                        " AND (m.subject LIKE ?{slot} ESCAPE '\\' \
                         OR m.sender_name LIKE ?{slot} ESCAPE '\\' \
                         OR m.sender_email LIKE ?{slot} ESCAPE '\\' \
                         OR m.recipients LIKE ?{slot} ESCAPE '\\' \
                         OR m.text LIKE ?{slot} ESCAPE '\\')"
                    ),
                }
            })
            .collect::<String>();
        let query = format!(
            "SELECT {SUMMARY_COLUMNS} FROM messages m JOIN mailboxes b ON b.id = m.mailbox \
             WHERE {filter} AND m.deleted = 0{search_filter}{ENABLED} ORDER BY m.date DESC, m.id DESC"
        );
        let mut statement = self.connection.prepare(&query)?;
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = vec![scope_value];
        values.extend(
            terms
                .iter()
                .map(|(_, term)| Box::new(like_pattern(term)) as Box<dyn rusqlite::ToSql>),
        );
        let summaries = statement
            .query_map(params_from_iter(values.iter()), summary_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(summaries)
    }

    pub fn conversation(&self, of: &Summary) -> Result<Vec<Summary>> {
        let Some(key) = &of.thread else {
            return Ok(vec![of.clone()]);
        };
        let query = format!(
            "SELECT {SUMMARY_COLUMNS}, m.message_id FROM messages m \
             JOIN mailboxes b ON b.id = m.mailbox \
             WHERE b.account = ?1 AND m.thread = ?2 AND m.deleted = 0 \
             AND (m.mailbox = ?3 OR b.role IS NULL OR b.role NOT IN ('trash', 'junk')) \
             ORDER BY m.id = ?4 DESC, m.mailbox = ?3 DESC, b.role = 'all', m.id"
        );
        let mut statement = self.connection.prepare(&query)?;
        let found = statement
            .query_map(params![of.account, key, of.mailbox, of.id], |row| {
                Ok((summary_from(row)?, row.get::<_, Option<String>>(19)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut seen = std::collections::HashSet::new();
        let mut messages = found
            .into_iter()
            .filter(|(_, message_id)| {
                message_id
                    .as_ref()
                    .is_none_or(|message_id| seen.insert(message_id.clone()))
            })
            .map(|(summary, _)| summary)
            .collect::<Vec<_>>();
        messages.sort_by_key(|summary| (summary.date, summary.id));
        Ok(messages)
    }

    pub fn summary(&self, id: MessageId) -> Result<Option<Summary>> {
        let query = format!(
            "SELECT {SUMMARY_COLUMNS} FROM messages m JOIN mailboxes b ON b.id = m.mailbox WHERE m.id = ?1"
        );
        Ok(self
            .connection
            .query_row(&query, [id], summary_from)
            .optional()?)
    }

    pub fn body(&self, id: MessageId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .connection
            .query_row("SELECT body FROM messages WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .flatten())
    }

    pub fn header(&self, id: MessageId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .connection
            .query_row("SELECT header FROM messages WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn message_id(&self, id: MessageId) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT message_id FROM messages WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn locate(
        &self,
        account: AccountId,
        message_id: &str,
        role: Option<Role>,
    ) -> Result<Option<MessageId>> {
        Ok(self
            .connection
            .query_row(
                "SELECT m.id FROM messages m JOIN mailboxes b ON b.id = m.mailbox \
                 WHERE b.account = ?1 AND m.message_id = ?2 AND m.deleted = 0 \
                 AND (?3 IS NULL OR b.role = ?3) ORDER BY m.id DESC LIMIT 1",
                params![account, message_id, role.map(Role::name)],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn find_message(&self, mailbox: MailboxId, message_id: &str) -> Result<Option<MessageId>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM messages WHERE mailbox = ?1 AND message_id = ?2 \
                 ORDER BY id DESC LIMIT 1",
                params![mailbox, message_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn located(&self, ids: &[MessageId]) -> Result<Vec<(Mailbox, u32, u32, MessageId)>> {
        let mut found = Vec::new();
        for id in ids {
            let row: Option<(MailboxId, u32, Option<u32>)> = self
                .connection
                .query_row(
                    "SELECT m.mailbox, m.uid, b.uid_validity FROM messages m \
                     JOIN mailboxes b ON b.id = m.mailbox WHERE m.id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((mailbox, uid, Some(validity))) = row
                && let Some(mailbox) = self.mailbox(mailbox)?
            {
                found.push((mailbox, uid, validity, *id));
            }
        }
        Ok(found)
    }

    fn queue(&self, ids: &[MessageId], action: &Action) -> Result<Vec<AccountId>> {
        let mut groups: Vec<(Mailbox, u32, Vec<u32>)> = Vec::new();
        for (mailbox, uid, validity, _) in self.located(ids)? {
            match groups.iter_mut().find(|group| group.0.id == mailbox.id) {
                Some(group) => group.2.push(uid),
                None => groups.push((mailbox, validity, vec![uid])),
            }
        }
        let (kind, flag, value, target) = match action {
            Action::Flag(flag, value) => ("flag", Some(flag.name()), Some(*value), None),
            Action::Move(target) => ("move", None, None, Some(target.as_str())),
            Action::Expunge => ("expunge", None, None, None),
        };
        let mut accounts = Vec::new();
        for (mailbox, validity, uids) in groups {
            self.connection.execute(
                "INSERT INTO pending (account, mailbox, uid_validity, uids, action, flag, value, target) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    mailbox.account,
                    mailbox.path,
                    validity,
                    joined(&uids),
                    kind,
                    flag,
                    value,
                    target
                ],
            )?;
            if !accounts.contains(&mailbox.account) {
                accounts.push(mailbox.account);
            }
        }
        Ok(accounts)
    }

    pub fn set_flag(&self, ids: &[MessageId], flag: Flag, value: bool) -> Result<Vec<AccountId>> {
        let change = match flag {
            Flag::Seen => "seen = ?2".to_owned(),
            Flag::Flagged => "flagged = ?2".to_owned(),
            Flag::Answered => "answered = ?2".to_owned(),
            Flag::Deleted => "deleted = ?2".to_owned(),
            Flag::Draft => "draft = ?2".to_owned(),
            Flag::Color(bit) => format!("color = (color & ~{0}) | (?2 * {0})", 1 << bit),
        };
        let accounts = self.queue(ids, &Action::Flag(flag, value))?;
        for id in ids {
            self.connection.execute(
                &format!("UPDATE messages SET {change} WHERE id = ?1"),
                params![id, value],
            )?;
        }
        Ok(accounts)
    }

    pub fn move_messages(&self, ids: &[MessageId], target: &Mailbox) -> Result<Vec<AccountId>> {
        let same_account = self
            .located(ids)?
            .into_iter()
            .filter(|(mailbox, ..)| mailbox.account == target.account && mailbox.id != target.id)
            .map(|(.., id)| id)
            .collect::<Vec<_>>();
        let accounts = self.queue(&same_account, &Action::Move(target.path.clone()))?;
        self.forget(&same_account)?;
        Ok(accounts)
    }

    pub fn expunge(&self, ids: &[MessageId]) -> Result<Vec<AccountId>> {
        let accounts = self.queue(ids, &Action::Expunge)?;
        self.forget(ids)?;
        Ok(accounts)
    }

    fn forget(&self, ids: &[MessageId]) -> Result<()> {
        for id in ids {
            self.connection
                .execute("DELETE FROM messages WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    pub fn pending(&self, account: AccountId) -> Result<Vec<Pending>> {
        let mut statement = self.connection.prepare(
            "SELECT id, mailbox, uid_validity, uids, action, flag, value, target FROM pending \
             WHERE account = ?1 ORDER BY id",
        )?;
        let pending = statement
            .query_map([account], |row| {
                let uids: String = row.get(3)?;
                let kind: String = row.get(4)?;
                let flag: Option<String> = row.get(5)?;
                let value: Option<bool> = row.get(6)?;
                let target: Option<String> = row.get(7)?;
                let action = match (kind.as_str(), flag.as_deref().and_then(Flag::from_name)) {
                    ("flag", Some(flag)) => Action::Flag(flag, value.unwrap_or_default()),
                    ("move", _) => Action::Move(target.unwrap_or_default()),
                    _ => Action::Expunge,
                };
                Ok(Pending {
                    id: row.get(0)?,
                    mailbox: row.get(1)?,
                    uid_validity: row.get(2)?,
                    uids: uids.split(',').filter_map(|uid| uid.parse().ok()).collect(),
                    action,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(pending)
    }

    pub fn trust_remote(&self, email: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO remote_senders (email) VALUES (?1)",
            [email.trim().to_lowercase()],
        )?;
        Ok(())
    }

    pub fn trusts_remote(&self, email: &str) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM remote_senders WHERE email = ?1",
                [email.trim().to_lowercase()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn remote_senders(&self) -> Result<u32> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM remote_senders", [], |row| row.get(0))?)
    }

    pub fn forget_remote_senders(&self) -> Result<()> {
        self.connection.execute("DELETE FROM remote_senders", [])?;
        Ok(())
    }

    pub fn rules(&self) -> Result<Vec<Rule>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, every, action, mailbox FROM rules ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<MailboxId>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut conditions = self
            .connection
            .prepare("SELECT field, pattern FROM rule_conditions WHERE rule = ?1 ORDER BY id")?;
        let mut rules = Vec::new();
        for (id, every, action, mailbox) in rows {
            let action = match (action.as_str(), mailbox) {
                ("move", Some(mailbox)) => RuleAction::Move(mailbox),
                ("flag", _) => RuleAction::Flag,
                ("read", _) => RuleAction::Read,
                _ => continue,
            };
            let conditions = conditions
                .query_map([id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter_map(|(field, pattern)| Some((RuleField::named(&field)?, pattern)))
                .collect::<Vec<_>>();
            if !conditions.is_empty() {
                rules.push(Rule {
                    id,
                    every,
                    conditions,
                    action,
                });
            }
        }
        Ok(rules)
    }

    pub fn add_rule(&mut self, rule: &Rule) -> Result<()> {
        self.keep_rule(rule, None)
    }

    pub fn replace_rule(&mut self, rule: &Rule) -> Result<()> {
        self.keep_rule(rule, Some(rule.id))
    }

    fn keep_rule(&mut self, rule: &Rule, replaces: Option<i64>) -> Result<()> {
        let (action, mailbox) = match rule.action {
            RuleAction::Move(mailbox) => ("move", Some(mailbox)),
            RuleAction::Flag => ("flag", None),
            RuleAction::Read => ("read", None),
        };
        let transaction = self.connection.transaction()?;
        let id = match replaces {
            Some(id) => {
                transaction.execute(
                    "UPDATE rules SET every = ?2, action = ?3, mailbox = ?4 WHERE id = ?1",
                    params![id, rule.every, action, mailbox],
                )?;
                transaction.execute("DELETE FROM rule_conditions WHERE rule = ?1", [id])?;
                id
            }
            None => {
                transaction.execute(
                    "INSERT INTO rules (every, action, mailbox) VALUES (?1, ?2, ?3)",
                    params![rule.every, action, mailbox],
                )?;
                transaction.last_insert_rowid()
            }
        };
        for (field, pattern) in &rule.conditions {
            transaction.execute(
                "INSERT INTO rule_conditions (rule, field, pattern) VALUES (?1, ?2, ?3)",
                params![id, field.name(), pattern],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_rule(&self, id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM rules WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn matching(&self, ids: &[MessageId], rule: &Rule) -> Result<Vec<MessageId>> {
        let joiner = match rule.every {
            true => " AND ",
            false => " OR ",
        };
        let query = format!(
            "SELECT id FROM messages WHERE id IN ({}) AND ({})",
            ids.iter()
                .map(MessageId::to_string)
                .collect::<Vec<_>>()
                .join(","),
            rule.conditions
                .iter()
                .enumerate()
                .map(|(index, (field, _))| field.columns(index + 1))
                .collect::<Vec<_>>()
                .join(joiner)
        );
        let mut statement = self.connection.prepare(&query)?;
        let patterns = rule
            .conditions
            .iter()
            .map(|(_, pattern)| like_pattern(pattern))
            .collect::<Vec<_>>();
        let matched = statement
            .query_map(params_from_iter(patterns), |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(matched)
    }

    pub fn finish_pending(&self, id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM pending WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn unread(&self, scope: &Scope) -> Result<usize> {
        let (filter, scope_value) = scope_filter(scope);
        let query = format!(
            "SELECT COUNT(*) FROM messages m JOIN mailboxes b ON b.id = m.mailbox \
             WHERE {filter} AND m.deleted = 0 AND m.seen = 0{ENABLED}"
        );
        let count: i64 = self
            .connection
            .query_row(&query, [scope_value], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or_default())
    }
}
