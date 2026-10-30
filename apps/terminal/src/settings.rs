use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use alacritty_terminal::vte::ansi::CursorShape;

use crate::render::DEFAULT_FONT_SIZE;
use crate::theme::Profile;

pub const FONT_SIZES: (f32, f32) = (9.0, 32.0);
const FILE_NAME: &str = "terminal.conf";

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub profile: Profile,
    pub font_size: f32,
    pub cursor: CursorStyle,
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
            flash_on_bell: true,
            badge_on_bell: true,
            notify_finished: true,
        }
    }
}

fn flag(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
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

    pub fn to_text(&self) -> String {
        format!(
            "profile = {}\nfont-size = {}\ncursor = {}\nflash-on-bell = {}\nbadge-on-bell = {}\nnotify-finished = {}\n",
            self.profile.name(),
            self.font_size,
            self.cursor.name(),
            self.flash_on_bell,
            self.badge_on_bell,
            self.notify_finished,
        )
    }

    pub fn load(folder: &Path) -> Self {
        fs::read_to_string(folder.join(FILE_NAME))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(&self, folder: &Path) -> io::Result<()> {
        fs::create_dir_all(folder)?;
        let staging = folder.join(format!("{FILE_NAME}.new"));
        fs::write(&staging, self.to_text())?;
        fs::rename(staging, folder.join(FILE_NAME))
    }
}

pub fn settings_folder() -> Option<PathBuf> {
    unbound_xdg::config_home().map(|config| config.join("unbound"))
}

pub fn is_settings_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == FILE_NAME)
}
