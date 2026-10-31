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
            Request::Compose(outgoing) => Some((&outgoing.draft, outgoing.answering)),
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

#[test]
fn a_reply_opens_in_a_compose_window_with_the_quote() {
    use bande_mail::ui::{COMPOSE_SIZE, ComposeView};
    let (_folder, path) = place();
    let mut view = view(&path);
    let first = view.messages()[0].id;
    view.select(first);
    view.respond(false);
    let Some(Request::Compose(outgoing)) = view.take_requests().into_iter().next() else {
        panic!("no draft")
    };
    let senders = view
        .accounts()
        .iter()
        .map(|account| account.address())
        .collect();
    let mut compose = ComposeView::new(
        *outgoing,
        senders,
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    assert_eq!(compose.title(), "Re: Plans for the weekend trip");
    for (scheme, suffix) in [(Scheme::Light, ""), (Scheme::Dark, "-dark")] {
        let palette = Palette::for_scheme(scheme);
        compose.set_palette(palette);
        compose.scene(COMPOSE_SIZE.0, COMPOSE_SIZE.1);
        let scene = compose.scene(COMPOSE_SIZE.0, COMPOSE_SIZE.1);
        let scene = framed_scene(
            &scene,
            &compose.title(),
            Some(FrameRegions {
                sidebar: 0.0,
                toolbar: 0.0,
            }),
            Some(compose.header()),
            &palette,
        );
        let frame = CpuRenderer::new().render(&scene).expect("render");
        unbound_golden::assert_golden_rgba!(
            &format!("compose-reply{suffix}"),
            COMPOSE_SIZE,
            frame.rgba
        );
    }
    compose.handle(&Event::Key {
        key: Key::Character('D'),
        modifiers: Modifiers {
            logo: true,
            shift: true,
            ..Modifiers::default()
        },
    });
    let sent = compose.take_sent().unwrap();
    assert_eq!(sent.answering, Some(first));
    assert_eq!(sent.draft.to[0].email, "jorg@example.de");
    assert!(compose.finished());
}

#[test]
fn attachments_added_to_a_message_can_be_removed_before_sending() {
    use bande_mail::compose::Draft;
    use bande_mail::message::{Address, Attachment};
    use bande_mail::ui::{COMPOSE_SIZE, ComposeView};
    let me = Address {
        name: "Ann Lee".to_owned(),
        email: "ann@example.com".to_owned(),
    };
    let draft = Draft {
        from: me.clone(),
        to: vec![Address {
            name: String::new(),
            email: "bob@example.com".to_owned(),
        }],
        subject: "Photos".to_owned(),
        ..Draft::default()
    };
    let mut compose = ComposeView::new(
        bande_mail::compose::Outgoing {
            draft,
            ..Default::default()
        },
        vec![me],
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    let file = |name: &str| Attachment {
        name: name.to_owned(),
        mime: "image/jpeg".to_owned(),
        data: vec![0; 2048],
    };
    compose.add_attachments(vec![file("beach.jpg"), file("hills.jpg")]);
    compose.scene(COMPOSE_SIZE.0, COMPOSE_SIZE.1);
    let chip = Point::new(
        90.0,
        unbound_toolkit::WINDOW_HEADER_HEIGHT as f32 + 3.0 * 36.0 + 18.0,
    );
    compose.handle(&Event::PointerDown(Pointer::at(chip)));
    assert_eq!(compose.attachments().len(), 1);
    assert_eq!(compose.attachments()[0].name, "hills.jpg");
    compose.handle(&Event::Key {
        key: Key::Character('D'),
        modifiers: Modifiers {
            logo: true,
            shift: true,
            ..Modifiers::default()
        },
    });
    let sent = compose.take_sent().unwrap();
    assert_eq!(sent.draft.attachments.len(), 1);
}

#[test]
fn closing_an_untouched_reply_keeps_no_draft_but_an_edited_one_does() {
    use bande_mail::ui::ComposeView;
    let (_folder, path) = place();
    let mut view = view(&path);
    let first = view.messages()[0].id;
    view.select(first);
    view.respond(false);
    let Some(Request::Compose(outgoing)) = view.take_requests().into_iter().next() else {
        panic!("no draft")
    };
    let senders: Vec<_> = view
        .accounts()
        .iter()
        .map(|account| account.address())
        .collect();
    let compose = ComposeView::new(
        (*outgoing).clone(),
        senders.clone(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    assert_eq!(compose.unsent(), None);
    let mut edited = ComposeView::new(
        *outgoing,
        senders,
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    edited.handle(&Event::Commit("Sounds good".to_owned()));
    let unsent = edited.unsent().unwrap();
    assert!(unsent.draft.body.starts_with("Sounds good"));
    assert_eq!(unsent.answering, Some(first));
}

#[test]
fn inline_pictures_show_at_once_and_remote_ones_only_on_request() {
    let (_folder, path) = place();
    let mut picture = Vec::new();
    let pixels = image::RgbaImage::from_fn(240, 120, |x, y| {
        image::Rgba([(x % 256) as u8, (120 + y % 100) as u8, 90, 255])
    });
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(
            &mut std::io::Cursor::new(&mut picture),
            image::ImageFormat::Png,
        )
        .unwrap();
    let encoded = data_encoding::BASE64.encode(&picture);
    let raw = format!(
        "From: Garden Club <news@garden.example>\r\nTo: Ann Lee <ann@example.com>\r\nSubject: The new garden map\r\n\
Date: Sat, 31 Oct 2026 14:00:00 +0000\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"r\"\r\n\r\n--r\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<p>Our new garden map:</p><img src=\"cid:map@garden\"><p>And a view from the gate:</p><img src=\"https://127.0.0.1:9/gate.png\">\r\n\
--r\r\nContent-Type: image/png; name=\"map.png\"\r\nContent-ID: <map@garden>\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n--r--\r\n"
    )
    .into_bytes();
    {
        let mut store = Store::open(&path).unwrap();
        let account = store.accounts().unwrap()[0].id;
        let inbox = store
            .mailbox_with_role(account, Role::Inbox)
            .unwrap()
            .unwrap()
            .id;
        let header_end = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        store
            .insert_headers(
                inbox,
                &[Fetched {
                    uid: 9,
                    flags: Some(Flags::default()),
                    size: Some(raw.len() as u64),
                    internal_date: None,
                    header: Some(raw[..header_end].to_vec()),
                    body: None,
                }],
            )
            .unwrap();
        store.store_body(inbox, 9, &raw).unwrap();
    }
    let mut view = view(&path);
    let map = view
        .messages()
        .iter()
        .find(|summary| summary.subject == "The new garden map")
        .unwrap()
        .id;
    view.select(map);
    shoot("inline-picture", &mut view);
    let banner = (0..SIZE.1)
        .flat_map(|y| {
            (0..SIZE.0)
                .step_by(8)
                .map(move |x| Point::new(x as f32, y as f32))
        })
        .find(|point| view.hit(*point) == Some(bande_mail::ui::Hit::LoadRemote))
        .unwrap();
    assert!(
        !view
            .hit(banner)
            .is_some_and(|hit| matches!(hit, bande_mail::ui::Hit::Attachment(_)))
    );
    click(&mut view, banner);
    view.scene(SIZE.0, SIZE.1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        view.engine_ready();
        view.scene(SIZE.0, SIZE.1);
        let still_offered = (0..SIZE.1)
            .flat_map(|y| {
                (0..SIZE.0)
                    .step_by(8)
                    .map(move |x| Point::new(x as f32, y as f32))
            })
            .any(|point| view.hit(point) == Some(bande_mail::ui::Hit::LoadRemote));
        if !still_offered {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the remote content never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
