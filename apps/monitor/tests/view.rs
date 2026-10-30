#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use accesskit::Role;
use fixture::{entry, process};
use unbound_monitor::{
    AppList, AppRow, Catalog, CpuShare, ForceQuitView, Icons, Memory, Monitor, MonitorView,
    Process, Request, RowKey, Screen, Snapshot, Tab,
};
use unbound_toolkit::ui::{
    Event, HEADER_HEIGHT, Key, Modifiers, Pointer, PointerButton, ROW_HEIGHT, Scroll,
};
use unbound_toolkit::{
    CpuRenderer, FrameRegions, Image, Palette, Point, Scheme, TextSystem, framed_scene,
};

const MIN_SSIM: f64 = 0.995;
const MONITOR_SIZE: (u32, u32) = (860, 420);
const FORCE_QUIT_SIZE: (u32, u32) = (380, 340);
const LIST_TOP: f32 = 48.0 + HEADER_HEIGHT;
const TITLE: &str = "Activity Monitor";

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
        let theme = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/icons/Unbound");
        let path = match name {
            "dialog-information-symbolic" => theme.join(format!("symbolic/status/{name}.svg")),
            "org.unbound.Terminal"
            | "org.unbound.Files"
            | "org.unbound.Settings"
            | "org.unbound.ActivityMonitor" => theme.join(format!("scalable/apps/{name}.svg")),
            _ => return None,
        };
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

fn shifted(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    }
}

fn render(screen: &mut impl Screen, scheme: Scheme, title: &str, name: &str, size: (u32, u32)) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut scene = screen.scene(size.0, size.1);
    while screen.animating() {
        assert!(Instant::now() < deadline, "the view did not settle");
        std::thread::sleep(Duration::from_millis(10));
        scene = screen.scene(size.0, size.1);
    }
    let header = screen.header();
    let regions = header.as_ref().map(|_| FrameRegions::default());
    let scene = framed_scene(&scene, title, regions, header, &Palette::for_scheme(scheme));
    for (scale, suffix) in [(1.0, ""), (1.5, "@1.5x")] {
        let pixels = (
            (f64::from(scene.width) * scale).round() as u32,
            (f64::from(scene.height) * scale).round() as u32,
        );
        let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
        CpuRenderer::new()
            .render_into(&scene, scale, &mut rgba)
            .expect("frame");
        check_golden(&format!("{name}{suffix}"), pixels, rgba);
    }
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
        view.handle(&key(Key::Enter));
        view.handle(&key(Key::Down));
        assert!(view.set_responding(710, "", false));
        render(
            &mut view,
            scheme,
            TITLE,
            &format!("monitor-cpu-{suffix}"),
            MONITOR_SIZE,
        );
        view.handle(&Event::Key {
            key: Key::Character('q'),
            modifiers: Modifiers {
                logo: true,
                alt: true,
                ..Modifiers::default()
            },
        });
        render(
            &mut view,
            scheme,
            TITLE,
            &format!("monitor-quit-{suffix}"),
            MONITOR_SIZE,
        );
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
        render(
            &mut view,
            scheme,
            "Force Quit Apps",
            &format!("force-quit-{suffix}"),
            FORCE_QUIT_SIZE,
        );
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
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    view
}

fn shown_rows(screen: &impl Screen) -> Vec<String> {
    let tree = screen.accessibility().expect("an accessibility tree");
    let mut rows = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Row)
        .filter_map(|(_, node)| Some((node.row_index()?, node.label()?.to_owned())))
        .collect::<Vec<_>>();
    rows.sort();
    rows.into_iter().map(|(_, label)| label).collect()
}

fn row_point(index: usize) -> Point {
    Point::new(
        300.0,
        LIST_TOP + index as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0,
    )
}

fn click(view: &mut impl Screen, at: Point, modifiers: Modifiers, count: u8) {
    let pointer = Pointer::at(at).with_modifiers(modifiers).with_count(count);
    view.handle(&Event::PointerDown(pointer));
    view.handle(&Event::PointerUp(pointer));
}

fn cargo() -> RowKey {
    RowKey::Process {
        pid: 502,
        start_ticks: 502,
    }
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
fn another_tab_shows_its_own_columns_sorted_by_its_measure_and_keeps_the_selection() {
    let mut view = monitor_view();
    view.handle(&key(Key::Down));
    view.handle(&command('2'));
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    let tree = view.accessibility().expect("an accessibility tree");
    let headers = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::ColumnHeader)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        headers,
        ["Name", "Memory", "Resident", "Threads", "PID", "User"]
    );
    assert_eq!(
        shown_rows(&view),
        [
            "Terminal",
            "Files",
            "Settings",
            "systemd-journald",
            "pipewire"
        ]
    );
    assert_eq!(
        view.table().expect("table").selection(),
        ["app:org.unbound.Terminal"]
    );
    view.handle(&Event::Key {
        key: Key::Character('r'),
        modifiers: Modifiers {
            logo: true,
            shift: true,
            ..Modifiers::default()
        },
    });
    assert!(matches!(
        view.take_requests().as_slice(),
        [Request::Resample, Request::ShowInFiles(_)]
    ));
}

