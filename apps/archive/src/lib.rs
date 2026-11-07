mod panel;
mod settings;
mod work;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use panel::{Intent, PANEL_SIZE, PROMPT_SIZE, Panel, Row};
pub use settings::{After, ArchiveSettings, SETTINGS_SIZE, settings_form};
pub use work::{Ended, Kind, Work};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-archive")));
