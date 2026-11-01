#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_installer::{Installer, Machine, State, WINDOW_SIZE, missing_requirements, read};
use bande_packages::Client;
use bande_test_bus::SessionBus;
use bande_test_bus::packagekit::{FakePackage, FakePackageKit, SIMULATE, Script};
use bande_toolkit::{CpuRenderer, Palette, Scheme, TextSystem, WINDOW_HEADER_HEIGHT, framed_scene};

const WAIT: Duration = Duration::from_secs(10);

fn arm64() -> Machine {
    Machine {
        native: "arm64".to_owned(),
        foreign: Vec::new(),
    }
}

fn build_deb(dir: &Path, name: &str, version: &str, arch: &str) -> PathBuf {
    let root = dir.join(format!("{name}-{version}-{arch}"));
    std::fs::create_dir_all(root.join("DEBIAN")).expect("DEBIAN");
    std::fs::create_dir_all(root.join("usr/share/doc/tool")).expect("doc");
    std::fs::write(root.join("usr/share/doc/tool/README"), "hello\n").expect("readme");
    std::fs::write(
        root.join("DEBIAN/control"),
        format!(
            "Package: {name}\nVersion: {version}\nArchitecture: {arch}\nMaintainer: Ada Lovelace <ada@example.org>\nInstalled-Size: 2048\nHomepage: https://example.org/tool\nDescription: A handy tool\n Does handy things.\n .\n Second paragraph.\n"
        ),
    )
    .expect("control");
    let deb = dir.join(format!("{name}_{version}_{arch}.deb"));
    let status = Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(&root)
        .arg(&deb)
        .output()
        .expect("dpkg-deb")
        .status;
    assert!(status.success());
    deb
}

#[test]
fn a_built_deb_reads_back_its_fields_and_a_truncated_copy_is_invalid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let deb = build_deb(dir.path(), "tool", "1.2-1", "arm64");
    let read = read(&deb).expect("valid deb");
    assert_eq!(read.name, "tool");
    assert_eq!(read.version, "1.2-1");
    assert_eq!(read.arch, "arm64");
    assert_eq!(read.maintainer, "Ada Lovelace <ada@example.org>");
    assert_eq!(read.installed_size, 2048 * 1024);
    assert_eq!(read.homepage, "https://example.org/tool");
    assert_eq!(read.summary, "A handy tool");
    assert_eq!(read.description, "Does handy things.\n\nSecond paragraph.");

    let bytes = std::fs::read(&deb).expect("bytes");
    let truncated = dir.path().join("broken.deb");
    std::fs::write(&truncated, &bytes[..bytes.len() / 3]).expect("truncated");
    assert_eq!(bande_installer::read(&truncated), None);
}

#[test]
fn architectures_fit_when_native_all_or_foreign() {
    let mut machine = arm64();
    assert!(machine.fits("arm64"));
    assert!(machine.fits("all"));
    assert!(!machine.fits("amd64"));
    machine.foreign.push("amd64".to_owned());
    assert!(machine.fits("amd64"));
}

#[test]
fn unmet_dependencies_name_what_is_missing() {
    let details = "The following packages have unmet dependencies:\n  tool: Depends: libnope (>= 2) but it is not installable\n        PreDepends: other-missing but it is not going to be installed\n";
    assert_eq!(
        missing_requirements(details),
        ["libnope (>= 2)".to_owned(), "other-missing".to_owned()]
    );
}

struct Window {
    installer: Installer,
    _bus: SessionBus,
    fake: FakePackageKit,
    _dir: tempfile::TempDir,
}

impl Window {
    fn open(script: Script, version: &str, arch: &str) -> Self {
        Self::open_in(script, version, arch, Scheme::Light)
    }

    fn open_in(script: Script, version: &str, arch: &str, scheme: Scheme) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let deb = build_deb(dir.path(), "tool", version, arch);
        let bus = SessionBus::start().expect("dbus-daemon");
        let fake = FakePackageKit::start(&bus, script).expect("fake PackageKit");
        let system = bus.connect().expect("connection");
        let installer = Installer::new(
            Client::new(&system),
            deb,
            arm64(),
            Palette::for_scheme(scheme),
            TextSystem::new(),
        )
        .expect("installer");
        Self {
            installer,
            _bus: bus,
            fake,
            _dir: dir,
        }
    }

    fn texts(&mut self) -> Vec<(Role, String)> {
        self.installer.ui().scene(WINDOW_SIZE.0, WINDOW_SIZE.1);
        self.installer
            .accessibility()
            .nodes
            .into_iter()
            .map(|(_, node)| {
                let text = node
                    .label()
                    .or_else(|| node.value())
                    .unwrap_or_default()
                    .to_owned();
                (node.role(), text)
            })
            .collect()
    }

    fn shows(&mut self, text: &str) -> bool {
        self.texts().iter().any(|(_, shown)| shown.contains(text))
    }

    fn until(&mut self, done: impl Fn(&mut Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.installer.pump();
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "never got there: {:?}",
                self.texts()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn see(&mut self, text: &str) {
        self.until(|window| window.shows(text));
    }

    fn click(&mut self, button: &str) {
        self.installer.ui().scene(WINDOW_SIZE.0, WINDOW_SIZE.1);
        let target = self
            .installer
            .accessibility()
            .nodes
            .into_iter()
            .find(|(_, node)| node.role() == Role::Button && node.label() == Some(button))
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no {button} button in {:?}", self.texts()));
        self.installer.ui().perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: target,
            data: None,
        });
        self.installer.pump();
    }
}

