#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use bande_mail::donate::{DOMAIN, MESSAGE_ARGUMENT, offer_to_search};
use bande_mail::imap::{Fetched, Folder};
use bande_mail::net::{Security, Server};
use bande_mail::store::{Account, Store};
use bande_search::Donation;
use zbus::blocking::Connection;

#[derive(Default)]
struct Taken {
    items: Vec<Donation>,
    forgotten: Vec<String>,
    cursor: String,
}

struct Index(Arc<Mutex<Taken>>);

#[zbus::interface(name = "org.bande.Search")]
impl Index {
    fn donate(&self, domain: String, items: Vec<Donation>, cursor: String) {
        assert_eq!(domain, DOMAIN);
        let mut taken = self.0.lock().unwrap();
        taken.items.extend(items);
        taken.cursor = cursor;
    }

    fn cursor(&self, _domain: String) -> String {
        self.0.lock().unwrap().cursor.clone()
    }

    fn forget(&self, _domain: String, ids: Vec<String>) {
        self.0.lock().unwrap().forgotten.extend(ids);
    }
}

fn server() -> Server {
    Server {
        host: "localhost".to_owned(),
        port: 1,
        security: Security::Plain,
    }
}

#[test]
fn search_is_told_of_new_mail_its_text_and_its_removal_and_catches_up_after_losing_its_index() {
    let taken = Arc::new(Mutex::new(Taken::default()));
    let service = Connection::session().unwrap();
    service
        .object_server()
        .at("/org/bande/Search", Index(Arc::clone(&taken)))
        .unwrap();
    service.request_name("org.bande.Search").unwrap();

    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("mail.sqlite3")).unwrap();
    let account = store
        .add_account(&Account {
            id: 0,
            name: "Ann".to_owned(),
            email: "ann@example.org".to_owned(),
            user: "ann".to_owned(),
            imap: server(),
            smtp: server(),
            signature: String::new(),
            enabled: true,
        })
        .unwrap();
    store
        .replace_mailboxes(
            account,
            &[Folder {
                path: "INBOX".to_owned(),
                name: "Inbox".to_owned(),
                delimiter: None,
                role: None,
                selectable: true,
            }],
        )
        .unwrap();
    let mailbox = store.mailboxes(account).unwrap()[0].id;
    store
        .insert_headers(
            mailbox,
            &[Fetched {
                uid: 7,
                flags: None,
                size: Some(10),
                internal_date: Some(1),
                header: Some(b"From: Ada <ada@example.org>\r\nSubject: Lunch\r\n\r\n".to_vec()),
                body: None,
            }],
        )
        .unwrap();

    offer_to_search(&store);
    {
        let taken = taken.lock().unwrap();
        let mail = taken.items.iter().find(|item| item.kind == "mail").unwrap();
        assert_eq!(mail.title, "Lunch");
        assert_eq!(mail.detail, "Ada");
        assert!(mail.target.starts_with(MESSAGE_ARGUMENT));
        let contact = taken
            .items
            .iter()
            .find(|item| item.kind == "contact")
            .unwrap();
        assert_eq!(contact.target, "mailto:ada@example.org");
    }

    store
        .store_body(
            mailbox,
            7,
            b"From: Ada <ada@example.org>\r\nSubject: Lunch\r\n\r\nThe vermilion place\r\n",
        )
        .unwrap();
    offer_to_search(&store);
    assert!(
        taken
            .lock()
            .unwrap()
            .items
            .iter()
            .any(|item| item.body.contains("vermilion")),
        "the text reaches search once the body has arrived"
    );

    {
        let mut taken = taken.lock().unwrap();
        taken.items.clear();
        taken.cursor.clear();
    }
    offer_to_search(&store);
    assert!(
        taken
            .lock()
            .unwrap()
            .items
            .iter()
            .any(|item| item.body.contains("vermilion")),
        "an index that lost everything is given it all again"
    );

    store.remove_uids(mailbox, &[7]).unwrap();
    offer_to_search(&store);
    assert_eq!(taken.lock().unwrap().forgotten.len(), 1);
}