#[test]
fn super_f_searches_as_you_type_and_escape_clears_the_search() {
    let mut view = monitor_view();
    view.handle(&command('f'));
    for character in "pipe".chars() {
        view.handle(&key(Key::Character(character)));
    }
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(shown_rows(&view), ["pipewire"]);
    view.handle(&key(Key::Enter));
    assert_eq!(view.table().expect("table").selection().len(), 1);
    view.handle(&key(Key::Escape));
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(shown_rows(&view).len(), 5);
}

#[test]
fn searching_for_a_process_opens_its_app_and_clearing_the_search_closes_it_again() {
    let mut view = monitor_view();
    view.handle(&command('f'));
    for character in "cargo".chars() {
        view.handle(&key(Key::Character(character)));
    }
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(shown_rows(&view), ["Terminal", "cargo"]);
    view.handle(&key(Key::Escape));
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(
        shown_rows(&view),
        [
            "Terminal",
            "pipewire",
            "Files",
            "Settings",
            "systemd-journald"
        ]
    );
}

#[test]
fn the_selection_stays_on_its_process_while_the_numbers_reorder_the_list() {
    let mut view = monitor_view();
    click(&mut view, row_point(0), Modifiers::default(), 2);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(
        shown_rows(&view)[..4],
        ["Terminal", "cargo", "unbound-term", "bash"]
    );
    click(&mut view, row_point(1), Modifiers::default(), 1);
    assert_eq!(view.monitor.selection(), [cargo()]);
    let mut calmer = snapshot(1);
    calmer.processes[2].cpu_percent = 0.1;
    calmer.processes[3].cpu_percent = 50.0;
    view.update(calmer);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(shown_rows(&view)[..2], ["Files", "Terminal"]);
    assert_eq!(view.monitor.selection(), [cargo()]);
    let table = view.table().expect("table");
    assert_eq!(table.selection(), ["process:502:502"]);
    let shown = table.cursor_row().expect("cursor");
    assert_eq!(shown_rows(&view)[shown], "cargo");
}

#[test]
fn header_clicks_sort_numbers_largest_first_and_names_from_a() {
    let mut view = monitor_view();
    let header = |x: f32| Point::new(x, 48.0 + HEADER_HEIGHT / 2.0);
    click(&mut view, header(40.0), Modifiers::default(), 1);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(
        shown_rows(&view),
        [
            "Files",
            "pipewire",
            "Settings",
            "systemd-journald",
            "Terminal"
        ]
    );
    let threads = 854.0 - 96.0 - 64.0 - 42.0;

    click(&mut view, header(threads), Modifiers::default(), 1);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(shown_rows(&view)[0], "Terminal");
    assert_eq!(
        view.monitor.sort(),
        (unbound_monitor::Column::Threads, true)
    );
}

#[test]
fn several_selected_rows_quit_together() {
    let mut view = monitor_view();
    view.handle(&key(Key::Down));
    view.handle(&shifted(Key::Down));
    let target = view.monitor.stop_target().expect("two apps");
    assert_eq!(target.count, 2);
    assert_eq!(target.roots, [(500, 500), (710, 710)]);
    assert!(view.handle(&Event::Key {
        key: Key::Character('q'),
        modifiers: Modifiers {
            logo: true,
            alt: true,
            ..Modifiers::default()
        },
    }));
}

#[test]
fn the_wheel_scrolls_the_list_under_the_pointer() {
    let mut view = monitor_view();
    let mut crowded = snapshot(0);
    crowded
        .processes
        .extend((0..30).map(|index| process(2000 + index, 1, &format!("/usr/bin/daemon{index}"))));
    view.update(crowded);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert!(view.handle(&Event::Scroll(Scroll {
        position: row_point(0),
        delta: Point::new(0.0, 4.0 * ROW_HEIGHT),
        ..Scroll::default()
    })));
    let deadline = Instant::now() + Duration::from_secs(2);
    while view.table().expect("table").scroll_offset() < 4.0 * ROW_HEIGHT - 0.5 {
        assert!(Instant::now() < deadline, "the list did not scroll");
        std::thread::sleep(Duration::from_millis(10));
        view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    }
}