fn changes() -> Vec<FakePackage> {
    vec![
        FakePackage::new(12, "tool;1.2-1;arm64;local", "", 0),
        FakePackage::new(12, "libhandy;3;arm64;debian", "", 0),
        FakePackage::new(11, "libother;2;arm64;debian", "", 0),
    ]
}

#[test]
fn a_new_package_previews_its_extras_then_installs_and_offers_its_app() {
    let dir = tempfile::tempdir().expect("apps");
    let applications = dir.path().join("share/applications");
    std::fs::create_dir_all(&applications).expect("applications");
    let visible = applications.join("tool.desktop");
    std::fs::write(
        &visible,
        "[Desktop Entry]\nType=Application\nName=Tool\nExec=true\n",
    )
    .expect("desktop");
    let hidden = applications.join("tool-helper.desktop");
    std::fs::write(
        &hidden,
        "[Desktop Entry]\nType=Application\nName=Helper\nExec=true\nNoDisplay=true\n",
    )
    .expect("hidden");
    let mut window = Window::open(
        Script {
            changes: changes(),
            files: vec![
                visible.display().to_string(),
                hidden.display().to_string(),
                "/usr/bin/tool".to_owned(),
            ],
            ..Script::default()
        },
        "1.2-1",
        "arm64",
    );
    window.see("Also installs 2 packages");
    assert!(window.shows("Version 1.2-1"));
    assert!(window.shows("doesn't come from your software sources"));
    window.click("Install");
    window.see("“tool” is installed.");
    let flags: Vec<u64> = window
        .fake
        .called("InstallFiles")
        .iter()
        .map(|call| call.flags)
        .collect();
    assert_eq!(flags, [SIMULATE, 0]);
    assert!(window.shows("Open Tool"));
    assert!(!window.shows("Open Helper"));
}

#[test]
fn an_older_file_is_a_downgrade_with_a_caution_and_the_same_version_is_present() {
    let installed = vec![FakePackage::new(1, "tool;2.0-1;arm64;installed", "", 0)];
    let mut older = Window::open(
        Script {
            installed: installed.clone(),
            changes: vec![FakePackage::new(20, "tool;1.2-1;arm64;local", "", 0)],
            ..Script::default()
        },
        "1.2-1",
        "arm64",
    );
    older.see("Downgrades from 2.0-1 to 1.2-1");
    assert!(older.shows("This is older than the installed version 2.0-1."));

    let mut same = Window::open(
        Script {
            installed,
            ..Script::default()
        },
        "2.0-1",
        "arm64",
    );
    same.see("Version 2.0-1 is installed.");
    assert!(same.fake.called("InstallFiles").is_empty());
}

#[test]
fn a_refused_password_returns_to_the_preview_and_a_lock_says_to_wait() {
    let mut window = Window::open(
        Script {
            changes: changes(),
            ..Script::default()
        },
        "1.2-1",
        "arm64",
    );
    window.see("Also installs 2 packages");
    window.fake.script(|script| {
        script
            .failures
            .insert("InstallFiles".to_owned(), (48, "Not authorized".to_owned()));
    });
    window.click("Install");
    window.until(|window| window.fake.called("InstallFiles").len() == 2);
    window.see("Also installs 2 packages");
    assert!(matches!(window.installer.state(), State::Ready(_)));
    window.fake.script(|script| {
        script
            .failures
            .insert("InstallFiles".to_owned(), (26, "locked".to_owned()));
    });
    window.click("Install");
    window.see("Another installation is running. Try again when it finishes.");
}

#[test]
fn missing_dependencies_block_the_install_with_their_names() {
    let mut window = Window::open(
        Script {
            failures: [(
                "InstallFiles".to_owned(),
                (
                    13,
                    "The following packages have unmet dependencies:\n  tool: Depends: libnope but it is not installable\n".to_owned(),
                ),
            )]
            .into(),
            ..Script::default()
        },
        "1.2-1",
        "arm64",
    );
    window.see("Can't install: it needs packages that aren't available");
    assert!(window.shows("Needs libnope"));
    assert!(
        window
            .texts()
            .iter()
            .all(|(role, text)| *role != Role::Button || text != "Install")
    );
}

