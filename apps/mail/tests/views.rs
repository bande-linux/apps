#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use bande_mail::imap::{Fetched, Flags, Folder, Role};
use bande_mail::net::{Security, Server};
use bande_mail::store::{Account, Store};
use bande_mail::ui::{MailView, Mode, Place, Request, SIDEBAR_WIDTH};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{
    CpuRenderer, FrameRegions, Palette, Point, Scheme, TextSystem, framed_scene,
};

const SIZE: (u32, u32) = (1180, 680);

fn server(host: &str, port: u16) -> Server {
    Server {
        host: host.to_owned(),
        port,
        security: Security::Tls,
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

struct Mail<'a> {
    uid: u32,
    from: &'a str,
    subject: &'a str,
    date: &'a str,
    flags: Flags,
    body: &'a str,
}

fn raw(mail: &Mail<'_>) -> Vec<u8> {
    format!(
        "From: {}\r\nTo: Ann Lee <ann@example.com>\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}@example.com>\r\nMIME-Version: 1.0\r\n{}",
        mail.from, mail.subject, mail.date, mail.uid, mail.body
    )
    .into_bytes()
}

const PLAN: &str = "Content-Type: text/html; charset=utf-8\r\n\r\n<p>Hi Ann,</p><p>Here is the <b>plan</b> for the trip. The train leaves at 08:12 from platform 4, so let's meet at the café by the entrance at a quarter to eight.</p><ul><li>Tickets: booked</li><li>Hotel: two nights, breakfast included</li></ul><p>Details are on <a href=\"https://example.com/trip\">the trip page</a>.</p><img src=\"https://tracker.example/p.gif\"><blockquote><p>Can you send the plan before Friday?</p></blockquote><p>Best,<br>Jörg</p>";

fn place() -> (tempfile::TempDir, std::path::PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("mail.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let account = Account {
        id: 0,
        name: "Ann Lee".to_owned(),
        email: "ann@example.com".to_owned(),
        user: "ann@example.com".to_owned(),
        imap: server("imap.example.com", 993),
        smtp: server("smtp.example.com", 465),
    };
    let id = store.add_account(&account).unwrap();
    store
        .replace_mailboxes(
            id,
            &[
                folder("INBOX", Some(Role::Inbox)),
                folder("Drafts", Some(Role::Drafts)),
                folder("Sent", Some(Role::Sent)),
                folder("Archive", Some(Role::Archive)),
                folder("Junk", Some(Role::Junk)),
                folder("Trash", Some(Role::Trash)),
                folder("Projects", None),
                folder("Projects/Garden", None),
            ],
        )
        .unwrap();
    let inbox = store
        .mailbox_with_role(id, Role::Inbox)
        .unwrap()
        .unwrap()
        .id;
    store.set_mailbox_state(inbox, 1, Some(10)).unwrap();
    let unread = Flags::default();
    let read = Flags {
        seen: true,
        ..Flags::default()
    };
    let mails = [
        Mail {
            uid: 1,
            from: "=?UTF-8?Q?J=C3=B6rg_M=C3=BCller?= <jorg@example.de>",
            subject: "Plans for the weekend trip",
            date: "Sat, 31 Oct 2026 09:41:00 +0000",
            flags: unread,
            body: PLAN,
        },
        Mail {
            uid: 2,
            from: "Garden Club <news@garden.example>",
            subject: "Autumn planting: bulbs, hedges and the last mowing",
            date: "Fri, 30 Oct 2026 17:05:00 +0000",
            flags: Flags {
                flagged: true,
                ..read
            },
            body: "Content-Type: text/plain; charset=utf-8\r\n\r\nThis month we plant tulips and daffodils before the first frost, trim the hedges and put the mower away for winter.\r\n",
        },
        Mail {
            uid: 3,
            from: "Bob <bob@example.com>",
            subject: "Invoice October",
            date: "Tue, 27 Oct 2026 11:20:00 +0000",
            flags: read,
            body: "Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nThe invoice for October is attached. Thanks!\r\n--b\r\nContent-Type: application/pdf; name=\"invoice-october.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n",
        },
        Mail {
            uid: 4,
            from: "Cat <cat@example.org>",
            subject: "Re: Book club",
            date: "Mon, 12 Oct 2026 20:00:00 +0000",
            flags: read,
            body: "Content-Type: text/plain\r\n\r\nSee you on Thursday, I'll bring the second volume.\r\n",
        },
    ];
    let fetched = mails
        .iter()
        .map(|mail| {
            let raw = raw(mail);
            let header_end = raw
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            Fetched {
                uid: mail.uid,
                flags: Some(mail.flags),
                size: Some(raw.len() as u64),
                internal_date: None,
                header: Some(raw[..header_end].to_vec()),
                body: None,
            }
        })
        .collect::<Vec<_>>();
    store.insert_headers(inbox, &fetched).unwrap();
    for mail in &mails {
        store.store_body(inbox, mail.uid, &raw(mail)).unwrap();
    }
    (scratch, path)
}

fn view(path: &Path) -> MailView {
    let mut view = MailView::new(
        path.to_path_buf(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .unwrap();
    view.set_clock(
        "2026-10-31T15:00:00Z".parse::<Timestamp>().unwrap(),
        TimeZone::UTC,
    );
    view.scene(SIZE.0, SIZE.1);
    view
}

fn shoot(name: &str, view: &mut MailView) {
    for (scheme, suffix) in [(Scheme::Light, ""), (Scheme::Dark, "-dark")] {
        let palette = Palette::for_scheme(scheme);
        view.set_palette(palette);
        view.scene(SIZE.0, SIZE.1);
        let scene = view.scene(SIZE.0, SIZE.1);
        let scene = framed_scene(
            &scene,
            &view.title(),
            Some(FrameRegions {
                sidebar: SIDEBAR_WIDTH,
                toolbar: 0.0,
            }),
            Some(view.header()),
            &palette,
        );
        let frame = CpuRenderer::new().render(&scene).expect("render");
        unbound_golden::assert_golden_rgba!(&format!("{name}{suffix}"), SIZE, frame.rgba);
    }
}

fn click(view: &mut MailView, at: Point) {
    view.handle(&Event::PointerDown(Pointer::at(at)));
    view.handle(&Event::PointerUp(Pointer::at(at)));
}

fn press(view: &mut MailView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn the_unified_inbox_lists_newest_first_and_reading_marks_read() {
    let (_folder, path) = place();
    let mut view = view(&path);
    let subjects = view
        .messages()
        .iter()
        .map(|summary| summary.subject.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        subjects,
        [
            "Plans for the weekend trip",
            "Autumn planting: bulbs, hedges and the last mowing",
            "Invoice October",
            "Re: Book club"
        ]
    );
    assert!(!view.messages()[0].flags.seen);
    press(&mut view, Key::Down);
    assert_eq!(view.selected(), Some(view.messages()[0].id));
    assert!(view.messages()[0].flags.seen);
    press(&mut view, Key::Down);
    press(&mut view, Key::Down);
    assert_eq!(view.selected(), Some(view.messages()[2].id));
    shoot("inbox-attachment", &mut view);
}

#[test]
fn reading_a_message_shows_sanitised_html_and_blocks_remote_content() {
    let (_folder, path) = place();
    let mut view = view(&path);
    let first = view.messages()[0].id;
    view.select(first);
    shoot("inbox-reading", &mut view);
}

#[test]
fn search_narrows_the_list_and_mailboxes_switch_from_the_sidebar() {
    let (_folder, path) = place();
    let mut view = view(&path);
    view.search_for("bulbs");
    assert_eq!(view.messages().len(), 1);
    view.search_for("");
    let flagged = view
        .sidebar()
        .iter()
        .position(|row| {
            matches!(
                row,
                bande_mail::ui::Row::Place {
                    place: Place::Flagged,
                    ..
                }
            )
        })
        .unwrap();
    view.scene(SIZE.0, SIZE.1);
    let target = (0..SIZE.1)
        .map(|y| Point::new(60.0, y as f32))
        .find(|point| view.hit(*point) == Some(bande_mail::ui::Hit::Place(flagged)))
        .unwrap();
    click(&mut view, target);
    assert_eq!(view.place(), &Place::Flagged);
    assert_eq!(view.messages().len(), 1);
    assert_eq!(
        view.messages()[0].subject,
        "Autumn planting: bulbs, hedges and the last mowing"
    );
}

#[test]
fn reply_and_forward_open_drafts_that_quote_the_message() {
    let (_folder, path) = place();
    let mut view = view(&path);
    let first = view.messages()[0].id;
    view.select(first);
    view.respond(false);
    view.forward_selected();
    let requests = view.take_requests();
    let drafts = requests
        .iter()
        .filter_map(|request| match request {
            Request::Compose { draft, answering } => Some((draft, *answering)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(drafts.len(), 2);
    let (reply, answering) = &drafts[0];
    assert_eq!(*answering, Some(first));
    assert_eq!(reply.subject, "Re: Plans for the weekend trip");
    assert_eq!(reply.to[0].email, "jorg@example.de");
    assert!(reply.body.contains("Jörg Müller wrote:\n> Hi Ann,"));
    let (forward, answering) = &drafts[1];
    assert_eq!(*answering, None);
    assert_eq!(forward.subject, "Fwd: Plans for the weekend trip");
    assert!(forward.body.contains("Subject: Plans for the weekend trip"));
}

#[test]
fn archiving_moves_the_message_and_selects_the_next() {
    let (_folder, path) = place();
    let mut view = view(&path);
    let first = view.messages()[0].id;
    let second = view.messages()[1].id;
    view.select(first);
    view.handle(&Event::Key {
        key: Key::Character('A'),
        modifiers: Modifiers {
            logo: true,
            shift: true,
            ..Modifiers::default()
        },
    });
    assert_eq!(view.selected(), Some(second));
    assert_eq!(view.messages().len(), 3);
    let secondary =
        Pointer::at(Point::new(SIDEBAR_WIDTH + 100.0, 120.0)).with_button(PointerButton::Secondary);
    view.handle(&Event::PointerDown(secondary));
    assert!(
        view.take_requests()
            .iter()
            .any(|request| matches!(request, Request::ShowMenu(..)))
    );
}

#[test]
fn the_account_sheet_asks_for_name_address_and_password() {
    let (_folder, path) = place();
    let mut view = view(&path);
    view.present_setup(Mode::Add);
    for _ in 0..30 {
        view.scene(SIZE.0, SIZE.1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    shoot("account-setup", &mut view);
}
