use bande_l10n::Text;
mod canvas;
mod copy;
mod files;
mod info;
mod loader;
mod pdf;
mod sidebar;
mod view;

pub use bande_content::pdf::protocol::WORKER_FLAG;
pub use bande_content::pdf::worker::run as run_pdf_worker;
pub use bande_content::{Decoded, Failure, Kind, Pixels, decode_file};
pub use canvas::{Canvas, Orientation, Zoom};
pub use files::{argument_path, is_image_name};
pub use info::{InfoPanel, Row, document_rows, file_rows, info_ui, picture_rows};
pub use view::{PreviewView, Request, SidebarMode, file_name};

pub fn tr_with(id: Text, args: &[(&str, String)]) -> String {
    id.tr_args(&[])
}
