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

use bande_l10n::Localizer;

pub use banner::{Banner, BannerAction};
pub use keys::{
    Command, MouseButton, MouseKind, bracketed, dropped_text, encode_key, mouse_report,
    next_font_size, reports_mouse, shortcut, wants_scroll_as_arrows,
};
pub use menu::{CLOSE_WINDOW, MenuCommand, TerminalContext, app_menus, tab_menu, terminal_menu};
pub use panes::{Axis, Divider, Layout, Side};
pub use process::{Report, Reports};
pub use render::{DEFAULT_FONT_SIZE, Painter, Pane, TabBar};
pub use restore::{SavedWindow, claim, left_behind, window_file, windows_folder};
pub use session::{Notice, Session};
pub use settings::{CursorStyle, FONT_SIZES, Settings};
pub use settings_view::{SETTINGS_SIZE, settings_view};
pub use tabs::{FULL_SCREEN_LEADING, HEADER_LEADING, STRIP_HEIGHT, TabBarLayout, TabTarget};
pub use terminal::{
    CellPoint, GridSize, Highlights, Link, SearchDirection, SelectionKind, Terminal, is_openable,
};
pub use theme::{Profile, Theme, ansi_colors};

const ENGLISH: &str = include_str!("../locales/en-US/bande-terminal.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const COLUMNS: usize = 80;
pub const ROWS: usize = 24;
pub const SCROLLBACK: usize = 10_000;
