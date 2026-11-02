mod engine;
mod machines;
mod readiness;
mod ui;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use engine::{Action, Informer, Update};
pub use machines::{
    Act, Effect, Hub, MACHINE_WINDOW_SIZE, MachineWindow, MachinesPane, Notice,
    Request as MachineRequest,
};
pub use readiness::Readiness;
pub use ui::{Pane, Request, SIDEBAR_WIDTH, VeloxView};

const ENGLISH: &str = include_str!("../locales/en-US/bande-velox.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
