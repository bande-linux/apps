#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use accesskit::Role;
use bande_monitor::{
    Icons, InspectorView, OpenFile, Request, Screen, Section, Snapshot, read_details,
};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer};
use bande_toolkit::{
    Appearance, FrameRegions, Palette, Point, Scheme, TextSize, TextSystem, framed_scene,
};
use fixture::{Fixture, TICKS_PER_SECOND, Task, check_golden};

const SIZE: (u32, u32) = (680, 520);

fn family(fixture: &Fixture) -> Snapshot {
    fixture.spawn(&Task {
        pid: 300,
        comm: "bande-shell",
        command: &["/usr/bin/bande-shell"],
        start_ticks: 300,
        ..Task::default()
    });
    fixture.spawn(&Task {
        pid: 400,
        ppid: 300,
        comm: "bash",
        command: &["/bin/bash", "--login"],
        cpu_ticks: 250,
        start_ticks: 400,
        ..Task::default()
    });
    fixture.spawn(&Task {
        pid: 500,
        ppid: 400,
        comm: "vim",
        command: &["vim", "notes.txt"],
        start_ticks: 500,
        ..Task::default()
    });
    fixture.write("proc/uptime", "60.00 100.00\n");
    fixture.write(
        "proc/400/task/400/stat",
        "400 (bash) S 300 400 400 0 -1 4194304 0 0 0 0 200 50 0 0 20 0 2 0 400 1000000 256 0\n",
    );
    fixture.write(
        "proc/400/task/402/stat",
        "402 (worker) R 300 400 400 0 -1 4194304 0 0 0 0 10 0 0 0 20 0 2 0 400 1000000 256 0\n",
    );
    fixture.write(
        "proc/400/maps",
        "aaaa00000000-aaaa00010000 r-xp 00000000 fe:03 12 /usr/bin/bash\nffff00000000-ffff00021000 rw-p 00000000 00:00 0 \n",
    );
    fixture.write(
        "proc/400/status",
        "Name:\tbash\nUid:\t1000\t1000\t1000\t1000\nVmSize:\t    8192 kB\n",
    );
    let fds = fixture.path().join("proc/400/fd");
    std::fs::create_dir_all(&fds).expect("fd folder");
    std::os::unix::fs::symlink("/dev/pts/0", fds.join("0")).expect("link");
    std::os::unix::fs::symlink("/home/mikael/notes.txt", fds.join("10")).expect("link");
    std::os::unix::fs::symlink("socket:[1234]", fds.join("3")).expect("link");
    fixture.sampler().sample().expect("sample")
}

fn bash(snapshot: &Snapshot) -> &bande_monitor::Process {
    snapshot
        .processes
        .iter()
        .find(|process| process.pid == 400)
        .expect("bash")
}

fn inspector(fixture: &Fixture, snapshot: &Snapshot) -> InspectorView {
    let mut view = InspectorView::new(
        bash(snapshot),
        fixture.path(),
        TICKS_PER_SECOND,
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        Icons::new(|_| None),
    );
    view.update(snapshot.clone());
    view.scene(SIZE.0, SIZE.1);
    view
}

fn rows(view: &InspectorView) -> Vec<String> {
    let tree = view.accessibility().expect("tree");
    let mut rows = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Row)
        .filter_map(|(_, node)| Some((node.row_index()?, node.label()?.to_owned())))
        .collect::<Vec<_>>();
    rows.sort();
    rows.into_iter().map(|(_, label)| label).collect()
}

fn cells(view: &InspectorView) -> Vec<String> {
    let tree = view.accessibility().expect("tree");
    tree.nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Cell)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

fn command(character: char) -> Event {
    Event::Key {
        key: Key::Character(character),
        modifiers: Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    }
}

