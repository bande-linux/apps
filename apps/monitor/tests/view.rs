#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::path::PathBuf;

use fixture::{entry, process};
use unbound_monitor::{
    AppList, Catalog, CpuShare, ForceQuitView, Icons, Memory, Monitor, MonitorView, Process,
    Request, Screen, Snapshot, Tab,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{CpuRenderer, Image, Palette, Scheme, TextSystem};

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
