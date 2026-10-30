#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::path::PathBuf;

use fixture::{entry, process};
use unbound_monitor::{
    AppList, AppRow, Catalog, CpuShare, Disclosure, ForceQuitView, Icons, Memory, Monitor,
    MonitorView, Process, ROW_HEIGHT, Request, Screen, Snapshot, Tab,
};
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer, Scroll};
use unbound_toolkit::{CpuRenderer, Image, Palette, Point, Scheme, TextSystem};

const MIN_SSIM: f64 = 0.995;
const MONITOR_SIZE: (u32, u32) = (860, 420);
const FORCE_QUIT_SIZE: (u32, u32) = (380, 340);

fn catalog() -> Catalog {
    let mut terminal = entry("org.unbound.Terminal", "Terminal", "unbound-term", false);
    terminal.icon = Some("org.unbound.Terminal".to_owned());
    let mut files = entry("org.unbound.Files", "Files", "unbound-files", false);
    files.icon = Some("org.unbound.Files".to_owned());
    let mut settings = entry(
        "org.unbound.Settings",
        "Settings",
        "unbound-settings",
        false,
    );
    settings.icon = Some("org.unbound.Settings".to_owned());
    Catalog::new(&[terminal, files, settings])
}

fn icons() -> Icons {
    Icons::new(|name| {
        let app = match name {
            "org.unbound.Terminal" => "terminal",
            "org.unbound.Files" => "files",
            _ => "settings",
        };
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../{app}/data/{name}.svg"));
        Image::load(&path, 32)
    })
}

fn measured(mut process: Process, cpu_percent: f32, megabytes: u64, threads: u32) -> Process {
    process.cpu_percent = cpu_percent;
    process.cpu_seconds = f64::from(cpu_percent) * 37.0;
    process.resident = megabytes * 1_000_000;
    process.threads = threads;
    process
}

fn snapshot(step: usize) -> Snapshot {
    let wave = (step as f32 * 0.7).sin().abs();
    Snapshot {
        seconds: 2.0,
        cpu: CpuShare {
            user: 0.1 + 0.3 * wave,
            system: 0.05 + 0.05 * wave,
            idle: 0.85 - 0.35 * wave,
        },
        processes: vec![
            measured(process(500, 1, "/usr/bin/unbound-term"), 4.2, 84, 6),
            measured(process(501, 500, "/bin/bash"), 0.0, 5, 1),
            measured(process(502, 501, "/usr/bin/cargo"), 96.5, 410, 18),
            measured(process(600, 1, "/usr/bin/unbound-files"), 1.3, 120, 4),
            measured(process(650, 1, "/usr/bin/unbound-settings"), 0.4, 70, 3),
            measured(
                process(700, 1, "/usr/lib/systemd/systemd-journald"),
                0.2,
                30,
                1,
            ),
            measured(process(710, 1, "/usr/bin/pipewire"), 2.1, 22, 3),
        ],
        memory: Memory {
            total: 16_000_000_000,
            used: 6_400_000_000,
            cached: 3_100_000_000,
            swap_total: 8_000_000_000,
            swap_used: 120_000_000,
            compressed: 450_000_000,
            pressure: 0.0,
        },
        ..Snapshot::default()
    }
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers::default(),
    }
}

fn render(screen: &mut impl Screen, name: &str, size: (u32, u32)) {
    let scene = screen.scene(size.0, size.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden(name, size, frame.rgba);
}

#[test]
fn activity_monitor_matches_its_goldens_in_light_and_dark() {
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut view = MonitorView::new(
            Monitor::new(catalog()),
            Palette::for_scheme(scheme),
            TextSystem::new(),
            icons(),
        );
        for step in 0..40 {
            view.update(snapshot(step));
        }
        view.handle(&key(Key::Down));
        view.handle(&key(Key::Right));
        view.handle(&key(Key::Down));
        render(&mut view, &format!("monitor-cpu-{suffix}"), MONITOR_SIZE);
        view.handle(&Event::Key {
            key: Key::Character('q'),
            modifiers: Modifiers {
                logo: true,
                alt: true,
                ..Modifiers::default()
            },
        });
        render(&mut view, &format!("monitor-quit-{suffix}"), MONITOR_SIZE);
    }
}

#[test]
fn force_quit_matches_its_goldens_in_light_and_dark() {
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut view = ForceQuitView::new(
            AppList::new(catalog(), 1000),
            Palette::for_scheme(scheme),
            TextSystem::new(),
            icons(),
        );
        view.update(snapshot(0));
        assert!(view.set_responding(500, "org.unbound.Terminal", false));
        view.handle(&key(Key::Down));
        view.handle(&key(Key::Down));
        render(&mut view, &format!("force-quit-{suffix}"), FORCE_QUIT_SIZE);
    }
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

fn monitor_view() -> MonitorView {
    let mut view = MonitorView::new(
        Monitor::new(catalog()),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        icons(),
    );
    view.update(snapshot(0));
    view
}

