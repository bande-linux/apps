#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use bande_terminal::{SavedWindow, claim, left_behind, window_file};

#[test]
fn windows_left_behind_are_restored_once_and_running_ones_are_left_alone() {
    let folder = tempfile::tempdir().expect("folder");
    let saved = SavedWindow {
        tabs: vec![PathBuf::from("/tmp"), PathBuf::from("/usr/share")],
        active: 1,
    };
    saved
        .save(&window_file(folder.path(), 10, 0))
        .expect("save");
    saved
        .save(&window_file(folder.path(), 10, 1))
        .expect("save");
    saved
        .save(&window_file(folder.path(), 11, 0))
        .expect("save");
    saved.save(&folder.path().join("9.window")).expect("save");
    std::fs::write(folder.path().join("notes.txt"), "x").expect("write");
    let left = left_behind(folder.path(), |pid| pid == 11);
    assert_eq!(
        left,
        [
            window_file(folder.path(), 10, 0),
            window_file(folder.path(), 10, 1),
            folder.path().join("9.window"),
        ]
    );
    let mine = window_file(folder.path(), 12, 0);
    assert_eq!(claim(&left[0], &mine), Some(saved));
    assert_eq!(claim(&left[0], &mine), None);
    assert_eq!(
        left_behind(folder.path(), |pid| pid == 11 || pid == 12),
        left[1..]
    );
    assert_eq!(SavedWindow::parse("active 5\ntab relative\n"), None);
    assert_eq!(
        SavedWindow::parse("active 5\ntab /srv\n").map(|window| window.active),
        Some(0)
    );
}
