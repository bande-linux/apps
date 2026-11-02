mod canvas;
mod copy;
mod files;
mod info;
mod loader;
mod pdf;
mod sidebar;
mod view;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use bande_content::pdf::protocol::WORKER_FLAG;
pub use bande_content::pdf::worker::run as run_pdf_worker;
pub use bande_content::{Decoded, Failure, Kind, Pixels, decode_file};
pub use canvas::{Canvas, Orientation, Zoom};
pub use files::{argument_path, is_image_name};
pub use info::{InfoPanel, Row, document_rows, file_rows, info_ui, picture_rows};
pub use view::{PreviewView, Request, SidebarMode, file_name};

const ENGLISH: &str = include_str!("../locales/en-US/bande-preview.ftl");
pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub fn tr_with(id: &str, args: &[(&str, String)]) -> String {
    L10N.tr_args(
        id,
        args.iter()
            .map(|(name, value)| (*name, value.as_str().into())),
    )
}
