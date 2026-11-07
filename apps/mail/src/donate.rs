use bande_search::{Donation, IndexProxyBlocking};
use zbus::blocking::Connection;

use crate::error::Result;
use crate::store::{SearchChange, Store};

pub const DOMAIN: &str = "org.bande.Mail";
pub const MESSAGE_ARGUMENT: &str = "message:";
const BATCH: usize = 200;
const CURSOR: &str = "1";

fn donations(change: &SearchChange) -> Vec<Donation> {
    let Some(content) = &change.content else {
        return Vec::new();
    };
    let sender = match content.sender_name.is_empty() {
        true => content.sender_email.clone(),
        false => content.sender_name.clone(),
    };
    let mut items = vec![Donation {
        id: change.message.to_string(),
        kind: "mail".to_owned(),
        title: content.subject.clone(),
        detail: sender.clone(),
        target: format!("{MESSAGE_ARGUMENT}{}", change.message),
        body: format!(
            "{sender} {} {}\n{}",
            content.sender_email, content.recipients, content.text
        ),
    }];
    if !content.sender_email.is_empty() {
        items.push(Donation {
            id: format!("contact:{}", content.sender_email.to_lowercase()),
            kind: "contact".to_owned(),
            title: sender,
            detail: content.sender_email.clone(),
            target: format!("mailto:{}", content.sender_email),
            body: content.sender_email.clone(),
        });
    }
    items
}

fn offer(store: &Store, session: &Connection) -> Result<()> {
    let index = IndexProxyBlocking::new(session)?;
    if index.cursor(DOMAIN)?.is_empty() {
        store.search_offer_everything()?;
    }
    loop {
        let changes = store.search_changes(BATCH)?;
        let Some(last) = changes.last().map(|change| change.change) else {
            return Ok(());
        };
        let (present, removed): (Vec<_>, Vec<_>) = changes
            .iter()
            .partition(|change| change.content.is_some());
        let items = present.iter().flat_map(|change| donations(change)).collect();
        index.donate(DOMAIN, items, CURSOR)?;
        let gone = removed.iter().map(|change| change.message.to_string()).collect();
        index.forget(DOMAIN, gone)?;
        store.search_changes_done(last)?;
    }
}

pub fn offer_to_search(store: &Store) {
    let offered = Connection::session()
        .map_err(Into::into)
        .and_then(|session| offer(store, &session));
    if let Err(error) = offered {
        tracing::debug!("search has not taken the latest mail: {error}");
    }
}

pub fn forget_in_search() {
    let forgotten = Connection::session()
        .and_then(|session| IndexProxyBlocking::new(&session)?.forget_all(DOMAIN));
    if let Err(error) = forgotten {
        tracing::debug!("search has not forgotten the mail: {error}");
    }
}
