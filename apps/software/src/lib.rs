mod view;
mod worker;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use bande_catalog::Component;
use bande_l10n::Localizer;

pub use view::{Page, Software, WINDOW_SIZE};

const ENGLISH: &str = include_str!("../locales/en-US/bande-software.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub fn installed_keys(apps: &[Component], desktop_ids: &HashSet<String>) -> HashSet<String> {
    apps.iter()
        .filter(|app| {
            app.launchable
                .clone()
                .unwrap_or_else(|| format!("{}.desktop", app.key()))
                .split_whitespace()
                .any(|id| desktop_ids.contains(id))
        })
        .map(|app| app.key().to_owned())
        .collect()
}

pub fn desktop_ids(data_dirs: &[PathBuf]) -> HashSet<String> {
    bande_xdg::applications(data_dirs, None, &bande_xdg::current_desktops())
        .iter()
        .filter_map(|entry| entry.path.file_name()?.to_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_software_uses_is_translated() {
        let sources = [include_str!("view.rs")];
        assert_eq!(
            bande_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
