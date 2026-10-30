use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use unbound_toolkit::{Appearance, Color, Palette, Scheme};

const LIGHT_ANSI: [u32; 16] = [
    0x000000, 0x990000, 0x00A600, 0x999900, 0x0000B2, 0xB200B2, 0x00A6B2, 0xBFBFBF, 0x666666,
    0xE50000, 0x00D900, 0xE5E500, 0x0000FF, 0xE500E5, 0x00E5E5, 0xE5E5E5,
];

const DARK_ANSI: [u32; 16] = [
    0x000000, 0xD74B4B, 0x4FB84F, 0xC7C43F, 0x4A7BF0, 0xC350C3, 0x3FB6C4, 0xBFBFBF, 0x737373,
    0xFF6B6B, 0x6BE36B, 0xF0EC5A, 0x7FA2FF, 0xF07CF0, 0x6BE8F0, 0xFFFFFF,
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub foreground: Color,
    pub background: Color,
    pub selection: Color,
    pub inactive_selection: Color,
    pub match_highlight: Color,
    pub current_match: Color,
    pub link: Color,
    pub palette: Palette,
    ansi: [u32; 16],
}

impl Theme {
    pub fn new(appearance: Appearance) -> Self {
        let palette = appearance.palette();
        match appearance.scheme {
            Scheme::Light => Self {
                foreground: Color::hex(0x000000),
                background: Color::hex(0xFFFFFF),
                selection: palette.accent.with_alpha(0x4C),
                inactive_selection: Color::hex(0x000000).with_alpha(0x1F),
                match_highlight: Color::hex(0xFFE066).with_alpha(0xB0),
                current_match: Color::hex(0xFF9F0A),
                link: palette.accent,
                palette,
                ansi: LIGHT_ANSI,
            },
            Scheme::Dark => Self {
                foreground: Color::hex(0xF2F2F2),
                background: Color::hex(0x1E1E1E),
                selection: palette.accent.with_alpha(0x66),
                inactive_selection: Color::hex(0xFFFFFF).with_alpha(0x2E),
                match_highlight: Color::hex(0xB38F00).with_alpha(0xA0),
                current_match: Color::hex(0xFF9F0A),
                link: palette.accent,
                palette,
                ansi: DARK_ANSI,
            },
        }
    }

    fn indexed(&self, index: u8) -> Color {
        match index {
            0..16 => Color::hex(self.ansi[usize::from(index)]),
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
                    NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => {
                        self.foreground
                    }
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
