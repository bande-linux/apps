use std::ops::Range;

use bande_toolkit::{Color, Palette, Scheme};

const ESCAPE: char = '\u{1b}';
const BELL: char = '\u{7}';
const TAB_STOP: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pen {
    pub tone: Option<Tone>,
    pub bold: bool,
    pub faint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Styled {
    pub text: String,
    pub spans: Vec<(Range<usize>, Pen)>,
}

const LIGHT: [u32; 16] = [
    0x201E1A, 0xC0302C, 0x2E8B3A, 0x9A6B00, 0x2B5FC7, 0x9B3CB4, 0x10848C, 0x6E6A63, 0x5A5650,
    0xE0453E, 0x3AA84A, 0xB88600, 0x457CE8, 0xB855D0, 0x1AA0A8, 0x302D29,
];

const DARK: [u32; 16] = [
    0x5A5650, 0xF06A60, 0x7CCF7A, 0xE8C25C, 0x74A6F5, 0xD08CE8, 0x5CCFD6, 0xD8D4CC, 0x8A8680,
    0xFF8A80, 0x9CE898, 0xF8D87C, 0x9CC2FF, 0xE4ACF8, 0x80E4EA, 0xF4F2EE,
];

fn cube(level: u8) -> u8 {
    match level {
        0 => 0,
        level => 55 + level * 40,
    }
}

impl Tone {
    pub fn color(self, palette: &Palette) -> Color {
        let (r, g, b) = match self {
            Self::Rgb(r, g, b) => (r, g, b),
            Self::Indexed(index @ 0..16) => {
                return Color::hex(match palette.scheme {
                    Scheme::Light => LIGHT[usize::from(index)],
                    Scheme::Dark => DARK[usize::from(index)],
                });
            }
            Self::Indexed(index @ 16..232) => {
                let index = index - 16;
                (cube(index / 36), cube(index / 6 % 6), cube(index % 6))
            }
            Self::Indexed(index) => {
                let grey = 8 + (index - 232) * 10;
                (grey, grey, grey)
            }
        };
        Color {
            r,
            g,
            b,
            a: u8::MAX,
        }
    }
}

fn channel(value: Option<u16>) -> u8 {
    u8::try_from(value.unwrap_or_default()).unwrap_or(u8::MAX)
}

fn extended(codes: &mut impl Iterator<Item = Option<u16>>) -> Option<Tone> {
    match codes.next().flatten() {
        Some(5) => Some(Tone::Indexed(channel(codes.next().flatten()))),
        Some(2) => Some(Tone::Rgb(
            channel(codes.next().flatten()),
            channel(codes.next().flatten()),
            channel(codes.next().flatten()),
        )),
        _ => None,
    }
}

fn select(pen: &mut Pen, parameters: &str) {
    let mut codes = parameters.split([';', ':']).map(|code| match code {
        "" => Some(0),
        code => code.parse::<u16>().ok(),
    });
    while let Some(code) = codes.next() {
        match code {
            Some(0) => *pen = Pen::default(),
            Some(1) => pen.bold = true,
            Some(2) => pen.faint = true,
            Some(22) => {
                pen.bold = false;
                pen.faint = false;
            }
            Some(code @ 30..=37) => pen.tone = u8::try_from(code - 30).ok().map(Tone::Indexed),
            Some(code @ 90..=97) => pen.tone = u8::try_from(code - 82).ok().map(Tone::Indexed),
            Some(38) => pen.tone = extended(&mut codes),
            Some(48 | 58) => {
                extended(&mut codes);
            }
            Some(39) => pen.tone = None,
            _ => {}
        }
    }
}

pub fn parse(line: &str, pen: &mut Pen) -> Styled {
    let mut styled = Styled::default();
    let mut start = 0;
    let mut column = 0;
    let mut current = *pen;
    let mut chars = line.chars().peekable();
    let mut close = |styled: &mut Styled, pen: Pen| {
        let end = styled.text.len();
        if end > start {
            styled.spans.push((start..end, pen));
        }
        start = end;
    };
    while let Some(char) = chars.next() {
        match char {
            ESCAPE => match chars.next() {
                Some('[') => {
                    let mut parameters = String::new();
                    let mut last = None;
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            last = Some(next);
                            break;
                        }
                        parameters.push(next);
                    }
                    if last == Some('m') {
                        close(&mut styled, current);
                        select(&mut current, &parameters);
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == BELL || (next == ESCAPE && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => {
                let spaces = TAB_STOP - column % TAB_STOP;
                styled.text.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            char if char.is_control() => {}
            char => {
                styled.text.push(char);
                column += 1;
            }
        }
    }
    close(&mut styled, current);
    *pen = current;
    styled
}
