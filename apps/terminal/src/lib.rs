mod click;
mod find;
mod keys;
mod process;
mod render;
mod session;
mod tabs;
mod terminal;
mod theme;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use click::ClickCounter;
pub use find::{FIND_BAR_HEIGHT, FindAction, FindBar};
pub use keys::{
    Command, MouseButton, MouseKind, bracketed, encode_key, mouse_report, next_font_size,
    reports_mouse, shortcut, wants_scroll_as_arrows,
};
pub use process::DirectoryReports;
pub use render::{DEFAULT_FONT_SIZE, Metrics, PADDING, Painter, TabBar};
pub use session::{Notice, Session, display_directory};
pub use tabs::{TAB_BAR_HEIGHT, TabBarLayout, TabTarget};
pub use terminal::{
    CellPoint, GridSize, Highlights, SearchDirection, SelectionKind, Terminal, escape_regex,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-term.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const COLUMNS: usize = 80;
pub const ROWS: usize = 24;
pub const SCROLLBACK: usize = 10_000;

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("app.rs"),
            include_str!("find.rs"),
            include_str!("render.rs"),
            include_str!("session.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
