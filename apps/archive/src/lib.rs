mod panel;
mod settings;
mod work;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use panel::{Intent, PANEL_SIZE, PROMPT_SIZE, Panel, Row};
pub use settings::{After, ArchiveSettings, SETTINGS_SIZE, settings_form};
pub use work::{Ended, Kind, Work};

const ENGLISH: &str = include_str!("../locales/en-US/bande-archive.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
