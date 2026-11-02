use alacritty_terminal::tty::Shell;
use alacritty_terminal::vte::ansi::CursorShape;
use bande_app_settings::{Stored, flag, pairs};

use crate::SCROLLBACK;
use crate::render::DEFAULT_FONT_SIZE;
use crate::theme::Profile;

pub const FONT_SIZES: (f32, f32) = (9.0, 32.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

impl CursorStyle {
    pub const ALL: [Self; 3] = [Self::Block, Self::Bar, Self::Underline];

    fn name(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Bar => "bar",
            Self::Underline => "underline",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.name() == name)
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Block => "cursor-block",
            Self::Bar => "cursor-bar",
            Self::Underline => "cursor-underline",
        }
    }

    pub fn shape(self) -> CursorShape {
        match self {
            Self::Block => CursorShape::Block,
            Self::Bar => CursorShape::Beam,
            Self::Underline => CursorShape::Underline,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseAsk {
    Always,
    #[default]
    WhenRunning,
    Never,
}

impl CloseAsk {
    pub const ALL: [Self; 3] = [Self::Always, Self::WhenRunning, Self::Never];

    fn name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::WhenRunning => "when-running",
            Self::Never => "never",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Always => "close-ask-always",
            Self::WhenRunning => "close-ask-running",
            Self::Never => "close-ask-never",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewTabFolder {
    #[default]
    Same,
    Home,
}

impl NewTabFolder {
    pub const ALL: [Self; 2] = [Self::Same, Self::Home];

    fn name(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Home => "home",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Same => "new-tab-same",
            Self::Home => "new-tab-home",
        }
    }
}

pub const SCROLLBACK_CHOICES: [usize; 4] = [1_000, 10_000, 100_000, 1_000_000];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub profile: Profile,
    pub font_size: f32,
    pub cursor: CursorStyle,
    pub blink: bool,
    pub scrollback: usize,
    pub alt_as_meta: bool,
    pub paste_warning: bool,
    pub close_ask: CloseAsk,
    pub new_tab_folder: NewTabFolder,
    pub shell: String,
    pub audible_bell: bool,
    pub flash_on_bell: bool,
    pub badge_on_bell: bool,
    pub notify_finished: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            profile: Profile::default(),
            font_size: DEFAULT_FONT_SIZE,
            cursor: CursorStyle::default(),
            blink: false,
            scrollback: SCROLLBACK,
            alt_as_meta: true,
            paste_warning: true,
            close_ask: CloseAsk::default(),
            new_tab_folder: NewTabFolder::default(),
            shell: String::new(),
            audible_bell: false,
            flash_on_bell: true,
            badge_on_bell: true,
            notify_finished: true,
        }
    }
}

impl Settings {
    pub fn shell(&self) -> Option<Shell> {
        let command = self.shell.trim();
        (!command.is_empty()).then(|| {
            Shell::new(
                "/bin/sh".to_owned(),
                vec!["-c".to_owned(), format!("exec {command}")],
            )
        })
    }
}

fn named<T: Copy>(all: &[T], name: fn(T) -> &'static str, value: &str) -> Option<T> {
    all.iter().copied().find(|choice| name(*choice) == value)
}

impl Stored for Settings {
    const FILE: &'static str = "terminal.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            match key {
                "profile" => {
                    settings.profile = Profile::parse(value).unwrap_or(settings.profile);
                }
                "font-size" => {
                    settings.font_size = value
                        .parse::<f32>()
                        .ok()
                        .filter(|size| (FONT_SIZES.0..=FONT_SIZES.1).contains(size))
                        .unwrap_or(settings.font_size);
                }
                "cursor" => settings.cursor = CursorStyle::parse(value).unwrap_or(settings.cursor),
                "blink" => settings.blink = flag(value).unwrap_or(settings.blink),
                "scrollback" => {
                    settings.scrollback = value
                        .parse()
                        .ok()
                        .filter(|lines| SCROLLBACK_CHOICES.contains(lines))
                        .unwrap_or(settings.scrollback);
                }
                "alt-as-meta" => settings.alt_as_meta = flag(value).unwrap_or(settings.alt_as_meta),
                "paste-warning" => {
                    settings.paste_warning = flag(value).unwrap_or(settings.paste_warning);
                }
                "close-ask" => {
                    settings.close_ask =
                        named(&CloseAsk::ALL, CloseAsk::name, value).unwrap_or(settings.close_ask);
                }
                "new-tab-folder" => {
                    settings.new_tab_folder = named(&NewTabFolder::ALL, NewTabFolder::name, value)
                        .unwrap_or(settings.new_tab_folder);
                }
                "shell" => settings.shell = value.to_owned(),
                "audible-bell" => {
                    settings.audible_bell = flag(value).unwrap_or(settings.audible_bell);
                }
                "flash-on-bell" => {
                    settings.flash_on_bell = flag(value).unwrap_or(settings.flash_on_bell);
                }
                "badge-on-bell" => {
                    settings.badge_on_bell = flag(value).unwrap_or(settings.badge_on_bell);
                }
                "notify-finished" => {
                    settings.notify_finished = flag(value).unwrap_or(settings.notify_finished);
                }
                _ => {}
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        [
            format!("profile = {}", self.profile.name()),
            format!("font-size = {}", self.font_size),
            format!("cursor = {}", self.cursor.name()),
            format!("blink = {}", self.blink),
            format!("scrollback = {}", self.scrollback),
            format!("alt-as-meta = {}", self.alt_as_meta),
            format!("paste-warning = {}", self.paste_warning),
            format!("close-ask = {}", self.close_ask.name()),
            format!("new-tab-folder = {}", self.new_tab_folder.name()),
            format!("shell = {}", self.shell.replace(['\n', '\r'], " ")),
            format!("audible-bell = {}", self.audible_bell),
            format!("flash-on-bell = {}", self.flash_on_bell),
            format!("badge-on-bell = {}", self.badge_on_bell),
            format!("notify-finished = {}", self.notify_finished),
        ]
        .map(|line| line + "\n")
        .concat()
    }
}
