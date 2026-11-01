#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
use bande_catalog::{Block, Bundle, Category, Component, Release};
use bande_packages::Client;
use bande_software::{Page, Software, WINDOW_SIZE, installed_keys};
use bande_test_bus::SessionBus;
use bande_test_bus::packagekit::{FakePackage, FakePackageKit, Script};
use bande_toolkit::{CpuRenderer, Palette, Scheme, TextSystem, WINDOW_HEADER_HEIGHT, framed_scene};

fn app(id: &str, name: &str, summary: &str, categories: &[&str]) -> Component {
    Component {
        id: id.to_owned(),
        name: name.to_owned(),
        summary: summary.to_owned(),
        categories: categories.iter().map(|name| (*name).to_owned()).collect(),
        bundles: vec![Bundle::Deb(name.to_lowercase())],
        ..Component::default()
    }
}

fn catalogue() -> Vec<Component> {
    let mut calculator = app(
        "org.gnome.Calculator.desktop",
        "Calculator",
        "Perform arithmetic, scientific or financial calculations",
        &["GNOME", "Utility"],
    );
    calculator.developer = Some("The GNOME Project".to_owned());
    calculator.license = Some("GPL-3.0-or-later".to_owned());
    calculator.description = vec![
        Block::Paragraph("Calculator solves mathematical equations.".to_owned()),
        Block::List(vec!["Basic".to_owned(), "Advanced".to_owned()]),
    ];
    calculator.launchable = Some("org.gnome.Calculator.desktop".to_owned());
    calculator.releases = vec![Release {
        version: "48.1".to_owned(),
        timestamp: None,
    }];
    calculator.bundles.push(Bundle::Flatpak {
        remote: "flathub".to_owned(),
        reference: "app/org.gnome.Calculator/aarch64/stable".to_owned(),
    });
    let mut xterm = app(
        "debian-xterm.desktop",
        "XTerm",
        "standard terminal emulator",
        &["System"],
    );
    xterm.needs_x11 = true;
    vec![
        calculator,
        app(
            "org.gnome.TextEditor",
            "Text Editor",
            "Edit text files",
            &["Utility", "TextEditor"],
        ),
        app(
            "org.gimp.GIMP",
            "GIMP",
            "Create images and edit photographs",
            &["Graphics"],
        ),
        app(
            "org.gnome.Characters",
            "Characters",
            "Character map",
            &["Utility"],
        ),
        xterm,
    ]
}

struct Opened {
    software: Software,
    fake: FakePackageKit,
    _bus: SessionBus,
}

fn open_with(scheme: Scheme, script: Script) -> Opened {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakePackageKit::start(&bus, script).expect("fake PackageKit");
    let system = bus.connect().expect("connection");
    let installed = HashSet::from(["org.gnome.TextEditor".to_owned()]);
    let software = Software::new(
        catalogue(),
        installed,
        Client::new(&system),
        "arm64",
        Palette::for_scheme(scheme),
        TextSystem::new(),
    )
    .expect("software");
    Opened {
        software,
        fake,
        _bus: bus,
    }
}

fn open(scheme: Scheme) -> Opened {
    open_with(scheme, Script::default())
}

fn nodes(software: &mut Software) -> Vec<(accesskit::NodeId, accesskit::Node)> {
    software.ui().scene(WINDOW_SIZE.0, WINDOW_SIZE.1);
    software.accessibility().nodes
}

fn buttons(software: &mut Software) -> Vec<String> {
    nodes(software)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::Button)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

fn texts(software: &mut Software) -> Vec<String> {
    nodes(software)
        .into_iter()
        .filter_map(|(_, node)| node.label().or_else(|| node.value()).map(str::to_owned))
        .collect()
}

fn perform(
    software: &mut Software,
    target: accesskit::NodeId,
    action: Action,
    data: Option<ActionData>,
) {
    software.ui().perform(&ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data,
    });
    software.pump();
}

fn click(software: &mut Software, label: &str) {
    let target = nodes(software)
        .into_iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some(label))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {label} button in {:?}", buttons(software)));
    perform(software, target, Action::Click, None);
}

fn type_search(software: &mut Software, text: &str) {
    let field = nodes(software)
        .into_iter()
        .find(|(_, node)| matches!(node.role(), Role::SearchInput | Role::TextInput))
        .map(|(id, _)| id)
        .expect("search field");
    perform(
        software,
        field,
        Action::SetValue,
        Some(ActionData::Value(text.into())),
    );
}

const APPS: [&str; 4] = ["Calculator", "Characters", "GIMP", "Text Editor"];

fn listed(software: &mut Software) -> Vec<String> {
    buttons(software)
        .into_iter()
        .filter(|label| APPS.contains(&label.as_str()))
        .collect()
}

