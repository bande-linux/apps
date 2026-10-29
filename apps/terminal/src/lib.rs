mod click;
mod keys;
mod render;
mod terminal;
mod theme;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use click::ClickCounter;
pub use keys::{Command, bracketed, encode_key, next_font_size, shortcut, wants_scroll_as_arrows};
pub use render::{DEFAULT_FONT_SIZE, Metrics, PADDING, Painter};
pub use terminal::{CellPoint, GridSize, SelectionKind, Terminal};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-term.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const COLUMNS: usize = 80;
pub const ROWS: usize = 24;
pub const SCROLLBACK: usize = 10_000;

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("lib.rs"), include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
