#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use bande_catalog::{
    Block, Bundle, Category, Component, Icon, flatpak_files, flatpak_needs_x11, icon_path, load,
    mark_x11, read_text, search, system_files, x11_only_packages,
};
use bande_xdg::Locale;

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn flathub(locale: &str) -> Vec<Component> {
    load(
        &flatpak_files(&data().join("flatpak"), "aarch64"),
        Locale::parse(locale).as_ref(),
    )
}

fn packages() -> String {
    read_text(&data().join("lists/deb.debian.org_debian_dists_trixie_main_binary-arm64_Packages"))
        .expect("Packages index")
}

fn everything(locale: &str) -> Vec<Component> {
    let mut files = system_files(&data().join("share"));
    files.extend(system_files(&data().join("lib")));
    files.extend(flatpak_files(&data().join("flatpak"), "aarch64"));
    let mut apps = load(&files, Locale::parse(locale).as_ref());
    mark_x11(&mut apps, &x11_only_packages(&packages()));
    apps
}

fn find<'a>(apps: &'a [Component], key: &str) -> Option<&'a Component> {
    apps.iter().find(|app| app.key() == key)
}

#[test]
fn only_applications_are_kept() {
    let ids: Vec<_> = flathub("C").into_iter().map(|app| app.id).collect();
    assert_eq!(ids, ["org.gnome.Calculator", "org.gimp.GIMP"]);
}

