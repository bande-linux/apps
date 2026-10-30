#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::path::PathBuf;

use accesskit::Role;
use fixture::{Fixture, TICKS_PER_SECOND, Task};
use unbound_monitor::{
    Icons, InspectorView, OpenFile, Request, Screen, Section, Snapshot, read_details,
};
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer};
use unbound_toolkit::{
    CpuRenderer, FrameRegions, Palette, Point, Scheme, TextSystem, framed_scene,
};

const SIZE: (u32, u32) = (680, 520);
const MIN_SSIM: f64 = 0.995;

fn family(fixture: &Fixture) -> Snapshot {
    fixture.spawn(&Task {
        pid: 300,
        comm: "unbound-shell",
        command: &["/usr/bin/unbound-shell"],
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

fn bash(snapshot: &Snapshot) -> &unbound_monitor::Process {
    snapshot
        .processes
        .iter()
        .find(|process| process.pid == 400)
        .expect("bash")
}

fn inspector(fixture: &Fixture, snapshot: &Snapshot, scheme: Scheme) -> InspectorView {
    let mut view = InspectorView::new(
        bash(snapshot),
        fixture.path(),
        TICKS_PER_SECOND,
        Palette::for_scheme(scheme),
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
        .filter_map(|(_, node)| node.value().map(str::to_owned))
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

#[test]
fn the_inspector_links_the_family_and_shows_each_section() {
    let fixture = Fixture::new();
    let snapshot = family(&fixture);
    let mut view = inspector(&fixture, &snapshot, Scheme::Light);
    let shown = cells(&view);
    for value in ["unbound-shell (300)", "vim (500)", "/bin/bash --login"] {
        assert!(
            shown.iter().any(|cell| cell == value),
            "{value} in {shown:?}"
        );
    }
    let parent = rows(&view)
        .iter()
        .position(|row| row == "Parent")
        .expect("parent row");
    let row = view
        .table()
        .expect("table")
        .row_bounds(parent)
        .expect("row");
    let pointer = Pointer::at(Point::new(row.x + 200.0, row.y + row.height / 2.0)).with_count(2);
    view.handle(&Event::PointerDown(pointer));
    view.handle(&Event::PointerUp(pointer));
    assert_eq!(view.take_requests(), [Request::Inspect(300)]);

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
fn the_inspector_matches_its_goldens_in_light_and_dark() {
    let fixture = Fixture::new();
    let snapshot = family(&fixture);
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut view = inspector(&fixture, &snapshot, scheme);
        let scene = view.scene(SIZE.0, SIZE.1);
        let scene = framed_scene(
            &scene,
            &view.title(),
            Some(FrameRegions::default()),
            view.header(),
            &Palette::for_scheme(scheme),
        );
        for (scale, scale_name) in [(1.0, ""), (1.5, "@1.5x")] {
            let pixels = (
                (f64::from(scene.width) * scale).round() as u32,
                (f64::from(scene.height) * scale).round() as u32,
            );
            let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
            CpuRenderer::new()
                .render_into(&scene, scale, &mut rgba)
                .expect("frame");
            let name = format!("inspector-{suffix}{scale_name}");
            let actual = image::DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(pixels.0, pixels.1, rgba).expect("frame"),
            )
            .to_rgb8();
            let path =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
            if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
                actual.save(&path).expect("write golden");
            }
            let expected = image::open(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                .to_rgb8();
            let similarity =
                image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
            assert!(
                similarity.score >= MIN_SSIM,
                "{name}: {:.4}",
                similarity.score
            );
        }
    }
}
