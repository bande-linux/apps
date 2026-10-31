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
    smtp_security TEXT NOT NULL
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
    UNIQUE (mailbox, uid)
);
CREATE INDEX IF NOT EXISTS messages_by_date ON messages (mailbox, date DESC);
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
    })
}

const SUMMARY_COLUMNS: &str = "m.id, m.mailbox, b.account, m.uid, m.subject, m.sender_name, \
    m.sender_email, m.date, m.preview, m.seen, m.flagged, m.answered, m.deleted, m.draft, \
    m.attachments, m.body IS NOT NULL";

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
        },
        attachments: row.get(14)?,
        downloaded: row.get(15)?,
    })
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

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection })
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, email, user, imap_host, imap_port, imap_security, \
             smtp_host, smtp_port, smtp_security FROM accounts ORDER BY id",
        )?;
        let accounts = statement
            .query_map([], account_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(accounts)
    }

    pub fn account(&self, id: AccountId) -> Result<Option<Account>> {
        Ok(self.accounts()?.into_iter().find(|account| account.id == id))
    }

    pub fn add_account(&self, account: &Account) -> Result<AccountId> {
        self.connection.execute(
            "INSERT INTO accounts (name, email, user, imap_host, imap_port, imap_security, \
             smtp_host, smtp_port, smtp_security) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
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
            let mut statement = transaction
                .prepare("SELECT id, path FROM mailboxes WHERE account = ?1")?;
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
            let date = envelope
                .date
                .or(message.internal_date)
                .unwrap_or_default();
            transaction.execute(
                "INSERT INTO messages (mailbox, uid, message_id, subject, sender_name, sender_email, \
                 recipients, date, size, seen, flagged, answered, deleted, draft, header) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
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
                "UPDATE messages SET seen = ?3, flagged = ?4, answered = ?5, deleted = ?6, draft = ?7 \
                 WHERE mailbox = ?1 AND uid = ?2 AND (seen, flagged, answered, deleted, draft) \
                 IS NOT (?3, ?4, ?5, ?6, ?7)",
                params![
                    mailbox,
                    message.uid,
                    flags.seen,
                    flags.flagged,
                    flags.answered,
                    flags.deleted,
                    flags.draft
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
        let (filter, scope_value): (&str, Box<dyn rusqlite::ToSql>) = match scope {
            Scope::Mailbox(id) => ("m.mailbox = ?1", Box::new(*id)),
            Scope::AllOf(role) => ("b.role = ?1", Box::new(role.name())),
            Scope::Flagged => ("m.flagged = ?1", Box::new(true)),
        };
        let searching = !search.trim().is_empty();
        let search_filter = match searching {
            true => {
                " AND (m.subject LIKE ?2 ESCAPE '\\' OR m.sender_name LIKE ?2 ESCAPE '\\' \
                 OR m.sender_email LIKE ?2 ESCAPE '\\' OR m.recipients LIKE ?2 ESCAPE '\\' \
                 OR m.text LIKE ?2 ESCAPE '\\')"
            }
            false => "",
        };
        let query = format!(
            "SELECT {SUMMARY_COLUMNS} FROM messages m JOIN mailboxes b ON b.id = m.mailbox \
             WHERE {filter} AND m.deleted = 0{search_filter} ORDER BY m.date DESC, m.id DESC"
        );
        let mut statement = self.connection.prepare(&query)?;
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = vec![scope_value];
        if searching {
            values.push(Box::new(like_pattern(search)));
        }
        let summaries = statement
            .query_map(params_from_iter(values.iter()), summary_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(summaries)
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
        let column = match flag {
            Flag::Seen => "seen",
            Flag::Flagged => "flagged",
            Flag::Answered => "answered",
            Flag::Deleted => "deleted",
        };
        let accounts = self.queue(ids, &Action::Flag(flag, value))?;
        for id in ids {
            self.connection.execute(
                &format!("UPDATE messages SET {column} = ?2 WHERE id = ?1"),
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

    pub fn finish_pending(&self, id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM pending WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn unread(&self, scope: &Scope) -> Result<usize> {
        Ok(self
            .summaries(scope, "")?
            .iter()
            .filter(|summary| !summary.flags.seen)
            .count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        let server = |host: &str, port| Server {
            host: host.to_owned(),
            port,
            security: Security::Tls,
        };
        Account {
            id: 0,
            name: "Ann".to_owned(),
            email: "ann@example.com".to_owned(),
            user: "ann".to_owned(),
            imap: server("imap.example.com", 993),
            smtp: server("smtp.example.com", 465),
        }
    }

    fn folder(path: &str, role: Option<Role>) -> Folder {
        Folder {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            delimiter: Some('/'),
            role,
            selectable: true,
        }
    }

    fn header(uid: u32, subject: &str, seen: bool) -> Fetched {
        Fetched {
            uid,
            flags: Some(Flags {
                seen,
                ..Flags::default()
            }),
            size: Some(100),
            internal_date: Some(1_000 + i64::from(uid)),
            header: Some(
                format!("Subject: {subject}\r\nFrom: Bob <bob@example.com>\r\n\r\n").into_bytes(),
            ),
            body: None,
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let folder = tempfile::tempdir().unwrap_or_else(|error| unreachable!("{error}"));
        let store = Store::open(&folder.path().join("mail.sqlite3"))
            .unwrap_or_else(|error| unreachable!("{error}"));
        (folder, store)
    }

    #[test]
    fn mail_moves_through_the_store_and_queues_its_changes() -> Result<()> {
        let (_folder, mut store) = store();
        let id = store.add_account(&account())?;
        assert_eq!(store.accounts()?[0].imap.host, "imap.example.com");
        store.replace_mailboxes(
            id,
            &[
                folder("INBOX", Some(Role::Inbox)),
                folder("Archive", Some(Role::Archive)),
            ],
        )?;
        let inbox = store.mailbox_with_role(id, Role::Inbox)?.map(|inbox| inbox.id);
        let Some(inbox) = inbox else {
            unreachable!("the inbox was added")
        };
        store.set_mailbox_state(inbox, 7, Some(4))?;
        store.insert_headers(
            inbox,
            &[header(1, "First", true), header(2, "Second", false), header(3, "Third 100%", false)],
        )?;
        let listed = store.summaries(&Scope::AllOf(Role::Inbox), "")?;
        assert_eq!(
            listed.iter().map(|summary| summary.subject.as_str()).collect::<Vec<_>>(),
            ["Third 100%", "Second", "First"]
        );
        assert_eq!(store.mailboxes(id)?[1].unread, 2);
        assert_eq!(store.summaries(&Scope::Mailbox(inbox), "100%")?.len(), 1);
        assert_eq!(store.summaries(&Scope::Mailbox(inbox), "_")?.len(), 0);
        assert_eq!(store.missing_bodies(inbox, 1000, 10)?, [3, 2, 1]);
        store.store_body(inbox, 2, b"Subject: Second\r\n\r\nHello body\r\n")?;
        assert_eq!(store.summaries(&Scope::Mailbox(inbox), "hello")?[0].preview, "Hello body");

        store.set_flag(&[listed[1].id], Flag::Seen, true)?;
        assert_eq!(store.unread(&Scope::Mailbox(inbox))?, 1);
        let archive = store.mailbox_with_role(id, Role::Archive)?;
        let Some(archive) = archive else {
            unreachable!("the archive was added")
        };
        store.move_messages(&[listed[0].id], &archive)?;
        assert_eq!(store.known_uids(inbox)?, [1, 2]);
        let pending = store.pending(id)?;
        assert_eq!(
            pending.iter().map(|pending| (&pending.action, &pending.uids)).collect::<Vec<_>>(),
            [
                (&Action::Flag(Flag::Seen, true), &vec![2]),
                (&Action::Move("Archive".to_owned()), &vec![3]),
            ]
        );
        assert!(pending.iter().all(|pending| pending.uid_validity == 7));
        store.finish_pending(pending[0].id)?;
        assert_eq!(store.pending(id)?.len(), 1);

        store.set_mailbox_state(inbox, 8, None)?;
        assert!(store.known_uids(inbox)?.is_empty());
        store.replace_mailboxes(id, &[folder("INBOX", Some(Role::Inbox))])?;
        assert_eq!(store.mailboxes(id)?.len(), 1);
        store.remove_account(id)?;
        assert!(store.accounts()?.is_empty());
        assert!(store.pending(id)?.is_empty());
        Ok(())
    }

    #[test]
    fn flags_from_the_server_only_count_real_changes() -> Result<()> {
        let (_folder, mut store) = store();
        let id = store.add_account(&account())?;
        store.replace_mailboxes(id, &[folder("INBOX", Some(Role::Inbox))])?;
        let inbox = store.mailboxes(id)?[0].id;
        store.insert_headers(inbox, &[header(1, "a", false), header(2, "b", true)])?;
        let changed = store.update_flags(inbox, &[header(1, "a", true), header(2, "b", true)])?;
        assert_eq!(changed, 1);
        assert_eq!(store.unread(&Scope::Mailbox(inbox))?, 0);
        store.remove_uids(inbox, &[1])?;
        assert_eq!(store.known_uids(inbox)?, [2]);
        Ok(())
    }
}
