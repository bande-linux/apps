#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use bande_installer::{identity, read};

const FILES: [(&str, &str); 3] = [
    (
        "DEBIAN/control",
        "Package: tool\nVersion: 1.2-1\nArchitecture: arm64\nMaintainer: Ada <ada@example.org>\n\
         Installed-Size: 2048\nDescription: A handy tool\n Does handy things.\n",
    ),
    (
        "usr/share/applications/tool.desktop",
        "[Desktop Entry]\nType=Application\nName=Handy Tool\nExec=tool\nIcon=tool\n",
    ),
    (
        "usr/share/icons/hicolor/scalable/apps/tool.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><rect width=\"16\" height=\"16\"/></svg>",
    ),
];

#[test]
fn a_built_deb_opens_with_its_fields_and_a_truncated_download_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tool");
    for (path, text) in FILES {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let deb = dir.path().join("tool_1.2-1_arm64.deb");
    let mut build = Command::new("dpkg-deb");
    build
        .args(["--build", "--root-owner-group"])
        .arg(&root)
        .arg(&deb);
    assert!(build.output().unwrap().status.success());
    let opened = read(&deb).unwrap();
    assert_eq!(
        (
            opened.name.as_str(),
            opened.version.as_str(),
            opened.arch.as_str()
        ),
        ("tool", "1.2-1", "arm64")
    );
    assert_eq!(
        (opened.installed_size, opened.summary.as_str()),
        (2048 * 1024, "A handy tool")
    );
    let shown = identity(&deb).unwrap();
    assert_eq!(
        (shown.name.as_str(), shown.icon.is_some()),
        ("Handy Tool", true)
    );
    let bytes = std::fs::read(&deb).unwrap();
    let truncated = dir.path().join("partial.deb");
    std::fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
    assert_eq!((read(&truncated), identity(&truncated)), (None, None));
}