#[test]
fn discover_lists_apps_by_name_and_hides_x11_ones() {
    let mut opened = open(Scheme::Light);
    let software = &mut opened.software;
    assert_eq!(listed(software), APPS);
    assert!(!texts(software).iter().any(|text| text.contains("XTerm")));
}

#[test]
fn typing_filters_the_list_in_rank_order() {
    let mut opened = open(Scheme::Light);
    let software = &mut opened.software;
    type_search(software, "edit");
    assert_eq!(listed(software), ["Text Editor", "GIMP"]);
    type_search(software, "nothing like this");
    assert!(
        texts(software)
            .iter()
            .any(|text| text.contains("No apps match"))
    );
}

#[test]
fn an_app_page_shows_its_details_and_back_returns_to_the_list() {
    let mut opened = open(Scheme::Light);
    let software = &mut opened.software;
    click(software, "Calculator");
    assert_eq!(software.page(), Page::App(0));
    let shown = texts(software);
    for expected in [
        "by The GNOME Project",
        "Calculator solves mathematical equations.",
        "• Basic\n• Advanced",
        "Version 48.1",
        "Licence: GPL-3.0-or-later",
        "From Debian\nFrom flathub",
    ] {
        assert!(
            shown.iter().any(|text| text == expected),
            "{expected} missing from {shown:?}"
        );
    }
    click(software, "Back");
    assert_eq!(software.page(), Page::Discover);
}

#[test]
fn the_installed_section_lists_installed_apps() {
    let mut opened = open(Scheme::Light);
    let software = &mut opened.software;
    software.show(Page::Installed);
    assert_eq!(listed(software), ["Text Editor"]);
}

#[test]
fn installed_apps_match_desktop_entries_by_launchable_or_id() {
    let apps = catalogue();
    let desktop_ids = HashSet::from([
        "org.gnome.Calculator.desktop".to_owned(),
        "org.gimp.GIMP.desktop".to_owned(),
    ]);
    let installed = installed_keys(&apps, &desktop_ids);
    let mut installed: Vec<_> = installed.into_iter().collect();
    installed.sort();
    assert_eq!(installed, ["org.gimp.GIMP", "org.gnome.Calculator"]);
}

#[test]
fn the_utilities_page_matches_its_golden() {
    let mut opened = open(Scheme::Light);
    let software = &mut opened.software;
    software.show(Page::Category(Category::Utilities));
    assert_eq!(
        listed(software),
        ["Calculator", "Characters", "Text Editor"]
    );
    let content = software
        .ui()
        .scene(WINDOW_SIZE.0, WINDOW_SIZE.1 - WINDOW_HEADER_HEIGHT);
    let scene = framed_scene(
        &content,
        "Software",
        None,
        None,
        &Palette::for_scheme(Scheme::Light),
    );
    let size = (scene.width, scene.height);
    let mut rgba = vec![0; size.0 as usize * size.1 as usize * 4];
    CpuRenderer::new()
        .render_into(&scene, 1.0, &mut rgba)
        .expect("frame");
    bande_golden::assert_golden_rgba!("utilities-light", size, rgba);
}

fn until(software: &mut Software, done: impl Fn(&mut Software) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        software.pump();
        if done(software) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never got there: {:?}",
            texts(software)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn calculator_packages() -> Vec<FakePackage> {
    vec![
        FakePackage::new(2, "gnome-calculator;48.1-1;amd64;debian", "", 0),
        FakePackage::new(2, "gnome-calculator;48.1-1;arm64;debian", "", 0),
    ]
}

#[test]
fn install_resolves_the_native_package_and_ends_with_open() {
    let mut opened = open_with(
        Scheme::Light,
        Script {
            available: calculator_packages(),
            ..Script::default()
        },
    );
    let software = &mut opened.software;
    click(software, "Calculator");
    click(software, "Install");
    until(software, |software| {
        buttons(software).contains(&"Open".to_owned())
    });
    let installs = opened.fake.called("InstallPackages");
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0].args, ["gnome-calculator;48.1-1;arm64;debian"]);
}

#[test]
fn a_failed_install_says_why_and_offers_install_again() {
    let mut opened = open_with(
        Scheme::Light,
        Script {
            available: calculator_packages(),
            failures: [("InstallPackages".to_owned(), (4, "no network".to_owned()))].into(),
            ..Script::default()
        },
    );
    let software = &mut opened.software;
    click(software, "Calculator");
    click(software, "Install");
    until(software, |software| {
        texts(software).iter().any(|text| {
            text.starts_with("Couldn’t install Calculator") && text.contains("no network")
        })
    });
    assert!(buttons(software).contains(&"Install".to_owned()));
}
