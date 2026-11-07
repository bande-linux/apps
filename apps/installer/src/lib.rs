mod deb;
mod flow;
mod icon;
mod view;
mod worker;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use deb::{Deb, Identity, Machine, identity, missing_requirements, read};
pub use flow::{Action, Preview, Problem, State};
pub use view::{Installer, WINDOW_SIZE};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-installer")));
