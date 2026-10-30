use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use unbound_design::{Accent, Appearance, Color, Contrast, Palette, Scheme};

const BRIGHTER: f32 = 0.2;
const BRIGHTER_ON_LIGHT: f32 = 0.12;
const MATCH_OPACITY: f32 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Datum,
    Paper,
    Ink,
    Midnight,
    Contrast,
}

impl Profile {
    pub const ALL: [Self; 5] = [
        Self::Datum,
        Self::Paper,
        Self::Ink,
        Self::Midnight,
        Self::Contrast,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Datum => "datum",
            Self::Paper => "paper",
            Self::Ink => "ink",
            Self::Midnight => "midnight",
            Self::Contrast => "contrast",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|profile| profile.name() == name)
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Datum => "profile-datum",
            Self::Paper => "profile-paper",
            Self::Ink => "profile-ink",
            Self::Midnight => "profile-midnight",
            Self::Contrast => "profile-contrast",
        }
    }

    fn appearance(self, system: Appearance) -> Appearance {
        let scheme = match self {
            Self::Datum | Self::Contrast => system.scheme,
            Self::Paper => Scheme::Light,
            Self::Ink | Self::Midnight => Scheme::Dark,
        };
        let contrast = match self {
            Self::Contrast => Contrast::Increased,
            _ => system.contrast,
        };
        Appearance {
            scheme,
            contrast,
            ..system
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    pub on_cursor: Color,
    pub selection: Color,
    pub inactive_selection: Color,
    pub match_highlight: Color,
    pub current_match: Color,
    pub on_current_match: Color,
    pub link: Color,
    pub palette: Palette,
    pub chrome: Palette,
    ansi: [Color; 16],
}

pub fn ansi_colors(palette: &Palette) -> [Color; 16] {
    let scheme = palette.scheme;
    let normal = [
        palette.danger,
        Accent::Moss.color(scheme),
        Accent::Amber.color(scheme),
        Accent::Ultramarine.color(scheme),
        Accent::Plum.color(scheme),
        Accent::Teal.color(scheme),
    ];
    let lift = match scheme {
        Scheme::Light => BRIGHTER_ON_LIGHT,
        Scheme::Dark => BRIGHTER,
    };
    let (black, white, bright_black, bright_white) = match scheme {
        Scheme::Light => (palette.ink, palette.edge, palette.ink3, palette.hairline),
        Scheme::Dark => (palette.edge, palette.ink2, palette.ink3, palette.ink),
    };
    let mut colors = [black; 16];
    for (index, color) in normal.into_iter().enumerate() {
        colors[index + 1] = color;
        colors[index + 9] = color.mix(Color::WHITE, lift);
    }
    colors[7] = white;
    colors[8] = bright_black;
    colors[15] = bright_white;
    colors
}

impl Theme {
    pub fn new(system: Appearance, profile: Profile) -> Self {
        let palette = profile.appearance(system).palette();
        let chrome = system.palette();
        let (foreground, background) = match profile {
            Profile::Midnight => (palette.ink2, palette.chassis),
            Profile::Contrast => match palette.scheme {
                Scheme::Light => (palette.ink, palette.overlay),
                Scheme::Dark => (palette.ink, palette.chassis),
            },
            Profile::Datum | Profile::Paper | Profile::Ink => (palette.ink, palette.sheet),
        };
        Self {
            foreground,
            background,
            cursor: palette.accent,
            on_cursor: palette.on_accent,
            selection: palette.accent.mix(background, soft(palette.scheme)),
            inactive_selection: palette.well,
            match_highlight: palette.warning.with_opacity(MATCH_OPACITY),
            current_match: palette.warning,
            on_current_match: match palette.scheme {
                Scheme::Light => palette.ink,
                Scheme::Dark => palette.chassis,
            },
            link: palette.accent_text,
            palette,
            chrome,
            ansi: ansi_colors(&palette),
        }
    }

    fn indexed(&self, index: u8) -> Color {
        match index {
            0..16 => self.ansi[usize::from(index)],
            16..232 => {
                let cube = index - 16;
                let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
                Color {
                    r: level(cube / 36),
                    g: level(cube / 6 % 6),
                    b: level(cube % 6),
                    a: u8::MAX,
                }
            }
            _ => {
                let gray = 8 + (index - 232) * 10;
                Color {
                    r: gray,
                    g: gray,
                    b: gray,
                    a: u8::MAX,
                }
            }
        }
    }

    pub fn resolve(&self, color: AnsiColor, colors: &Colors, bold: bool) -> Color {
        match color {
            AnsiColor::Spec(rgb) => from_rgb(rgb),
            AnsiColor::Indexed(index) => {
                let index = if bold && index < 8 { index + 8 } else { index };
                colors[usize::from(index)].map_or_else(|| self.indexed(index), from_rgb)
            }
            AnsiColor::Named(named) => {
                if let Some(rgb) = colors[named] {
                    return from_rgb(rgb);
                }
                let number = named as usize;
                match named {
                    _ if number < 8 && bold => self.indexed(number as u8 + 8),
                    _ if number < 16 => self.indexed(number as u8),
                    NamedColor::Background => self.background,
                    NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
                    NamedColor::Cursor => self.cursor,
                    NamedColor::DimForeground => dimmed(self.foreground),
                    _ => {
                        let dim = number.saturating_sub(NamedColor::DimBlack as usize);
                        dimmed(self.indexed((dim % 8) as u8))
                    }
                }
            }
        }
    }
}

fn soft(scheme: Scheme) -> f32 {
    match scheme {
        Scheme::Light => 0.75,
        Scheme::Dark => 0.65,
    }
}

fn from_rgb(rgb: Rgb) -> Color {
    Color {
        r: rgb.r,
        g: rgb.g,
        b: rgb.b,
        a: u8::MAX,
    }
}

pub fn dimmed(color: Color) -> Color {
    let scale = |channel: u8| (u16::from(channel) * 2 / 3) as u8;
    Color {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
        a: color.a,
    }
}
