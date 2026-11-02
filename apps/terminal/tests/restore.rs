#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use bande_terminal::{SavedWindow, claim, left_behind, window_file};

#[test]
fn windows_left_behind_are_restored_once_and_running_ones_are_left_alone() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    let saved = SavedWindow {
        tabs: vec![PathBuf::from("/tmp"), PathBuf::from("/usr/share")],
        active: 1,
    };
    for file in [
        window_file(dir, 10, 0),
        window_file(dir, 10, 1),
        window_file(dir, 11, 0),
    ] {
        saved.save(&file).unwrap();
    }
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    let left = left_behind(dir, |pid| pid == 11);
    assert_eq!(left, [window_file(dir, 10, 0), window_file(dir, 10, 1)]);
    let mine = window_file(dir, 12, 0);
    assert_eq!(claim(&left[0], &mine), Some(saved));
    assert_eq!(claim(&left[0], &mine), None);
    assert_eq!(left_behind(dir, |pid| pid == 11 || pid == 12), left[1..]);
    assert_eq!(SavedWindow::parse("active 5\ntab relative\n"), None);
    let clamped = SavedWindow::parse("active 5\ntab /srv\n").map(|window| window.active);
    assert_eq!(clamped, Some(0));
}