#[test]
fn details_list_threads_open_files_and_mappings_in_order() {
    let fixture = Fixture::new();
    family(&fixture);
    let details = read_details(fixture.path(), 400, TICKS_PER_SECOND as f64).expect("details");
    let threads = details
        .threads
        .iter()
        .map(|thread| {
            (
                thread.tid,
                thread.name.as_str(),
                thread.state,
                thread.cpu_seconds,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        threads,
        [(400, "bash", 'S', 2.5), (402, "worker", 'R', 0.1)]
    );
    assert_eq!(
        details.files.expect("files"),
        [
            OpenFile {
                fd: 0,
                target: "/dev/pts/0".to_owned()
            },
            OpenFile {
                fd: 3,
                target: "socket:[1234]".to_owned()
            },
            OpenFile {
                fd: 10,
                target: "/home/mikael/notes.txt".to_owned()
            },
        ]
    );
    assert_eq!(details.maps.expect("maps").len(), 2);
    assert_eq!(details.virtual_size, Some(8192 * 1024));
    assert_eq!(details.running_seconds, Some(56.0));
}

fn click_row(view: &mut InspectorView, label: &str, count: u8) {
    let index = rows(view)
        .iter()
        .position(|row| row == label)
        .expect("a row with that label");
    let row = view.table().expect("table").row_bounds(index).expect("row");
    let pointer =
        Pointer::at(Point::new(row.x + 200.0, row.y + row.height / 2.0)).with_count(count);
    view.handle(&Event::PointerDown(pointer));
    view.handle(&Event::PointerUp(pointer));
    view.scene(SIZE.0, SIZE.1);
}

#[test]
fn the_inspector_links_the_family_and_shows_each_section() {
    let fixture = Fixture::new();
    let snapshot = family(&fixture);
    let mut view = inspector(&fixture, &snapshot);
    let shown = cells(&view);
    for value in [
        "bande-shell (300)",
        "vim (500)",
        "/bin/bash --login",
        "session-1.scope",
    ] {
        assert!(
            shown.iter().any(|cell| cell == value),
            "{value} in {shown:?}"
        );
    }
    click_row(&mut view, "Parent", 2);
    assert_eq!(view.take_requests(), [Request::Inspect(300)]);
    click_row(&mut view, "Children", 2);
    assert_eq!(view.take_requests(), [Request::Inspect(500)]);
    click_row(&mut view, "PID", 1);
    assert!(view.handle(&command('c')));
    assert_eq!(view.take_requests(), [Request::Copy("400".to_owned())]);

    view.handle(&command('2'));
    view.scene(SIZE.0, SIZE.1);
    assert_eq!(view.section(), Section::Threads);
    assert_eq!(rows(&view).len(), 2);
    view.handle(&command('3'));
    view.scene(SIZE.0, SIZE.1);
    assert_eq!(rows(&view), ["0", "3", "10"]);

    fixture.exit(400);
    let ended = fixture.sampler().sample().expect("sample");
    view.update(ended);
    view.handle(&command('1'));
    view.scene(SIZE.0, SIZE.1);
    assert!(cells(&view).iter().any(|cell| cell == "Ended"));
}

#[test]
fn the_inspector_matches_its_golden() {
    let fixture = Fixture::new();
    let snapshot = family(&fixture);
    let mut view = inspector(&fixture, &snapshot);
    for (scheme, text_size, name) in [
        (Scheme::Light, TextSize::DEFAULT, "inspector"),
        (Scheme::Dark, TextSize::DEFAULT, "inspector-dark"),
        (Scheme::Light, TextSize::from_percent(150), "inspector-150"),
    ] {
        let palette = Palette::for_scheme(scheme);
        view.set_appearance(Appearance {
            scheme,
            text_size,
            ..Appearance::default()
        });
        let scene = view.scene(SIZE.0, SIZE.1);
        let scene = framed_scene(
            &scene,
            &view.title(),
            Some(FrameRegions::default()),
            view.header(),
            &palette,
        );
        check_golden(name, &scene);
    }
}

#[test]
fn escape_closes_the_inspector() {
    let fixture = Fixture::new();
    let snapshot = family(&fixture);
    let mut view = inspector(&fixture, &snapshot);
    assert!(!view.finished());
    view.handle(&Event::Key {
        key: Key::Escape,
        modifiers: Modifiers::default(),
    });
    assert!(view.finished());
}