#[test]
fn a_package_for_another_architecture_is_refused_before_packagekit_is_asked() {
    let mut window = Window::open(Script::default(), "1.2-1", "amd64");
    window.see("“tool” is built for Intel and AMD computers (amd64). This computer needs a package for 64-bit ARM (arm64).");
    assert!(window.fake.calls().is_empty());
}

#[test]
fn removing_lists_what_else_goes_and_a_system_package_says_so() {
    let installed = vec![FakePackage::new(1, "tool;1.2-1;arm64;installed", "", 0)];
    let mut window = Window::open(
        Script {
            installed: installed.clone(),
            changes: vec![
                FakePackage::new(13, "tool;1.2-1;arm64;installed", "", 0),
                FakePackage::new(13, "tool-plugins;1.2-1;arm64;installed", "", 0),
            ],
            ..Script::default()
        },
        "1.2-1",
        "arm64",
    );
    window.see("Version 1.2-1 is installed.");
    window.click("Remove…");
    window.see("This also removes 1 package: tool-plugins");
    assert_eq!(window.fake.called("RemovePackages").len(), 1);
    window.click("Remove");
    window.see("“tool” was removed.");
    let removals = window.fake.called("RemovePackages");
    assert_eq!(
        removals.iter().map(|call| call.flags).collect::<Vec<_>>(),
        [SIMULATE, 0]
    );
    assert_eq!(removals[1].args, ["tool;1.2-1;arm64;installed".to_owned()]);

    let mut system = Window::open(
        Script {
            installed,
            ..Script::default()
        },
        "1.2-1",
        "arm64",
    );
    system.see("Version 1.2-1 is installed.");
    system.click("Remove…");
    system.see("Remove “tool”?");
    system.fake.script(|script| {
        script
            .failures
            .insert("RemovePackages".to_owned(), (20, "essential".to_owned()));
    });
    system.click("Remove");
    system.see("This package is part of the system and can't be removed.");
}

#[test]
fn the_shipped_defaults_open_a_deb_in_the_installer_though_it_hides_from_menus() {
    let data = tempfile::tempdir().expect("data");
    let applications = data.path().join("applications");
    std::fs::create_dir_all(&applications).expect("applications");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/org.bande.Installer.desktop"
        ),
        applications.join("org.bande.Installer.desktop"),
    )
    .expect("desktop file");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../packaging/bande-desktop/bande-mimeapps.list"
        ),
        applications.join("bande-mimeapps.list"),
    )
    .expect("mimeapps");
    let desktops = ["Bande".to_owned()];
    let found = bande_xdg::handlers(&[data.path().to_path_buf()], None, &desktops);
    let paths = bande_xdg::mimeapps_paths(&[], &[data.path().to_path_buf()], &desktops);
    let apps = bande_xdg::MimeApps::load(&paths);
    let deb = ["application/vnd.debian.binary-package".to_owned()];
    assert_eq!(
        apps.default_for(&deb, &found)
            .map(|entry| entry.id.as_str()),
        Some("org.bande.Installer.desktop")
    );
}

#[test]
fn the_preview_and_a_refusal_match_their_goldens() {
    let shots: [(&str, Scheme, &str, &str); 3] = [
        (
            "ready-light",
            Scheme::Light,
            "arm64",
            "Also installs 2 packages",
        ),
        (
            "ready-dark",
            Scheme::Dark,
            "arm64",
            "Also installs 2 packages",
        ),
        (
            "wrong-architecture-light",
            Scheme::Light,
            "amd64",
            "is built for",
        ),
    ];
    for (name, scheme, arch, wait_for) in shots {
        let mut window = Window::open_in(
            Script {
                changes: changes(),
                ..Script::default()
            },
            "1.2-1",
            arch,
            scheme,
        );
        window.see(wait_for);
        let content = window
            .installer
            .ui()
            .scene(WINDOW_SIZE.0, WINDOW_SIZE.1 - WINDOW_HEADER_HEIGHT);
        let scene = framed_scene(
            &content,
            "Package Installer",
            None,
            None,
            &Palette::for_scheme(scheme),
        );
        let size = (scene.width, scene.height);
        let mut rgba = vec![0; size.0 as usize * size.1 as usize * 4];
        CpuRenderer::new()
            .render_into(&scene, 1.0, &mut rgba)
            .expect("frame");
        bande_golden::assert_golden_rgba!(name, size, rgba);
    }
}
