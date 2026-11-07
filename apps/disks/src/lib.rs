mod jobs;
mod model;
mod size;
mod view;
mod widgets;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use jobs::{Abilities, Erase, Format, Job, Report, Worker};
pub use model::{Model, Scheme};
pub use view::{
    ADD_PARTITION, Choose, DELETE_PARTITION, DisksView, EJECT, ERASE, FIRST_AID, IMAGE_FROM, MOUNT,
    OPEN_IMAGE, RENAME, RESIZE, RESTORE, Request,
};
pub use widgets::SIDEBAR_WIDTH;

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-disks")));
