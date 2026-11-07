mod document;
mod encoding;
mod file;
mod find;
mod formats;
mod portal;
mod session;
mod view;
mod worker;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use document::{Format, Loaded, open, read};
pub use encoding::{CHOICES, LineEnding, TextEncoding};
pub use file::{MAX_FILE_BYTES, read_bounded};
pub use portal::{choose_files, choose_save_path};
pub use session::{Record, Session, new_id, now};
pub use view::{COLORS, DocumentView, FAMILIES, Opening, Request, Settings};
pub use worker::{Message, spawn};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-textedit")));

pub fn encoding_name(encoding: TextEncoding) -> String {
    L10N.tr(&format!(
        "encoding-{}",
        encoding.encoding.name().to_ascii_lowercase()
    ))
}