fn right_click(at: Point) -> Event {
    let mut pointer = Pointer::at(at);
    pointer.button = PointerButton::Secondary;
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
fn a_right_click_selects_the_row_and_offers_quit_turning_into_force_quit_with_alt() {
    let mut view = monitor_view();
    assert!(view.handle(&right_click(row_point(0))));
    assert_eq!(view.table().expect("table").selection().len(), 1);
    let name = shown_rows(&view)[0].clone();
    assert_eq!(
        menu_actions(view.take_requests()),
        [
            ("quit".to_owned(), true),
            ("inspect".to_owned(), true),
            ("show-in-files".to_owned(), true),
            ("copy-name".to_owned(), true),
            ("copy-pid".to_owned(), true),
        ]
    );
    view.handle(&right_click(row_point(0)));
    match view.take_requests().as_slice() {
        [Request::Menu(menu, _)] => assert_eq!(
            menu.items[0]
                .alternate
                .as_ref()
                .map(|item| item.action.as_str()),
            Some("force-quit")
        ),
        other => panic!("expected one menu, got {other:?}"),
    }

    view.menu_action("copy-name");
    assert_eq!(view.take_requests(), [Request::Copy(name)]);
    assert!(view.menu_action("force-quit"));
    view.handle(&key(Key::Escape));
    view.handle(&key(Key::Menu));
    assert_eq!(menu_actions(view.take_requests())[0].0, "quit");
}

#[test]
fn an_app_that_stopped_responding_is_marked_in_the_list_until_it_answers_again() {
    let mut view = monitor_view();
    assert!(view.set_responding(600, "org.unbound.Files", false));
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert!(shown_rows(&view).contains(&"Files (Not Responding)".to_owned()));
    assert!(view.set_responding(600, "org.unbound.Files", true));
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert!(shown_rows(&view).contains(&"Files".to_owned()));
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

#[test]
fn force_quit_lists_apps_for_assistive_technology_and_force_quits_the_selected_ones() {
    let mut view = ForceQuitView::new(
        AppList::new(catalog(), 1000),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        icons(),
    );
    view.update(snapshot(0));
    view.scene(FORCE_QUIT_SIZE.0, FORCE_QUIT_SIZE.1);
    assert_eq!(shown_rows(&view), ["Files", "Settings", "Terminal"]);
    view.handle(&key(Key::Down));
    view.handle(&shifted(Key::Down));
    assert_eq!(view.table().expect("table").selection().len(), 2);
    assert!(view.handle(&key(Key::Enter)));
    view.handle(&key(Key::Escape));
    assert!(!view.finished());
    view.handle(&key(Key::Escape));
    assert!(view.finished());
}

fn column_headers(view: &MonitorView) -> Vec<String> {
    let tree = view.accessibility().expect("an accessibility tree");
    tree.nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::ColumnHeader)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

#[test]
fn a_right_click_on_the_header_chooses_columns_and_the_choice_is_remembered() {
    let mut view = monitor_view();
    assert!(view.handle(&right_click(Point::new(300.0, 48.0 + HEADER_HEIGHT / 2.0))));
    let shown = menu_actions(view.take_requests());
    assert_eq!(shown[0], ("column:name".to_owned(), false));
    assert!(shown.contains(&("column:resident".to_owned(), true)));
    assert!(view.menu_action("column:user"));
    assert!(view.menu_action("column:resident"));
    assert!(view.menu_action("column:cpu"));
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(
        column_headers(&view),
        ["Name", "CPU Time", "Threads", "PID", "Resident"]
    );
    assert_eq!(view.monitor.sort(), (unbound_monitor::Column::Name, false));
    let remembered = match view.take_requests().last() {
        Some(Request::SaveColumns(text)) => text.clone(),
        other => panic!("expected the columns to be saved, got {other:?}"),
    };
    let mut again = Monitor::new(catalog());
    again.choose_columns(&remembered);
    assert_eq!(again.columns(), view.monitor.columns());
}

#[test]
fn the_gpu_tab_appears_once_the_system_reports_gpu_use() {
    let mut view = monitor_view();
    assert!(!view.handle(&command('5')));
    let mut busy = snapshot(1);
    busy.gpu = true;
    busy.processes[3].gpu = Some(unbound_monitor::GpuUse {
        percent: 30.0,
        seconds: 4.0,
        memory: 64_000_000,
    });
    view.update(busy);
    assert!(view.handle(&command('5')));
    assert_eq!(view.monitor.tab(), Tab::Gpu);
    assert!(view.measures_gpu());
    assert_eq!(view.take_requests(), [Request::Resample]);
    view.scene(MONITOR_SIZE.0, MONITOR_SIZE.1);
    assert_eq!(shown_rows(&view)[0], "Files");
    assert_eq!(
        column_headers(&view),
        ["Name", "% GPU", "GPU Time", "GPU Memory", "PID", "User"]
    );
}
