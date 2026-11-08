use bande_l10n::tr;
mod document;
mod encoding;
mod file;
mod find;
mod formats;
mod portal;
mod session;
mod view;
mod worker;

pub use document::{Format, Loaded, open, read};
pub use encoding::{CHOICES, LineEnding, TextEncoding};
pub use file::{MAX_FILE_BYTES, read_bounded};
pub use portal::{choose_files, choose_save_path};
pub use session::{Record, Session, new_id, now};
pub use view::{COLORS, DocumentView, FAMILIES, Opening, Request, Settings, line_ending_title};
pub use worker::{Message, spawn};

pub fn encoding_name(encoding: TextEncoding) -> String {
    let name = encoding.encoding.name().to_ascii_lowercase();
    match name.as_str() {
        "utf-8" => tr!("Unicode (UTF-8)"),
        "utf-16le" => tr!("Unicode (UTF-16 Little Endian)"),
        "utf-16be" => tr!("Unicode (UTF-16 Big Endian)"),
        "windows-1252" => tr!("Western (Windows Latin 1)"),
        "iso-8859-15" => tr!("Western (ISO Latin 9)"),
        "macintosh" => tr!("Western (Mac OS Roman)"),
        "windows-1250" => tr!("Central European (Windows Latin 2)"),
        "iso-8859-2" => tr!("Central European (ISO Latin 2)"),
        "windows-1251" => tr!("Cyrillic (Windows)"),
        "iso-8859-5" => tr!("Cyrillic (ISO 8859-5)"),
        "koi8-r" => tr!("Cyrillic (KOI8-R)"),
        "ibm866" => tr!("Cyrillic (DOS)"),
        "windows-1253" => tr!("Greek (Windows)"),
        "iso-8859-7" => tr!("Greek (ISO 8859-7)"),
        "windows-1254" => tr!("Turkish (Windows Latin 5)"),
        "shift_jis" => tr!("Japanese (Shift JIS)"),
        "euc-jp" => tr!("Japanese (EUC)"),
        "gbk" => tr!("Chinese Simplified (GBK)"),
        "big5" => tr!("Chinese Traditional (Big 5)"),
        "euc-kr" => tr!("Korean (EUC)"),
        "windows-874" => tr!("Thai (Windows)"),
        _ => name,
    }
}