#[test]
fn names_fall_back_from_country_to_language_to_untranslated() {
    let names = |locale| {
        flathub(locale)
            .into_iter()
            .map(|app| app.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names("sv_SE.UTF-8"),
        ["Räknare", "GNU Image Manipulation Program"]
    );
    assert_eq!(
        names("sv_FI.UTF-8"),
        ["Miniräknare", "GNU Image Manipulation Program"]
    );
    assert_eq!(
        names("fr_FR.UTF-8"),
        ["Calculator", "GNU Image Manipulation Program"]
    );
}

#[test]
fn description_markup_becomes_localised_blocks() {
    let calculator = flathub("sv_SE.UTF-8").remove(0);
    assert_eq!(
        calculator.description,
        [
            Block::Paragraph("Miniräknare är ett program som löser matematiska ekvationer.".into()),
            Block::Paragraph("Miniräknare har flera lägen:".into()),
            Block::List(vec!["Enkel".into(), "Avancerad".into()]),
        ]
    );
    assert_eq!(calculator.keywords, ["beräkning"]);
    assert_eq!(
        calculator.screenshots[0].caption.as_deref(),
        Some("Enkelt läge")
    );
}

#[test]
fn flatpak_fields_carry_remote_icons_and_largest_thumbnail() {
    let calculator = flathub("C").remove(0);
    let active = data().join("flatpak/flathub/aarch64/active/icons");
    assert_eq!(calculator.developer.as_deref(), Some("The GNOME Project"));
    assert_eq!(
        calculator.bundles,
        [Bundle::Flatpak {
            remote: "flathub".into(),
            reference: "app/org.gnome.Calculator/aarch64/stable".into(),
        }]
    );
    assert_eq!(
        calculator.icons[1],
        Icon::Cached {
            path: active.join("128x128/org.gnome.Calculator.png"),
            size: 128,
        }
    );
    assert_eq!(
        calculator.screenshots[0].thumbnail.as_deref(),
        Some("https://dl.flathub.org/media/org/gnome/Calculator/basic-624.png")
    );
    assert_eq!(calculator.releases[0].version, "48.1");
    assert_eq!(flathub("C")[1].developer.as_deref(), Some("The GIMP team"));
}

#[test]
fn an_app_in_both_sources_merges_with_debian_first() {
    let apps = everything("C");
    let calculator = find(&apps, "org.gnome.Calculator");
    assert_eq!(
        calculator.map(|app| app.bundles.clone()),
        Some(vec![
            Bundle::Deb("gnome-calculator".into()),
            Bundle::Flatpak {
                remote: "flathub".into(),
                reference: "app/org.gnome.Calculator/aarch64/stable".into(),
            },
        ])
    );
    let keys: Vec<_> = apps.iter().map(Component::key).collect();
    assert_eq!(
        keys,
        [
            "org.gnome.Calculator",
            "debian-xterm",
            "org.gnome.TextEditor",
            "org.gimp.GIMP"
        ]
    );
}

#[test]
fn dep11_documents_parse_with_locale_media_urls_and_cached_icons() {
    let apps = everything("sv_SE.UTF-8");
    let editor = find(&apps, "org.gnome.TextEditor").expect("Text Editor");
    assert_eq!(editor.name, "Textredigerare");
    assert_eq!(editor.summary, "Redigera textfiler");
    assert_eq!(editor.bundles, [Bundle::Deb("gnome-text-editor".into())]);
    assert_eq!(editor.developer.as_deref(), Some("GNOME-projektet"));
    assert!(matches!(editor.description.first(), Some(Block::Paragraph(text)) if !text.is_empty()));
    assert!(editor.categories.contains(&"Utility".to_owned()));
    assert!(editor.screenshots.iter().all(|shot| {
        shot.image
            .starts_with("https://appstream.debian.org/media/trixie/")
    }));
    assert!(matches!(
        editor.icons.first(),
        Some(Icon::Cached { path, .. }) if path.starts_with(data().join("lib/icons/debian-trixie-main"))
    ));
}

#[test]
fn cached_icons_are_preferred_and_missing_ones_fall_back_to_stock() {
    let apps = everything("C");
    let calculator = find(&apps, "org.gnome.Calculator")
        .map(|app| app.icons.clone())
        .unwrap_or_default();
    assert_eq!(
        icon_path(&calculator, 64, &[]),
        Some(
            data().join(
                "lib/icons/debian-trixie-main/64x64/gnome-calculator_org.gnome.Calculator.png"
            )
        )
    );
    let themes = tempfile::tempdir().expect("temporary directory");
    let stock = themes
        .path()
        .join("icons/hicolor/scalable/apps/mini.xterm.svg");
    std::fs::create_dir_all(stock.parent().expect("parent")).expect("icon directory");
    std::fs::write(&stock, "<svg/>").expect("icon");
    std::fs::write(
        themes.path().join("icons/hicolor/index.theme"),
        "[Icon Theme]\nName=Hicolor\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=128\nMinSize=8\nMaxSize=512\nType=Scalable\n",
    )
    .expect("index.theme");
    let xterm = find(&apps, "debian-xterm")
        .map(|app| app.icons.clone())
        .unwrap_or_default();
    assert_eq!(
        icon_path(&xterm, 64, &[themes.path().to_path_buf()]),
        Some(stock)
    );
}

#[test]
fn x11_only_debs_are_marked() {
    let apps = everything("C");
    let x11: Vec<_> = apps
        .iter()
        .filter(|app| app.needs_x11)
        .map(Component::key)
        .collect();
    assert_eq!(x11, ["debian-xterm"]);
    let index = read_text(
        &data().join("lists/deb.debian.org_debian_dists_trixie_main_binary-arm64_Packages"),
    )
    .unwrap_or_default();
    let mut packages: Vec<_> = x11_only_packages(&index).into_iter().collect();
    packages.sort();
    assert_eq!(packages, ["xpdf", "xterm"]);
}

#[test]
fn flatpak_sockets_decide_x11() {
    let metadata = |sockets: &str| {
        format!(
            "[Application]\nname=org.example.App\n\n[Context]\nshared=network;ipc;\nsockets={sockets}\n"
        )
    };
    assert!(flatpak_needs_x11(&metadata("x11;pulseaudio;")));
    assert!(!flatpak_needs_x11(&metadata(
        "wayland;fallback-x11;pulseaudio;"
    )));
    assert!(!flatpak_needs_x11(&metadata("fallback-x11;")));
    assert!(!flatpak_needs_x11("[Application]\nname=org.example.App\n"));
}

#[test]
fn search_ranks_names_before_keywords_before_summaries() {
    let apps = everything("C");
    let keys = |text| {
        search(&apps, text)
            .into_iter()
            .filter_map(|index| apps.get(index).map(Component::key))
            .collect::<Vec<_>>()
    };
    assert_eq!(keys("calc"), ["org.gnome.Calculator"]);
    assert_eq!(keys("edit"), ["org.gnome.TextEditor", "org.gimp.GIMP"]);
    assert_eq!(keys("terminal"), ["debian-xterm"]);
    assert_eq!(keys("arithmetic"), ["org.gnome.Calculator"]);
    assert_eq!(keys("photographs"), ["org.gimp.GIMP"]);
    assert_eq!(keys("").len(), apps.len());
}

#[test]
fn freedesktop_categories_map_to_store_categories() {
    let categories = |names: &[&str]| {
        Category::of(
            &names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        categories(&["GNOME", "GTK", "Utility"]),
        [Category::Utilities]
    );
    assert_eq!(
        categories(&["AudioVideo", "Audio", "Player", "Network"]),
        [Category::MusicAndVideo, Category::Internet]
    );
    assert!(categories(&["GTK"]).is_empty());
}