#[test]
fn the_memory_tab_resamples_at_once_to_show_proportional_memory() {
    let mut view = monitor_view();
    assert!(view.handle(&command('2')));
    assert_eq!(view.monitor.tab(), Tab::Memory);
    assert!(view.measures_proportional_memory());
    assert_eq!(view.take_requests(), [Request::Resample]);
    assert!(view.handle(&command('1')));
    assert_eq!(view.take_requests(), [Request::Resample]);
}

#[test]
fn super_f_searches_as_you_type_and_escape_clears_the_search() {
    let mut view = monitor_view();
    assert!(!view.handle(&key(Key::Character('p'))));
    view.handle(&command('f'));
    for character in "pipe".chars() {
        view.handle(&key(Key::Character(character)));
    }
    let names = |view: &MonitorView| {
        view.monitor
            .rows()
            .iter()
            .map(|row| row.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&view), ["pipewire"]);
    view.handle(&key(Key::Enter));
    assert_eq!(view.monitor.selected_index(), Some(0));
    view.handle(&key(Key::Escape));
    assert_eq!(names(&view).len(), 5);
}

#[test]
fn the_wheel_scrolls_the_list_under_the_pointer_and_a_double_click_opens_an_app() {
    let mut view = monitor_view();
    let mut crowded = snapshot(0);
    crowded
        .processes
        .extend((0..30).map(|index| process(2000 + index, 1, &format!("/usr/bin/daemon{index}"))));
    view.update(crowded);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    let list_top = 48.0 + 26.0;
    let at = Point::new(300.0, list_top + 5.0);
    assert!(view.handle(&Event::Scroll(Scroll {
        position: at,
        delta: Point::new(0.0, 2.0 * ROW_HEIGHT),
        ..Scroll::default()
    })));
    view.handle(&Event::PointerDown(Pointer::at(at)));
    assert_eq!(view.monitor.selected_index(), Some(2));

    assert_eq!(view.monitor.rows()[0].name, "Terminal");
    view.handle(&Event::Scroll(Scroll {
        position: at,
        delta: Point::new(0.0, -10.0 * ROW_HEIGHT),
        ..Scroll::default()
    }));
    view.handle(&Event::PointerDown(Pointer::at(at).with_count(2)));
    assert_eq!(view.monitor.rows()[0].disclosure, Disclosure::Expanded);
}

fn right_click(at: Point, alt: bool) -> Event {
    let mut pointer = Pointer::at(at);
    pointer.button = unbound_toolkit::ui::PointerButton::Secondary;
    pointer.modifiers.alt = alt;
    Event::PointerDown(pointer)
}

fn menu_actions(requests: Vec<Request>) -> Vec<(String, bool)> {
    match requests.as_slice() {
        [Request::Menu(menu, _)] => menu
            .items
            .iter()
            .filter(|item| !item.separator)
            .map(|item| (item.action.clone(), item.enabled))
            .collect(),
        other => panic!("expected one menu, got {other:?}"),
    }
}

#[test]
fn a_right_click_selects_the_row_and_offers_quit_or_with_alt_force_quit() {
    let mut view = monitor_view();
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    let row = Point::new(300.0, 48.0 + 26.0 + ROW_HEIGHT / 2.0);
    assert!(view.handle(&right_click(row, false)));
    assert_eq!(view.monitor.selected_index(), Some(0));
    let name = view.monitor.rows()[0].name.clone();
    assert_eq!(
        menu_actions(view.take_requests()),
        [
            ("quit".to_owned(), true),
            ("show-in-files".to_owned(), true),
            ("copy-name".to_owned(), true),
            ("copy-pid".to_owned(), true),
        ]
    );
    view.handle(&right_click(row, true));
    assert_eq!(menu_actions(view.take_requests())[0].0, "force-quit");

    view.menu_action("copy-name");
    assert_eq!(view.take_requests(), [Request::Copy(name)]);
    assert!(view.menu_action("force-quit"));
    view.handle(&key(Key::Escape));
    view.handle(&Event::Key {
        key: Key::Menu,
        modifiers: Modifiers::default(),
    });
    assert_eq!(menu_actions(view.take_requests())[0].0, "quit");
}

#[test]
fn force_quit_marks_an_app_that_stopped_responding_until_it_answers_again() {
    let mut apps = AppList::new(catalog(), 1000);
    apps.update(snapshot(0));
    let responding = |apps: &AppList| {
        apps.rows()
            .map(
                |AppRow {
                     name, responding, ..
                 }| (name.to_owned(), responding),
            )
            .collect::<Vec<_>>()
    };
    assert!(apps.set_responding(501, "", false));
    assert!(!apps.set_responding(501, "", false));
    assert_eq!(
        responding(&apps),
        [
            ("Files".to_owned(), true),
            ("Settings".to_owned(), true),
            ("Terminal".to_owned(), false),
        ]
    );
    assert!(apps.set_responding(501, "", true));
    assert!(responding(&apps).iter().all(|(_, responding)| *responding));
}
