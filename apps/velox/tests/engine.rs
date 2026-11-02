#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bande_toolkit::Appearance;
use bande_velox::{Informer, Update, VeloxView};

const CONTAINER: &str = r#"{"Id":"c1","Names":["/shop-web-1"],"Image":"web","State":"running","Status":"Up","Labels":{"com.docker.compose.project":"shop"}}"#;

fn serve(socket: &Path) -> Arc<Mutex<Vec<UnixStream>>> {
    let listener = UnixListener::bind(socket).unwrap();
    let held = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::clone(&held);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().map_while(Result::ok) {
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let body = match line.split(' ').nth(1).unwrap_or_default() {
                "/version" => r#"{"Version":"28.5.1","ApiVersion":"1.51"}"#.to_owned(),
                "/info" => r#"{"SecurityOptions":["name=rootless"]}"#.to_owned(),
                path if path.starts_with("/events") => {
                    let event = r#"{"Action":"start","Actor":{"ID":"c1"}}"#;
                    let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
                    write!(stream, "{head}{:x}\r\n{event}\n\r\n", event.len() + 1).unwrap();
                    events.lock().unwrap().push(stream);
                    continue;
                }
                _ => format!("[{CONTAINER}]"),
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    held
}

fn next(informer: &Informer, queue: &mut Vec<Update>, wanted: fn(&Update) -> bool) -> Update {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        queue.extend(informer.drain());
        if let Some(found) = queue.iter().position(wanted) {
            return queue.drain(..=found).next_back().unwrap();
        }
        assert!(Instant::now() < deadline, "no update in time");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_list_follows_events_and_reconciles_when_the_engine_stops_and_returns() {
    let folder = tempfile::tempdir().unwrap();
    let socket = folder.path().join("engine.sock");
    unsafe { std::env::set_var("DOCKER_HOST", format!("unix://{}", socket.display())) };
    let informer = Informer::start().unwrap();
    let queue = &mut Vec::new();
    let blocked = next(&informer, queue, |u| matches!(u, Update::Blocked(_)));
    let mut view = VeloxView::new(Appearance::default());
    view.apply(blocked);
    assert!(format!("{:?}", view.ui_ref().accessibility()).contains("Copy"));

    let engine = serve(&socket);
    let ready = next(&informer, queue, |u| matches!(u, Update::Ready(_)));
    assert!(matches!(ready, Update::Ready(info) if info.rootless && info.version == "28.5.1"));
    let listed = next(&informer, queue, |u| matches!(u, Update::Listed(_)));
    assert!(matches!(listed, Update::Listed(list) if list[0].project() == Some("shop")));
    next(&informer, queue, |u| matches!(u, Update::Changed(_)));

    engine.lock().unwrap().clear();
    std::fs::remove_file(&socket).unwrap();
    next(&informer, queue, |u| matches!(u, Update::Blocked(_)));
    let _engine = serve(&socket);
    next(
        &informer,
        queue,
        |u| matches!(u, Update::Listed(list) if list.len() == 1),
    );
}
