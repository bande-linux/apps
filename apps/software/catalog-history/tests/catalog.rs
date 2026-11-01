use std::path::{Path, PathBuf};

use bande_catalog::{Block, Bundle, Icon, flatpak_xml, load_xml, merge, system_xml};
use bande_xdg::Locale;

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn flathub(locale: &str) -> Vec<bande_catalog::Component> {
    load_xml(
        &flatpak_xml(&data().join("flatpak"), "aarch64"),
        Locale::parse(locale).as_ref(),
    )
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
    let root = data().join("swcatalog");
    let mut files = system_xml(&root);
    files.extend(flatpak_xml(&data().join("flatpak"), "aarch64"));
    let apps = merge(load_xml(&files, None));
    let calculator = apps.iter().find(|app| app.id == "org.gnome.Calculator");
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
    assert_eq!(apps.len(), 2);
}
