pub mod catalog;
mod media;
mod online;
pub mod settings;
mod view;
mod widgets;
mod worker;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use view::{
    COMMAND_BACK, COMMAND_DISCOVER, COMMAND_FIND, COMMAND_INSTALLED, COMMAND_UPDATES, Finished,
    Software, WINDOW_SIZE,
};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-software")));
