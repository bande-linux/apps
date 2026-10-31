#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use bande_calendar::caldav::Client;
use bande_calendar::store::{Account, AccountKind, CalendarInfo, Library};
use bande_calendar::sync::{apply, sync_calendar};

#[derive(Default)]
struct Collection {
    ctag: u32,
    items: BTreeMap<String, (u32, String)>,
    requests: Vec<String>,
}

type Shared = Arc<Mutex<Collection>>;

fn event(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Old//Server//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261001T000000Z\r\nDTSTART:20261102T090000Z\r\nDTEND:20261102T100000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &str) {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let _ = stream.write_all(format!("{head}\r\n{body}").as_bytes());
}

fn multistatus(entries: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:cs=\"http://calendarserver.org/ns/\">{entries}</d:multistatus>"
    )
}

fn item_entry(name: &str, etag: u32, data: Option<&str>) -> String {
    let data = data.map_or(String::new(), |data| {
        format!(
            "<c:calendar-data>{}</c:calendar-data>",
            data.replace('&', "&amp;").replace('<', "&lt;")
        )
    });
    format!(
        "<d:response><d:href>/cal/{name}</d:href><d:propstat><d:prop><d:getetag>\"{etag}\"</d:getetag>{data}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    )
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut length = 0;
    let mut if_match = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap(),
            "if-match" => if_match = Some(value.trim().trim_matches('"').to_owned()),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let body = String::from_utf8(body).unwrap();
    let mut parts = request_line.split_whitespace();
    let (method, path) = (
        parts.next().unwrap().to_owned(),
        parts.next().unwrap().to_owned(),
    );
    let mut collection = shared.lock().unwrap();
    collection.requests.push(format!("{method} {path}"));
    let name = path.trim_start_matches("/cal/").to_owned();
    match method.as_str() {
        "PROPFIND" => {
            let entry = format!(
                "<d:response><d:href>/cal/</d:href><d:propstat><d:prop><cs:getctag>{}</cs:getctag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><d:sync-token/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>",
                collection.ctag
            );
            respond(&mut stream, "207 Multi-Status", &[], &multistatus(&entry));
        }
        "REPORT" if body.contains("calendar-multiget") => {
            let entries: String = collection
                .items
                .iter()
                .filter(|(name, _)| body.contains(&format!("/cal/{name}")))
                .map(|(name, (etag, data))| item_entry(name, *etag, Some(data)))
                .collect();
            respond(&mut stream, "207 Multi-Status", &[], &multistatus(&entries));
        }
        "REPORT" => {
            let entries: String = collection
                .items
                .iter()
                .map(|(name, (etag, _))| item_entry(name, *etag, None))
                .collect();
            respond(&mut stream, "207 Multi-Status", &[], &multistatus(&entries));
        }
        "PUT" => {
            let current = collection
                .items
                .get(&name)
                .map(|(etag, _)| etag.to_string());
            if if_match.is_some() && if_match != current {
                respond(&mut stream, "412 Precondition Failed", &[], "");
                return;
            }
            let etag = collection.ctag + 100;
            collection.ctag += 1;
            collection.items.insert(name, (etag, body));
            respond(
                &mut stream,
                "201 Created",
                &[("ETag", format!("\"{etag}\""))],
                "",
            );
        }
        "GET" => match collection.items.get(&name) {
            Some((etag, data)) => respond(
                &mut stream,
                "200 OK",
                &[("ETag", format!("\"{etag}\""))],
                data,
            ),
            None => respond(&mut stream, "404 Not Found", &[], ""),
        },
        _ => respond(&mut stream, "405 Method Not Allowed", &[], ""),
    }
}

fn start() -> (String, Shared) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/cal/", listener.local_addr().unwrap());
    let shared: Shared = Arc::default();
    let server = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &server);
        }
    });
    (url, shared)
}

fn library(directory: &std::path::Path, url: &str) -> (Library, usize) {
    let mut library = Library::load(directory.to_owned());
    library.add_account(
        Account {
            id: "old".to_owned(),
            name: "Old server".to_owned(),
            kind: AccountKind::CalDav {
                server: url.to_owned(),
                user: "jane".to_owned(),
                home: url.to_owned(),
            },
        },
        vec![CalendarInfo {
            id: "cal".to_owned(),
            account: "old".to_owned(),
            url: Some(url.to_owned()),
            name: "Calendar".to_owned(),
            color: "#888888".to_owned(),
            read_only: false,
            visible: true,
            ctag: None,
            sync_token: None,
        }],
    );
    let index = library.calendar_index("cal").unwrap();
    (library, index)
}

fn summaries(library: &Library, index: usize) -> Vec<String> {
    library.calendars[index]
        .objects
        .iter()
        .filter_map(|stored| Some(stored.parsed.as_ref()?.master()?.summary.clone()))
        .collect()
}

fn sync(library: &mut Library, index: usize, shared: &Shared) -> Vec<String> {
    shared.lock().unwrap().requests.clear();
    let outcome = sync_calendar(&Client::new(None), &library.calendars[index]).unwrap();
    apply(library, outcome, false);
    shared.lock().unwrap().requests.clone()
}

#[test]
fn without_sync_tokens_the_ctag_and_etags_drive_an_incremental_sync() {
    let (url, shared) = start();
    {
        let mut collection = shared.lock().unwrap();
        collection.ctag = 1;
        collection
            .items
            .insert("a.ics".to_owned(), (1, event("a", "Planning")));
        collection
            .items
            .insert("b.ics".to_owned(), (2, event("b", "Retro")));
    }
    let directory = tempfile::tempdir().unwrap();
    let (mut library, index) = library(directory.path(), &url);
    sync(&mut library, index, &shared);
    assert_eq!(summaries(&library, index), ["Planning", "Retro"]);
    assert_eq!(library.calendars[index].info.ctag.as_deref(), Some("1"));

    let requests = sync(&mut library, index, &shared);
    assert_eq!(requests, ["PROPFIND /cal/"]);

    {
        let mut collection = shared.lock().unwrap();
        collection.ctag = 2;
        collection
            .items
            .insert("b.ics".to_owned(), (3, event("b", "Retro (moved)")));
        collection.items.remove("a.ics");
    }
    let requests = sync(&mut library, index, &shared);
    assert_eq!(requests, ["PROPFIND /cal/", "REPORT /cal/", "REPORT /cal/"]);
    assert_eq!(summaries(&library, index), ["Retro (moved)"]);
}

#[test]
fn edits_go_up_with_if_match_and_a_stale_etag_is_a_conflict() {
    let (url, shared) = start();
    {
        let mut collection = shared.lock().unwrap();
        collection.ctag = 1;
        collection
            .items
            .insert("b.ics".to_owned(), (2, event("b", "Retro")));
    }
    let directory = tempfile::tempdir().unwrap();
    let (mut library, index) = library(directory.path(), &url);
    sync(&mut library, index, &shared);
    let object = library.calendars[index].find("b").unwrap();
    let mut edited = library.calendars[index].objects[object]
        .parsed
        .clone()
        .unwrap();
    edited.master_mut().unwrap().summary = "Retro, longer".to_owned();
    library.put_object(index, edited);
    sync(&mut library, index, &shared);
    let stored = shared.lock().unwrap().items.get("b.ics").cloned().unwrap();
    assert!(stored.1.contains("SUMMARY:Retro\\, longer"));
    assert!(
        library.calendars[index]
            .objects
            .iter()
            .all(|stored| stored.pending.is_none())
    );
}
