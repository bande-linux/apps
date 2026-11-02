#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bande_mail::compose::{Draft, Outgoing, from_mailto};
use bande_mail::imap::Session;
use bande_mail::message::Address;
use bande_mail::net::{Network, Security, Server};
use bande_mail::store::{Account, Store};
use bande_mail::worker::{Command as Order, Event, Mailroom, Status, Worker};

const DOVECOT: &str = "/usr/sbin/dovecot";
const USER: &str = "ann@bande.test";
const PASSWORD: &str = "dovecot test";
const WAIT: Duration = Duration::from_secs(20);

type Sent = Arc<Mutex<Vec<(Vec<String>, String)>>>;

fn local(port: u16) -> Server {
    Server {
        host: "127.0.0.1".to_owned(),
        port,
        security: Security::Plain,
    }
}

fn person(email: &str) -> Address {
    Address {
        name: String::new(),
        email: email.to_owned(),
    }
}

struct Dovecot {
    folder: tempfile::TempDir,
    port: u16,
}

impl Dovecot {
    fn start() -> Self {
        let folder = tempfile::Builder::new()
            .prefix("dc")
            .tempdir_in("/tmp")
            .unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
        let (uid, gid) = (
            rustix::process::getuid().as_raw(),
            rustix::process::getgid().as_raw(),
        );
        let dir = folder.path().display();
        let config = format!(
            "dovecot_config_version = 2.4.1\ndovecot_storage_version = 2.4.1\n\
             base_dir = {dir}/run\nstate_dir = {dir}/state\nlog_path = {dir}/log\n\
             default_internal_user = {user}\ndefault_internal_group = {user}\n\
             default_login_user = {user}\nprotocols = imap\nlisten = 127.0.0.1\nssl = no\n\
             auth_allow_cleartext = yes\nmail_driver = maildir\nmail_home = {dir}/mail\n\
             mail_path = ~/Maildir\npassdb static {{\n password = {PASSWORD}\n}}\n\
             userdb static {{\n fields {{\n  uid = {uid}\n  gid = {gid}\n }}\n}}\n\
             service imap-login {{\n chroot =\n inet_listener imap {{\n  port = {port}\n }}\n\
             inet_listener imaps {{\n  port = 0\n }}\n}}\n\
             service anvil {{\n chroot =\n}}\nnamespace inbox {{\n inbox = yes\n\
             mailbox Sent {{\n  special_use = \\Sent\n  auto = subscribe\n }}\n}}\n"
        );
        std::fs::write(folder.path().join("dovecot.conf"), config).unwrap();
        let dovecot = Self { folder, port };
        assert!(dovecot.command(DOVECOT, &[]).status().unwrap().success());
        let deadline = Instant::now() + WAIT;
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "dovecot never listened");
            std::thread::sleep(Duration::from_millis(50));
        }
        dovecot
    }

    fn command(&self, program: &str, arguments: &[&str]) -> Command {
        let mut command = Command::new(program);
        command
            .arg("-c")
            .arg(self.folder.path().join("dovecot.conf"))
            .args(arguments);
        command
    }
}

impl Drop for Dovecot {
    fn drop(&mut self) {
        let _ = self.command(DOVECOT, &["stop"]).status();
    }
}

fn smtp_server(sent: Sent) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut writer = stream.try_clone().unwrap();
            let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
            let mut recipients = Vec::new();
            writer.write_all(b"220 ready\r\n").unwrap();
            while let Some(line) = lines.next() {
                let reply = match line.split(':').next().unwrap_or_default() {
                    "RCPT TO" => {
                        recipients.push(line.trim_start_matches("RCPT TO:").to_owned());
                        "250 ok"
                    }
                    "DATA" => {
                        writer.write_all(b"354 go on\r\n").unwrap();
                        let data = lines.by_ref().take_while(|line| line != ".");
                        let data = data.collect::<Vec<_>>().join("\r\n");
                        sent.lock().unwrap().push((recipients.clone(), data));
                        "250 queued"
                    }
                    "QUIT" => "221 bye",
                    _ => "250 ok",
                };
                writer.write_all(format!("{reply}\r\n").as_bytes()).unwrap();
            }
        }
    });
    port
}

#[test]
fn a_message_goes_out_once_without_injected_headers_after_the_imap_session_went_stale() {
    if !Path::new(DOVECOT).exists() {
        eprintln!("{DOVECOT} is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start();
    let sent = Sent::default();
    let path = dovecot.folder.path().join("mail.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut account = Account {
        id: 0,
        name: "Ann".to_owned(),
        email: USER.to_owned(),
        user: USER.to_owned(),
        imap: local(dovecot.port),
        smtp: local(smtp_server(sent.clone())),
        signature: String::new(),
        enabled: true,
    };
    account.id = store.add_account(&account).unwrap();
    let id = account.id;
    let mut mailroom = Mailroom::new().unwrap();
    let worker = Worker::spawn(
        account,
        path,
        Box::new(|| Ok(Some(PASSWORD.to_owned()))),
        Network::default(),
        mailroom.notifier(),
    );
    let deadline = Instant::now() + WAIT;
    while mailroom.wait(Duration::from_millis(100)) != Some(Event::Status(id, Status::Ready)) {
        assert!(Instant::now() < deadline, "the first sync never finished");
    }

    let kicked = dovecot
        .command("doveadm", &["kick", USER])
        .status()
        .unwrap();
    assert!(kicked.success());
    let mut eve = person("eve@example.com");
    eve.name = "Eve\r\nBcc: spy@example.net".to_owned();
    worker.send(Order::Send(Box::new(Outgoing {
        draft: Draft {
            from: person(USER),
            to: vec![person("bob@example.com"), eve],
            bcc: vec![person("boss@example.com")],
            subject: "Plans\r\nInjected: header".to_owned(),
            body: "See you".to_owned(),
            ..Draft::default()
        },
        ..Outgoing::default()
    })));

    let mut session = Session::connect(&local(dovecot.port), &Network::default()).unwrap();
    session.login(USER, PASSWORD).unwrap();
    let deadline = Instant::now() + WAIT;
    while session.select("Sent").unwrap().exists == 0 {
        assert!(Instant::now() < deadline, "no copy reached Sent");
        std::thread::sleep(Duration::from_millis(100));
    }
    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "the message went out more than once");
    let (recipients, data) = &sent[0];
    assert_eq!(
        recipients,
        &[
            "<bob@example.com>",
            "<eve@example.com>",
            "<boss@example.com>"
        ]
    );
    assert!(!data.contains("boss@example.com"));
    assert!(!data.contains("\r\nBcc:"));
    assert!(!data.contains("\r\nInjected:"));
}

#[test]
fn a_mailto_link_cannot_add_hidden_recipients_or_attachments() {
    let link =
        "mailto:ann@example.com?bcc=spy%40example.net&attach=/etc/passwd&cc=cat%40example.org";
    let draft = from_mailto(link).unwrap();
    assert_eq!(draft.to, [person("ann@example.com")]);
    assert_eq!(draft.cc, [person("cat@example.org")]);
    assert!(draft.bcc.is_empty());
    assert!(draft.attachments.is_empty());
}
