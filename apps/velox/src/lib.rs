mod containers;
mod engine;
mod images;
mod machines;
mod networks;
mod overview;
mod readiness;
mod resources;
mod ui;
mod volumes;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use containers::{LogsTarget, LogsView, Work};
pub use engine::{Action, Informer, Update};
pub use machines::{
    Act, Effect, Hub, INSPECTOR_COMMAND as MACHINE_INSPECTOR, MACHINE_WINDOW_SIZE, MachineWindow,
    MachinesPane, Notice, Request as MachineRequest,
};
pub use readiness::Readiness;
pub use resources::file_menu;
pub use ui::{Request, SIDEBAR_WIDTH, Section, VeloxView};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-velox")));
