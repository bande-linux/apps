#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use bande_mail::imap::{Flag, Role, Session};
use bande_mail::net::{Network, Security, Server};
use bande_mail::store::{Account, Scope, Store};
use bande_mail::worker::{Command as Order, Event, Mailroom, Status, Worker};

const DOVECOT: &str = "/usr/sbin/dovecot";
const USER: &str = "ann@bande.test";
const PASSWORD: &str = "dovecot test";
const WAIT: Duration = Duration::from_secs(20);

struct Dovecot {
    folder: tempfile::TempDir,
    port: u16,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .unwrap()
}

fn config(folder: &Path, port: u16) -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
    let (uid, gid) = (
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
    );
    let folder = folder.display();
    format!(
        "dovecot_config_version = 2.4.1
dovecot_storage_version = 2.4.1
base_dir = {folder}/run
state_dir = {folder}/state
log_path = {folder}/log
default_internal_user = {user}
default_internal_group = {user}
default_login_user = {user}
protocols = imap
listen = 127.0.0.1
ssl = no
auth_allow_cleartext = yes
mail_driver = maildir
mail_home = {folder}/mail/%{{user | username}}
mail_path = ~/Maildir
passdb static {{
  password = {PASSWORD}
}}
userdb static {{
  fields {{
    uid = {uid}
    gid = {gid}
  }}
}}
service imap-login {{
  chroot =
  inet_listener imap {{
    port = {port}
  }}
  inet_listener imaps {{
    port = 0
  }}
}}
service anvil {{
  chroot =
}}
namespace inbox {{
  inbox = yes
  separator = /
  mailbox Sent {{
    special_use = \\Sent
    auto = subscribe
  }}
  mailbox Trash {{
    special_use = \\Trash
    auto = subscribe
  }}
  mailbox Archive {{
    special_use = \\Archive
    auto = subscribe
  }}
}}
"
    )
}

impl Dovecot {
    fn start() -> Option<Self> {
        if !Path::new(DOVECOT).exists() {
            eprintln!("{DOVECOT} is not installed; skipping the dovecot test");
            return None;
        }
        let folder = tempfile::Builder::new()
            .prefix("dc")
            .tempdir_in("/tmp")
            .unwrap();
        let port = free_port();
        let conf = folder.path().join("dovecot.conf");
        std::fs::write(&conf, config(folder.path(), port)).unwrap();
        let status = Command::new(DOVECOT).arg("-c").arg(&conf).status().unwrap();
        assert!(
            status.success(),
            "dovecot did not start: {}",
            log(folder.path())
        );
        let deadline = Instant::now() + WAIT;
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                Instant::now() < deadline,
                "dovecot never listened: {}",
                log(folder.path())
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Some(Self { folder, port })
    }

    fn server(&self) -> Server {
        Server {
            host: "127.0.0.1".to_owned(),
            port: self.port,
            security: Security::Plain,
        }
    }

    fn session(&self) -> Session {
        let mut session = Session::connect(&self.server(), &Network::default()).unwrap();
        session.login(USER, PASSWORD).unwrap();
        session
    }

    fn deliver(&self, mailbox: &str, subject: &str, flags: &[Flag]) {
        let message = format!(
            "From: Bob <bob@bande.test>\r\nTo: Ann <{USER}>\r\nSubject: {subject}\r\nDate: Sat, 31 Oct 2026 10:00:00 +0000\r\n\r\nBody of {subject}\r\n"
        );
        let mut session = self.session();
        session.append(mailbox, flags, message.as_bytes()).unwrap();
        session.logout();
    }
}

fn log(folder: &Path) -> String {
    std::fs::read_to_string(folder.join("log")).unwrap_or_default()
}

impl Drop for Dovecot {
    fn drop(&mut self) {
        let conf: PathBuf = self.folder.path().join("dovecot.conf");
        let _ = Command::new(DOVECOT)
            .arg("-c")
            .arg(conf)
            .arg("stop")
            .status();
    }
}

fn wait_for(mailroom: &mut Mailroom, wanted: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if let Some(event) = mailroom.wait(Duration::from_millis(100))
            && wanted(&event)
        {
            return event;
        }
    }
    panic!("no such event in time");
}

#[test]
fn a_real_dovecot_syncs_both_ways_and_pushes_new_mail_by_idle() {
    let Some(dovecot) = Dovecot::start() else {
        return;
    };
    dovecot.deliver("INBOX", "First", &[]);
    dovecot.deliver("INBOX", "Second", &[Flag::Seen]);
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("mail.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut account = Account {
        id: 0,
        name: "Ann".to_owned(),
        email: USER.to_owned(),
        user: USER.to_owned(),
        imap: dovecot.server(),
        smtp: dovecot.server(),
    };
    account.id = store.add_account(&account).unwrap();
    let id = account.id;
    let mut mailroom = Mailroom::new().unwrap();
    let worker = Worker::spawn(
        account,
        path.clone(),
        Box::new(|| Ok(Some(PASSWORD.to_owned()))),
        Network::default(),
        mailroom.notifier(),
    );
    wait_for(&mut mailroom, |event| {
        *event == Event::Status(id, Status::Ready)
    });
    let deadline = Instant::now() + WAIT;
    while log(dovecot.folder.path())
        .matches("Logged in: user=<")
        .count()
        < 2
    {
        assert!(
            Instant::now() < deadline,
            "the worker never started to IDLE"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let roles = store
        .mailboxes(id)
        .unwrap()
        .into_iter()
        .filter_map(|mailbox| mailbox.role)
        .collect::<Vec<_>>();
    assert_eq!(roles, [Role::Archive, Role::Inbox, Role::Sent, Role::Trash]);
    let inbox = store.summaries(&Scope::AllOf(Role::Inbox), "").unwrap();
    assert_eq!(inbox.len(), 2);
    assert!(inbox.iter().all(|summary| summary.downloaded));
    let first = inbox
        .iter()
        .find(|summary| summary.subject == "First")
        .unwrap();
    assert!(!first.flags.seen);
    assert_eq!(first.preview, "Body of First");

    store.set_flag(&[first.id], Flag::Flagged, true).unwrap();
    let archive = store.mailbox_with_role(id, Role::Archive).unwrap().unwrap();
    let second = inbox
        .iter()
        .find(|summary| summary.subject == "Second")
        .unwrap();
    store.move_messages(&[second.id], &archive).unwrap();
    worker.send(Order::Flush);
    wait_for(&mut mailroom, |event| *event == Event::Changed(id));
    let mut session = dovecot.session();
    session.select("Archive").unwrap();
    assert_eq!(session.uids().unwrap().len(), 1);
    session.select("INBOX").unwrap();
    let uids = session.uids().unwrap();
    let flags = session.fetch_flags(&uids).unwrap();
    assert_eq!(flags.len(), 1);
    assert!(flags[0].flags.is_some_and(|flags| flags.flagged));
    session.logout();

    std::thread::sleep(Duration::from_millis(300));
    dovecot.deliver("INBOX", "Pushed", &[]);
    let Event::Arrived(_, arrived) =
        wait_for(&mut mailroom, |event| matches!(event, Event::Arrived(..)))
    else {
        unreachable!()
    };
    let pushed = store.summary(arrived[0]).unwrap().unwrap();
    assert_eq!(pushed.subject, "Pushed");
}
