mod banner;
mod keys;
mod menu;
mod panes;
mod process;
mod render;
mod restore;
mod session;
mod settings;
mod settings_view;
mod tabs;
mod terminal;
mod theme;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use banner::{BANNER_HEIGHT, Banner, BannerAction};
pub use keys::{
    Command, MouseButton, MouseKind, bracketed, encode_key, mouse_report, next_font_size,
    reports_mouse, shortcut, wants_scroll_as_arrows,
};
pub use menu::{MenuCommand, TerminalContext, app_menus, tab_menu, terminal_menu};
pub use panes::{Axis, DIVIDER, Divider, Layout, Side};
pub use process::{Report, Reports};
pub use render::{DEFAULT_FONT_SIZE, Metrics, PADDING, Painter, TabBar};
pub use restore::{SavedWindow, claim, left_behind, window_file, windows_folder};
pub use session::{Notice, Session, display_directory};
pub use settings::{CursorStyle, FONT_SIZES, Settings, is_settings_file, settings_folder};
pub use settings_view::{SETTINGS_SIZE, SharedSettings, settings_view};
pub use tabs::{
    FULL_SCREEN_LEADING, HEADER_LEADING, STRIP_HEIGHT, TAB_HEIGHT, TabBarLayout, TabTarget,
};
pub use terminal::{
    CellPoint, GridSize, Highlights, Link, SearchDirection, SelectionKind, Terminal, escape_regex,
    is_openable,
};
pub use theme::{Profile, Theme, ansi_colors};

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
            include_str!("banner.rs"),
            include_str!("menu.rs"),
            include_str!("render.rs"),
            include_str!("session.rs"),
            include_str!("settings.rs"),
            include_str!("theme.rs"),
            include_str!("settings_view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
