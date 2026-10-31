#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fake;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bande_mail::compose::Draft;
use bande_mail::error::Error;
use bande_mail::imap::{Flag, Role, Session};
use bande_mail::message::Address;
use bande_mail::net::{Network, Security, Server};
use bande_mail::store::{Account, Scope, Store};
use bande_mail::worker::{Command, Event, Mailroom, Status, Worker};
use fake::{Behaviour, Fake, PASSWORD, USER, message};

const WAIT: Duration = Duration::from_secs(20);

fn network() -> Network {
    Network {
        timeout: Duration::from_secs(2),
        ..Network::default()
    }
}

fn server(port: u16) -> Server {
    Server {
        host: "127.0.0.1".to_owned(),
        port,
        security: Security::Plain,
    }
}

struct Setup {
    fake: Fake,
    _folder: tempfile::TempDir,
    path: PathBuf,
    store: Store,
    account: Account,
    mailroom: Mailroom,
}

impl Setup {
    fn new() -> Self {
        let fake = Fake::start();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("mail.sqlite3");
        let store = Store::open(&path).unwrap();
        let mut account = Account {
            id: 0,
            name: "Ann".to_owned(),
            email: USER.to_owned(),
            user: USER.to_owned(),
            imap: server(fake.imap_port),
            smtp: server(fake.smtp_port),
        };
        account.id = store.add_account(&account).unwrap();
        Self {
            fake,
            _folder: folder,
            path,
            store,
            account,
            mailroom: Mailroom::new().unwrap(),
        }
    }

    fn worker(&self, password: Option<&str>) -> Worker {
        let password = password.map(str::to_owned);
        Worker::spawn(
            self.account.clone(),
            self.path.clone(),
            Box::new(move || Ok(password.clone())),
            network(),
            self.mailroom.notifier(),
        )
    }

    fn wait_for(&mut self, wanted: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(event) = self.mailroom.wait(Duration::from_millis(100))
                && wanted(&event)
            {
                return event;
            }
        }
        panic!("no such event in time");
    }

    fn ready(&mut self) {
        let id = self.account.id;
        self.wait_for(|event| *event == Event::Status(id, Status::Ready));
    }

    fn subjects(&self, scope: &Scope) -> Vec<String> {
        self.store
            .summaries(scope, "")
            .unwrap()
            .into_iter()
            .map(|summary| summary.subject)
            .collect()
    }

    fn inbox(&self) -> i64 {
        self.store
            .mailbox_with_role(self.account.id, Role::Inbox)
            .unwrap()
            .unwrap()
            .id
    }
}

#[test]
fn a_first_sync_brings_folders_headers_and_bodies() {
    let mut setup = Setup::new();
    setup
        .fake
        .deliver("INBOX", &message("Hello", "First body"), &["\\Seen"]);
    setup
        .fake
        .deliver("INBOX", &message("Plans", "Second body"), &[]);
    setup
        .fake
        .deliver("Sent", &message("Mine", "sent body"), &["\\Seen"]);
    let _worker = setup.worker(Some(PASSWORD));
    setup.ready();

    let mailboxes = setup.store.mailboxes(setup.account.id).unwrap();
    let roles = mailboxes
        .iter()
        .map(|mailbox| (mailbox.name.as_str(), mailbox.role))
        .collect::<Vec<_>>();
    assert_eq!(
        roles,
        [
            ("Archive", Some(Role::Archive)),
            ("Entwürfe", Some(Role::Drafts)),
            ("INBOX", Some(Role::Inbox)),
            ("Papierkorb", Some(Role::Trash)),
            ("Sent", Some(Role::Sent)),
        ]
    );
    let inbox = setup
        .store
        .summaries(&Scope::AllOf(Role::Inbox), "")
        .unwrap();
    assert_eq!(inbox.len(), 2);
    assert!(inbox.iter().all(|summary| summary.downloaded));
    let plans = inbox
        .iter()
        .find(|summary| summary.subject == "Plans")
        .unwrap();
    assert!(!plans.flags.seen);
    assert_eq!(plans.preview, "Second body");
    assert_eq!(plans.sender.name, "Bob");
    assert_eq!(setup.subjects(&Scope::AllOf(Role::Sent)), ["Mine"]);
    assert_eq!(setup.store.unread(&Scope::AllOf(Role::Inbox)).unwrap(), 1);
}

