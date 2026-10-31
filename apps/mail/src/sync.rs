use crate::error::{Error, Result};
use crate::imap::Session;
use crate::store::{AccountId, Action, Mailbox, Store};

pub const EAGER_BODY_SIZE: i64 = 2 << 20;
const BODY_BATCH: i64 = 20;
const HEADER_BATCH: usize = 250;
const FIRST_SYNC_LIMIT: usize = 5000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub arrived: Vec<u32>,
    pub changed: bool,
}

fn refused_is_done(result: Result<()>) -> Result<()> {
    match result {
        Err(Error::Refused(text)) => {
            tracing::info!("the server refused a queued change, dropping it: {text}");
            Ok(())
        }
        other => other,
    }
}

pub fn flush(session: &mut Session, store: &Store, account: AccountId) -> Result<bool> {
    let pending = store.pending(account)?;
    let flushed = !pending.is_empty();
    for change in pending {
        let selected = match session.select(&change.mailbox) {
            Ok(selected) => Some(selected),
            Err(Error::Refused(_)) => None,
            Err(error) => return Err(error),
        };
        if selected.is_some_and(|selected| selected.uid_validity == change.uid_validity) {
            let applied = match &change.action {
                Action::Flag(flag, value) => session.store(&change.uids, *flag, *value),
                Action::Move(target) => session.move_to(&change.uids, target),
                Action::Expunge => session.expunge(&change.uids),
            };
            refused_is_done(applied)?;
        }
        store.finish_pending(change.id)?;
    }
    Ok(flushed)
}

pub fn sync_folders(session: &mut Session, store: &mut Store, account: AccountId) -> Result<()> {
    let folders = session.list()?;
    if folders.is_empty() {
        return Err(Error::Protocol("the server lists no mailboxes".to_owned()));
    }
    store.replace_mailboxes(account, &folders)
}

pub fn sync_mailbox(session: &mut Session, store: &mut Store, mailbox: &Mailbox) -> Result<Report> {
    let selected = session.select(&mailbox.path)?;
    let (known_validity, known_next) = store.mailbox_state(mailbox.id)?;
    let first = known_validity != Some(selected.uid_validity);
    store.set_mailbox_state(mailbox.id, selected.uid_validity, selected.uid_next)?;
    let mut server = match selected.exists {
        0 => Vec::new(),
        _ => session.uids()?,
    };
    server.sort_unstable();
    let local = store.known_uids(mailbox.id)?;
    let removed = local
        .iter()
        .filter(|uid| server.binary_search(uid).is_err())
        .copied()
        .collect::<Vec<_>>();
    store.remove_uids(mailbox.id, &removed)?;
    let mut added = server
        .iter()
        .filter(|uid| local.binary_search(uid).is_err())
        .copied()
        .collect::<Vec<_>>();
    if first && added.len() > FIRST_SYNC_LIMIT {
        added.drain(..added.len() - FIRST_SYNC_LIMIT);
    }
    for batch in added.rchunks(HEADER_BATCH) {
        let fetched = session.fetch_headers(batch)?;
        store.insert_headers(mailbox.id, &fetched)?;
    }
    let kept = local
        .iter()
        .filter(|uid| server.binary_search(uid).is_ok())
        .copied()
        .collect::<Vec<_>>();
    let mut flags_changed = 0;
    if !kept.is_empty() {
        let flags = session.fetch_flags(&kept)?;
        flags_changed = store.update_flags(mailbox.id, &flags)?;
    }
    let arrived = match (first, known_next) {
        (false, Some(next)) => added.iter().filter(|uid| **uid >= next).copied().collect(),
        _ => Vec::new(),
    };
    Ok(Report {
        changed: !removed.is_empty() || !added.is_empty() || flags_changed > 0,
        arrived,
    })
}

pub fn download_bodies(session: &mut Session, store: &Store, mailbox: &Mailbox) -> Result<usize> {
    let missing = store.missing_bodies(mailbox.id, EAGER_BODY_SIZE, BODY_BATCH)?;
    if missing.is_empty() {
        return Ok(0);
    }
    session.select(&mailbox.path)?;
    let fetched = session.fetch_bodies(&missing)?;
    for message in &fetched {
        if let Some(body) = &message.body {
            store.store_body(mailbox.id, message.uid, body)?;
        }
    }
    Ok(fetched.len())
}

pub fn fetch_body(
    session: &mut Session,
    store: &Store,
    mailbox: &Mailbox,
    uid: u32,
) -> Result<bool> {
    session.select(&mailbox.path)?;
    let fetched = session.fetch_bodies(&[uid])?;
    let body = fetched
        .into_iter()
        .find(|message| message.uid == uid)
        .and_then(|message| message.body);
    match body {
        Some(body) => {
            store.store_body(mailbox.id, uid, &body)?;
            Ok(true)
        }
        None => Ok(false),
    }
}
