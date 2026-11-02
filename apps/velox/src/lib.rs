mod engine;
mod readiness;
mod ui;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use engine::{Action, Informer, Update};
pub use readiness::Readiness;
pub use ui::{Request, SIDEBAR_WIDTH, VeloxView};

const ENGLISH: &str = include_str!("../locales/en-US/bande-velox.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