#[test]
fn flags_moves_and_deletions_reach_the_server_and_come_back() {
    let mut setup = Setup::new();
    let first = setup.fake.deliver("INBOX", &message("One", "1"), &[]);
    let second = setup.fake.deliver("INBOX", &message("Two", "2"), &[]);
    let third = setup.fake.deliver("INBOX", &message("Three", "3"), &[]);
    let worker = setup.worker(Some(PASSWORD));
    setup.ready();

    let summaries = setup
        .store
        .summaries(&Scope::AllOf(Role::Inbox), "")
        .unwrap();
    let id_of = |uid: u32| {
        summaries
            .iter()
            .find(|summary| summary.uid == uid)
            .unwrap()
            .id
    };
    setup
        .store
        .set_flag(&[id_of(first)], Flag::Seen, true)
        .unwrap();
    setup
        .store
        .set_flag(&[id_of(first)], Flag::Flagged, true)
        .unwrap();
    let archive = setup
        .store
        .mailbox_with_role(setup.account.id, Role::Archive)
        .unwrap()
        .unwrap();
    setup
        .store
        .move_messages(&[id_of(second)], &archive)
        .unwrap();
    setup.store.expunge(&[id_of(third)]).unwrap();
    worker.send(Command::Flush);
    let id = setup.account.id;
    setup.wait_for(|event| *event == Event::Changed(id));

    let flags = setup.fake.flags("INBOX", first).unwrap();
    assert!(flags.contains("\\Seen") && flags.contains("\\Flagged"));
    assert_eq!(setup.fake.subjects("INBOX"), ["One"]);
    assert_eq!(setup.fake.subjects("Archive"), ["Two"]);

    setup.fake.set_flag("INBOX", first, "\\Answered");
    setup.fake.deliver("INBOX", &message("Four", "4"), &[]);
    worker.send(Command::Sync);
    setup.ready();
    let inbox = setup
        .store
        .summaries(&Scope::AllOf(Role::Inbox), "")
        .unwrap();
    assert_eq!(
        inbox
            .iter()
            .map(|summary| summary.subject.as_str())
            .collect::<Vec<_>>(),
        ["Four", "One"]
    );
    let one = inbox
        .iter()
        .find(|summary| summary.subject == "One")
        .unwrap();
    assert!(one.flags.answered && one.flags.flagged && one.flags.seen);
    assert_eq!(setup.subjects(&Scope::AllOf(Role::Archive)), ["Two"]);
    assert!(setup.store.pending(id).unwrap().is_empty());
}

