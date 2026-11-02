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

const ENGLISH: &str = include_str!("../locales/en-US/bande-installer.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
