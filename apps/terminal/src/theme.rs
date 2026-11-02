use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use bande_design::{Accent, Appearance, Color, Contrast, Palette, Scheme};

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
    SolarizedLight,
    SolarizedDark,
    Dracula,
    Nord,
    Gruvbox,
    TokyoNight,
    Catppuccin,
}

struct Classic {
    dark: bool,
    background: u32,
    foreground: u32,
    cursor: u32,
    selection: u32,
    ansi: [u32; 16],
}

const SOLARIZED_ANSI: [u32; 16] = [
    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x002b36,
    0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
];

const SOLARIZED_LIGHT: Classic = Classic {
    dark: false,
    background: 0xfdf6e3,
    foreground: 0x657b83,
    cursor: 0x586e75,
    selection: 0xeee8d5,
    ansi: SOLARIZED_ANSI,
};

const SOLARIZED_DARK: Classic = Classic {
    dark: true,
    background: 0x002b36,
    foreground: 0x839496,
    cursor: 0x93a1a1,
    selection: 0x073642,
    ansi: SOLARIZED_ANSI,
};

const DRACULA: Classic = Classic {
    dark: true,
    background: 0x282a36,
    foreground: 0xf8f8f2,
    cursor: 0xf8f8f2,
    selection: 0x44475a,
    ansi: [
        0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, 0x6272a4,
        0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
    ],
};

const NORD: Classic = Classic {
    dark: true,
    background: 0x2e3440,
    foreground: 0xd8dee9,
    cursor: 0xd8dee9,
    selection: 0x434c5e,
    ansi: [
        0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, 0x4c566a,
        0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
    ],
};

const GRUVBOX: Classic = Classic {
    dark: true,
    background: 0x282828,
    foreground: 0xebdbb2,
    cursor: 0xebdbb2,
    selection: 0x504945,
    ansi: [
        0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, 0x928374,
        0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
    ],
};

const TOKYO_NIGHT: Classic = Classic {
    dark: true,
    background: 0x1a1b26,
    foreground: 0xc0caf5,
    cursor: 0xc0caf5,
    selection: 0x283457,
    ansi: [
        0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868,
        0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
    ],
};

const CATPPUCCIN: Classic = Classic {
    dark: true,
    background: 0x1e1e2e,
    foreground: 0xcdd6f4,
    cursor: 0xf5e0dc,
    selection: 0x45475a,
    ansi: [
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70,
        0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ],
};

impl Profile {
    pub const ALL: [Self; 12] = [
        Self::Datum,
        Self::Paper,
        Self::Ink,
        Self::Midnight,
        Self::Contrast,
        Self::SolarizedLight,
        Self::SolarizedDark,
        Self::Dracula,
        Self::Nord,
        Self::Gruvbox,
        Self::TokyoNight,
        Self::Catppuccin,
    ];

    fn classic(self) -> Option<&'static Classic> {
        match self {
            Self::SolarizedLight => Some(&SOLARIZED_LIGHT),
            Self::SolarizedDark => Some(&SOLARIZED_DARK),
            Self::Dracula => Some(&DRACULA),
            Self::Nord => Some(&NORD),
            Self::Gruvbox => Some(&GRUVBOX),
            Self::TokyoNight => Some(&TOKYO_NIGHT),
            Self::Catppuccin => Some(&CATPPUCCIN),
            Self::Datum | Self::Paper | Self::Ink | Self::Midnight | Self::Contrast => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Datum => "datum",
            Self::Paper => "paper",
            Self::Ink => "ink",
            Self::Midnight => "midnight",
            Self::Contrast => "contrast",
            Self::SolarizedLight => "solarized-light",
            Self::SolarizedDark => "solarized-dark",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::Gruvbox => "gruvbox",
            Self::TokyoNight => "tokyo-night",
            Self::Catppuccin => "catppuccin",
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
            Self::SolarizedLight => "profile-solarized-light",
            Self::SolarizedDark => "profile-solarized-dark",
            Self::Dracula => "profile-dracula",
            Self::Nord => "profile-nord",
            Self::Gruvbox => "profile-gruvbox",
            Self::TokyoNight => "profile-tokyo-night",
            Self::Catppuccin => "profile-catppuccin",
        }
    }

    fn appearance(self, system: Appearance) -> Appearance {
        let scheme = match (self, self.classic()) {
            (_, Some(classic)) if classic.dark => Scheme::Dark,
            (_, Some(_)) => Scheme::Light,
            (Self::Paper, None) => Scheme::Light,
            (Self::Ink | Self::Midnight, None) => Scheme::Dark,
            _ => system.scheme,
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

fn ansi_colors(palette: &Palette) -> [Color; 16] {
    let scheme = palette.scheme;
    let normal = [
        palette.danger_text,
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
        if let Some(classic) = profile.classic() {
            let ansi = classic.ansi.map(Color::hex);
            let background = Color::hex(classic.background);
            return Self {
                foreground: Color::hex(classic.foreground),
                background,
                cursor: Color::hex(classic.cursor),
                on_cursor: background,
                selection: Color::hex(classic.selection),
                inactive_selection: Color::hex(classic.selection).mix(background, 0.5),
                match_highlight: ansi[3].with_opacity(MATCH_OPACITY),
                current_match: ansi[11],
                on_current_match: background,
                link: ansi[12],
                palette,
                chrome,
                ansi,
            };
        }
        let (foreground, background) = match profile {
            Profile::Midnight => (palette.ink2, palette.deep),
            Profile::Contrast => match palette.scheme {
                Scheme::Light => (palette.ink, palette.overlay),
                Scheme::Dark => (palette.ink, palette.chassis),
            },
            _ => (palette.ink, palette.sheet),
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
            on_current_match: palette.deep,
            link: palette.accent_text,
            palette,
            chrome,
            ansi: ansi_colors(&palette),
        }
    }

    pub fn ansi(&self) -> &[Color; 16] {
        &self.ansi
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
