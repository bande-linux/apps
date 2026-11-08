mod banner;
mod menu;
mod panes;
mod process;
mod session;
mod settings;
mod settings_view;
mod tabs;

pub use bande_terminal_view::{
    CellPoint, Command, DEFAULT_FONT_SIZE, GridSize, Highlights, Link, MouseButton, MouseKind,
    Painter, Pane, Profile, SCROLLBACK, SearchDirection, SelectionKind, Terminal, Theme, bracketed,
    dropped_text, encode_key, is_openable, mouse_report, next_font_size, reports_mouse, shortcut,
    wants_scroll_as_arrows,
};
pub use banner::{Banner, BannerAction};
pub use menu::{
    CLOSE_WINDOW, MenuCommand, TerminalContext, app_menus, look_up_text, tab_menu, terminal_menu,
};
pub use panes::{Axis, Divider, Layout, Side};
pub use process::{Report, Reports};
pub use session::{Notice, Session, ShellCommand};
pub use settings::{
    COLUMN_RANGE, CloseAsk, CursorStyle, FONT_SIZES, NewTabFolder, ROW_RANGE, SCROLLBACK_CHOICES,
    Settings,
};
pub use settings_view::{SETTINGS_SIZE, settings_view};
pub use tabs::{FULL_SCREEN_LEADING, STRIP_HEIGHT};

pub const COLUMNS: usize = 80;
pub const ROWS: usize = 24;
