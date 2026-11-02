#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use bande_installer::{identity, read};

#[test]
fn a_built_deb_opens_with_its_fields_and_a_truncated_download_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tool");
    std::fs::create_dir_all(root.join("DEBIAN")).unwrap();
    std::fs::write(
        root.join("DEBIAN/control"),
        "Package: tool\nVersion: 1.2-1\nArchitecture: arm64\nMaintainer: Ada <ada@example.org>\n\
         Installed-Size: 2048\nDescription: A handy tool\n Does handy things.\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("usr/share/applications")).unwrap();
    std::fs::write(
        root.join("usr/share/applications/tool.desktop"),
        "[Desktop Entry]\nType=Application\nName=Handy Tool\nExec=tool\nIcon=tool\n",
    )
    .unwrap();
    let icons = root.join("usr/share/icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&icons).unwrap();
    std::fs::write(
        icons.join("tool.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><rect width=\"16\" height=\"16\"/></svg>",
    )
    .unwrap();
    let deb = dir.path().join("tool_1.2-1_arm64.deb");
    let built = Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(&root)
        .arg(&deb)
        .output()
        .unwrap();
    assert!(built.status.success());
    let opened = read(&deb).unwrap();
    assert_eq!(
        (opened.name.as_str(), opened.version.as_str()),
        ("tool", "1.2-1")
    );
    assert_eq!(
        (opened.arch.as_str(), opened.installed_size),
        ("arm64", 2048 * 1024)
    );
    assert_eq!(opened.summary, "A handy tool");
    let shown = identity(&deb).unwrap();
    assert_eq!(shown.name, "Handy Tool");
    assert!(shown.icon.is_some());
    let bytes = std::fs::read(&deb).unwrap();
    let truncated = dir.path().join("partial.deb");
    std::fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
    assert_eq!(read(&truncated), None);
    assert_eq!(identity(&truncated), None);
}
