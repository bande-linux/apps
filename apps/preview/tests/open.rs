#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use bande_preview::PreviewView;
use bande_toolkit::Appearance;
use image::codecs::gif::GifEncoder;
use image::{Frame, RgbaImage};

fn settle(view: &mut PreviewView) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    while view.is_loading() {
        assert!(Instant::now() < deadline, "the file never finished opening");
        view.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    view.paint(800, 600);
    format!("{:?}", view.ui_ref().accessibility())
}

fn write_gif(path: &Path) {
    let mut encoder = GifEncoder::new(std::fs::File::create(path).unwrap());
    let frames = [[255, 0, 0, 255], [0, 0, 255, 255]]
        .map(|color| Frame::new(RgbaImage::from_pixel(30, 20, image::Rgba(color))));
    encoder.encode_frames(frames).unwrap();
}

#[test]
fn a_folder_steps_through_pictures_and_hostile_files_never_panic_hang_or_wake() {
    let folder = tempfile::tempdir().unwrap();
    let at = |name: &str| folder.path().join(name);
    RgbaImage::from_pixel(64, 48, image::Rgba([10, 200, 30, 255]))
        .save(at("a.png"))
        .unwrap();
    write_gif(&at("b.gif"));
    let mut png = Vec::new();
    RgbaImage::from_pixel(400, 300, image::Rgba([90, 90, 90, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(at("c.png"), &png[..png.len() / 2]).unwrap();
    let mut bomb = std::fs::read(at("b.gif")).unwrap();
    bomb[6..10].copy_from_slice(&[255; 4]);
    std::fs::write(at("d.gif"), bomb).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(at("e.jpg"))
            .status()
            .unwrap()
            .success()
    );

    let mut view = PreviewView::new(Appearance::default(), vec![at("a.png")]);
    assert!(settle(&mut view).contains("a.png, 64 × 48"));
    assert_eq!(
        view.deadline(),
        None,
        "a still picture schedules no wake-ups"
    );

    view.go(true);
    let shown = settle(&mut view);
    assert!(shown.contains("b.gif, 30 × 20"), "{shown}");
    assert!(view.is_animated() && view.deadline().is_some());
    let first = view.deadline().unwrap();
    assert!(view.tick(first + Duration::from_millis(1)));
    view.toggle_playing();
    assert_eq!(
        view.deadline(),
        None,
        "a paused animation schedules no wake-ups"
    );

    view.go(true);
    assert!(settle(&mut view).contains("“c.png” could not be opened."));
    view.go(true);
    let refused = settle(&mut view);
    assert!(
        refused.contains("too large to open (65535 × 65535 pixels)"),
        "{refused}"
    );
    view.go(true);
    assert!(settle(&mut view).contains("“e.jpg” could not be opened."));
    view.go(true);
    assert!(
        settle(&mut view).contains("a.png, 64 × 48"),
        "the folder wraps around"
    );
}
