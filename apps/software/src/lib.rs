pub mod catalog;
mod view;
mod worker;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use bande_l10n::Localizer;

use crate::catalog::Component;

pub use view::{Software, WINDOW_SIZE};

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
