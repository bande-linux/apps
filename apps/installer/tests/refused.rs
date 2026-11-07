#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_installer::{Installer, Machine, State};
use bande_packages::{Client, PATH, SERVICE};
use bande_toolkit::{Appearance, TextSystem};
use zbus::blocking::Connection;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::OwnedObjectPath;

const SIMULATE: u64 = 1 << 2;
const NOT_AUTHORIZED: u32 = 48;
const EXIT_SUCCESS: u32 = 1;
const EXIT_FAILED: u32 = 2;

struct PackageKit;

#[zbus::interface(name = "org.freedesktop.PackageKit")]
impl PackageKit {
    async fn create_transaction(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> OwnedObjectPath {
        let path = OwnedObjectPath::try_from("/transaction").unwrap();
        let _ = server.remove::<Transaction, _>(&path).await;
        server.at(&path, Transaction).await.unwrap();
        path
    }
}

struct Transaction;

#[zbus::interface(name = "org.freedesktop.PackageKit.Transaction")]
impl Transaction {
    fn set_hints(&self, _hints: Vec<String>) {}

    async fn resolve(
        &self,
        _filter: u64,
        _names: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        Self::finished(&emitter, EXIT_SUCCESS, 0).await.unwrap();
    }

    async fn install_files(
        &self,
        flags: u64,
        _files: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        if flags & SIMULATE != 0 {
            Self::finished(&emitter, EXIT_SUCCESS, 0).await.unwrap();
            return;
        }
        Self::error_code(&emitter, NOT_AUTHORIZED, "Failed to obtain authentication.")
            .await
            .unwrap();
        Self::finished(&emitter, EXIT_FAILED, 0).await.unwrap();
    }

    #[zbus(signal)]
    async fn finished(emitter: &SignalEmitter<'_>, exit: u32, runtime: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn error_code(emitter: &SignalEmitter<'_>, code: u32, details: &str) -> zbus::Result<()>;
}

fn ready(installer: &mut Installer) {
    let start = Instant::now();
    while !matches!(installer.state(), State::Ready(_)) {
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(10));
        installer.pump();
    }
    installer.ui().scene(520, 480);
}

#[test]
fn a_refused_password_says_so_instead_of_returning_silently_to_install() {
    let daemon = zbus::blocking::connection::Builder::session()
        .unwrap()
        .serve_at(PATH, PackageKit)
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tool/DEBIAN")).unwrap();
    std::fs::write(
        dir.path().join("tool/DEBIAN/control"),
        "Package: tool\nVersion: 1\nArchitecture: all\nMaintainer: A <a@b.c>\nDescription: T\n",
    )
    .unwrap();
    let deb = dir.path().join("tool.deb");
    let mut build = Command::new("dpkg-deb");
    build.args(["--build", "--root-owner-group"]);
    assert!(
        build
            .arg(dir.path().join("tool"))
            .arg(&deb)
            .output()
            .unwrap()
            .status
            .success()
    );

    let client = Client::new(&Connection::session().unwrap());
    let mut installer = Installer::new(
        client,
        deb,
        Machine::current().unwrap(),
        Appearance::default().palette(),
        TextSystem::new(),
    )
    .unwrap();
    ready(&mut installer);
    let tree = installer.accessibility();
    let (button, _) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Install"))
        .unwrap();
    installer.ui().perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: *button,
        data: None,
    });
    installer.pump();
    assert!(matches!(installer.state(), State::Installing(_)));

    ready(&mut installer);
    let tree = installer.accessibility();
    assert!(tree.nodes.iter().any(|(_, node)| {
        [node.label(), node.value()]
            .iter()
            .flatten()
            .any(|text| text.starts_with("Nothing was changed"))
    }));
    drop(daemon);
}