#[test]
fn new_mail_arrives_through_idle() {
    let mut setup = Setup::new();
    setup
        .fake
        .deliver("INBOX", &message("Old", "old"), &["\\Seen"]);
    let _worker = setup.worker(Some(PASSWORD));
    setup.ready();
    let deadline = Instant::now() + WAIT;
    while !setup
        .fake
        .with(|state| state.log.iter().any(|line| line.ends_with(" IDLE")))
    {
        assert!(
            Instant::now() < deadline,
            "the worker never started to IDLE"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    setup
        .fake
        .deliver("INBOX", &message("Fresh news", "new body"), &[]);
    let id = setup.account.id;
    let Event::Arrived(_, messages) =
        setup.wait_for(|event| matches!(event, Event::Arrived(account, _) if *account == id))
    else {
        unreachable!()
    };
    let arrived = setup.store.summary(messages[0]).unwrap().unwrap();
    assert_eq!(arrived.subject, "Fresh news");
    assert_eq!(messages.len(), 1);
}

#[test]
fn a_sent_message_goes_out_by_smtp_and_is_kept_in_sent() {
    let mut setup = Setup::new();
    setup.fake.deliver("INBOX", &message("Question", "?"), &[]);
    let worker = setup.worker(Some(PASSWORD));
    setup.ready();
    let question = setup
        .store
        .summaries(&Scope::AllOf(Role::Inbox), "")
        .unwrap()[0]
        .clone();
    let draft = Draft {
        from: setup.account.address(),
        to: vec![Address {
            name: "Bob".to_owned(),
            email: "bob@example.com".to_owned(),
        }],
        bcc: vec![Address {
            name: String::new(),
            email: "boss@example.com".to_owned(),
        }],
        subject: "Re: Question".to_owned(),
        body: "Answer\n.\nwith a dot line".to_owned(),
        ..Draft::default()
    };
    worker.send(Command::Send {
        draft: Box::new(draft),
        answering: Some(question.id),
    });
    let id = setup.account.id;
    setup.wait_for(|event| *event == Event::Sent(id));
    setup.wait_for(|event| *event == Event::Changed(id));
    let (from, recipients, data) = setup.fake.with(|state| state.sent[0].clone());
    assert_eq!(from, USER);
    assert_eq!(recipients, ["bob@example.com", "boss@example.com"]);
    let text = String::from_utf8_lossy(&data);
    assert!(text.contains("Subject: Re: Question"));
    let received = bande_mail::message::Content::parse(&data);
    assert_eq!(
        received
            .plain
            .as_deref()
            .map(|plain| plain.replace("\r\n", "\n").trim_end().to_owned()),
        Some("Answer\n.\nwith a dot line".to_owned())
    );
    assert!(!text.contains("boss@example.com"));
    let deadline = Instant::now() + WAIT;
    while setup.fake.subjects("Sent") != ["Re: Question"] {
        assert!(Instant::now() < deadline, "no copy in Sent");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        setup
            .fake
            .flags("INBOX", question.uid)
            .unwrap()
            .contains("\\Answered")
    );
}

#[test]
fn a_wrong_password_is_reported_and_a_new_one_is_used() {
    let mut setup = Setup::new();
    setup.fake.deliver("INBOX", &message("Hi", "x"), &[]);
    let worker = setup.worker(Some("wrong"));
    let id = setup.account.id;
    setup.wait_for(|event| matches!(event, Event::LoginFailed(account, text) if *account == id && text.contains("Invalid credentials")));
    worker.send(Command::Password(PASSWORD.to_owned()));
    setup.ready();
    assert_eq!(setup.subjects(&Scope::AllOf(Role::Inbox)), ["Hi"]);
}

#[test]
fn a_failed_send_hands_the_draft_back() {
    let mut setup = Setup::new();
    let worker = setup.worker(Some(PASSWORD));
    setup.ready();
    let draft = Draft {
        from: setup.account.address(),
        subject: "Nobody".to_owned(),
        ..Draft::default()
    };
    worker.send(Command::Send {
        draft: Box::new(draft.clone()),
        answering: None,
    });
    let id = setup.account.id;
    let event = setup.wait_for(|event| matches!(event, Event::NotSent(..)));
    assert!(
        matches!(event, Event::NotSent(account, _, returned) if account == id && *returned == draft)
    );
}

#[test]
fn a_new_uidvalidity_starts_the_mailbox_over() {
    let mut setup = Setup::new();
    setup.fake.deliver("INBOX", &message("Before", "x"), &[]);
    let worker = setup.worker(Some(PASSWORD));
    setup.ready();
    let inbox = setup.inbox();
    assert_eq!(setup.subjects(&Scope::Mailbox(inbox)), ["Before"]);
    setup.fake.with(|state| {
        let inbox = state.folders.get_mut("INBOX").unwrap();
        inbox.validity = 99;
        inbox.messages.clear();
        inbox.next_uid = 1;
    });
    setup.fake.deliver("INBOX", &message("After", "y"), &[]);
    worker.send(Command::Sync);
    setup.ready();
    assert_eq!(setup.subjects(&Scope::Mailbox(inbox)), ["After"]);
}

#[test]
fn servers_without_move_or_special_use_still_work() {
    let mut setup = Setup::new();
    setup.fake.with(|state| {
        state.moves = false;
        state.special_use = false;
        state.idle = false;
    });
    setup.fake.deliver("INBOX", &message("Keep", "x"), &[]);
    setup.fake.deliver("INBOX", &message("Bin", "x"), &[]);
    let worker = setup.worker(Some(PASSWORD));
    setup.ready();
    let trash = setup
        .store
        .mailbox_with_role(setup.account.id, Role::Trash)
        .unwrap();
    assert_eq!(trash, None);
    let sent = setup
        .store
        .mailbox_with_role(setup.account.id, Role::Sent)
        .unwrap()
        .unwrap();
    assert_eq!(sent.name, "Sent");
    let archive = setup
        .store
        .mailboxes(setup.account.id)
        .unwrap()
        .into_iter()
        .find(|mailbox| mailbox.name == "Archive")
        .unwrap();
    let bin = setup
        .store
        .summaries(&Scope::Mailbox(setup.inbox()), "bin")
        .unwrap()[0]
        .id;
    setup.store.move_messages(&[bin], &archive).unwrap();
    worker.send(Command::Flush);
    let id = setup.account.id;
    setup.wait_for(|event| *event == Event::Changed(id));
    assert_eq!(setup.fake.subjects("INBOX"), ["Keep"]);
    assert_eq!(setup.fake.subjects("Archive"), ["Bin"]);
}

#[test]
fn bad_servers_fail_quickly_instead_of_hanging() {
    let fake = Fake::start();
    for (behaviour, expected) in [
        (Behaviour::Silent, "timeout"),
        (Behaviour::Garbage, "protocol"),
    ] {
        fake.with(|state| state.behaviour = behaviour);
        let started = Instant::now();
        let result = Session::connect(&server(fake.imap_port), &network());
        let kind = match result {
            Err(Error::Timeout) => "timeout",
            Err(Error::Protocol(_)) => "protocol",
            Err(error) => panic!("{error}"),
            Ok(_) => "connected",
        };
        assert_eq!(kind, expected);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    fake.with(|state| state.behaviour = Behaviour::HangUpAfterLogin);
    let mut session = Session::connect(&server(fake.imap_port), &network()).unwrap();
    session.login(USER, PASSWORD).unwrap_or(());
    assert!(matches!(session.list(), Err(error) if error.is_connection()));
}
