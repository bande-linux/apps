use alacritty_terminal::term::TermMode;
use unbound_toolkit::ui::{Key, Modifiers};
use unbound_xdg::path_from_file_uri;

fn modifier_parameter(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.control)
}

fn cursor_key(letter: char, modifiers: Modifiers, mode: TermMode) -> Vec<u8> {
    let parameter = modifier_parameter(modifiers);
    if parameter > 1 {
        format!("\x1b[1;{parameter}{letter}").into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn tilde_key(number: u8, modifiers: Modifiers) -> Vec<u8> {
    let parameter = modifier_parameter(modifiers);
    if parameter > 1 {
        format!("\x1b[{number};{parameter}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

fn control_byte(character: char) -> Option<u8> {
    match character.to_ascii_lowercase() {
        letter @ 'a'..='z' => Some(letter as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn with_meta(alt: bool, bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        let mut prefixed = vec![0x1b];
        prefixed.extend(bytes);
        prefixed
    } else {
        bytes
    }
}

pub fn encode_key(key: Key, modifiers: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    if modifiers.logo {
        return None;
    }
    let plain = Modifiers {
        alt: false,
        ..modifiers
    };
    let bytes = match key {
        Key::Left if modifiers.alt && plain == Modifiers::default() => b"\x1bb".to_vec(),
        Key::Right if modifiers.alt && plain == Modifiers::default() => b"\x1bf".to_vec(),
        Key::Up => cursor_key('A', modifiers, mode),
        Key::Down => cursor_key('B', modifiers, mode),
        Key::Right => cursor_key('C', modifiers, mode),
        Key::Left => cursor_key('D', modifiers, mode),
        Key::Home => cursor_key('H', modifiers, mode),
        Key::End => cursor_key('F', modifiers, mode),
        Key::Insert => tilde_key(2, modifiers),
        Key::Delete => tilde_key(3, modifiers),
        Key::PageUp => tilde_key(5, modifiers),
        Key::PageDown => tilde_key(6, modifiers),
        Key::Function(number @ 1..=4) => {
            let letter = (b'P' + number - 1) as char;
            let parameter = modifier_parameter(modifiers);
            if parameter > 1 {
                format!("\x1b[1;{parameter}{letter}").into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        Key::Function(number @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(number - 5)];
            tilde_key(code, modifiers)
        }
        Key::Function(_) | Key::Menu => return None,
        Key::Enter => with_meta(modifiers.alt, b"\r".to_vec()),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => with_meta(modifiers.alt, b"\t".to_vec()),
        Key::Escape => with_meta(modifiers.alt, b"\x1b".to_vec()),
        Key::Backspace if modifiers.control => with_meta(modifiers.alt, vec![0x08]),
        Key::Backspace => with_meta(modifiers.alt, vec![0x7f]),
        Key::Space if modifiers.control => with_meta(modifiers.alt, vec![0]),
        Key::Space => with_meta(modifiers.alt, b" ".to_vec()),
        Key::Character(character) => {
            let bytes = match control_byte(character).filter(|_| modifiers.control) {
                Some(byte) => vec![byte],
                None => character.to_string().into_bytes(),
            };
            with_meta(modifiers.alt, bytes)
        }
    };
    Some(bytes)
}

pub fn bracketed(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let cleaned = text.replace('\x1b', "");
        format!("\x1b[200~{cleaned}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

pub fn dropped_text(uris: &[String], text: &str) -> String {
    if uris.is_empty() {
        return text.to_owned();
    }
    uris.iter()
        .map(|uri| {
            let word = path_from_file_uri(uri)
                .map_or_else(|| uri.clone(), |path| path.to_string_lossy().into_owned());
            format!("{} ", shell_quoted(&word))
        })
        .collect()
}

fn shell_quoted(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_alphanumeric() || "/._-+,:@".contains(character));
    match plain {
        true => word.to_owned(),
        false => format!("'{}'", word.replace('\'', r"'\''")),
    }
}

pub fn wants_scroll_as_arrows(mode: TermMode) -> bool {
    mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

impl MouseButton {
    fn code(self) -> u32 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::WheelUp => 64,
            Self::WheelDown => 65,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press(MouseButton),
    Release(MouseButton),
    Drag(MouseButton),
    Move,
}

const MOTION: u32 = 32;
const X10_RELEASE: u32 = 3;
const X10_OFFSET: u32 = 32;

pub fn reports_mouse(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

pub fn mouse_report(
    kind: MouseKind,
    column: usize,
    line: usize,
    modifiers: Modifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let wanted = match kind {
        MouseKind::Press(_) | MouseKind::Release(_) => reports_mouse(mode),
        MouseKind::Drag(_) => mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
        MouseKind::Move => mode.contains(TermMode::MOUSE_MOTION),
    };
    if !wanted {
        return None;
    }
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let code = match kind {
        MouseKind::Press(button) => button.code(),
        MouseKind::Release(MouseButton::WheelUp | MouseButton::WheelDown) => return None,
        MouseKind::Release(button) if sgr => button.code(),
        MouseKind::Release(_) => X10_RELEASE,
        MouseKind::Drag(button) => button.code() + MOTION,
        MouseKind::Move => X10_RELEASE + MOTION,
    };
    let code = code
        + 4 * u32::from(modifiers.shift)
        + 8 * u32::from(modifiers.alt)
        + 16 * u32::from(modifiers.control);
    let (x, y) = (
        u32::try_from(column + 1).unwrap_or(u32::MAX),
        u32::try_from(line + 1).unwrap_or(u32::MAX),
    );
    if sgr {
        let end = if matches!(kind, MouseKind::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    let mut bytes = b"\x1b[M".to_vec();
    if mode.contains(TermMode::UTF8_MOUSE) {
        for value in [code, x, y] {
            let character = char::from_u32((value + X10_OFFSET).min(2047))?;
            bytes.extend(character.to_string().into_bytes());
        }
    } else {
        for value in [code, x, y] {
            bytes.push(u8::try_from((value + X10_OFFSET).min(255)).unwrap_or(u8::MAX));
        }
    }
    Some(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Copy,
    Paste,
    SelectAll,
    ClearScrollback,
    BiggerText,
    SmallerText,
    DefaultTextSize,
    NewTab,
    NewWindow,
    NextTab,
    PreviousTab,
    SelectTab(usize),
    LastTab,
    Find,
    FindNext,
    FindPrevious,
    SplitSideBySide,
    SplitStacked,
    NextPane,
    PreviousPane,
    PreviousPrompt,
    NextPrompt,
    Settings,
}

pub fn shortcut(key: Key, modifiers: Modifiers) -> Option<Command> {
    if !modifiers.logo || modifiers.control || modifiers.alt {
        return None;
    }
    let character = match key {
        Key::Character(character) => character,
        Key::Up if !modifiers.shift => return Some(Command::PreviousPrompt),
        Key::Down if !modifiers.shift => return Some(Command::NextPrompt),
        _ => return None,
    };
    let command = match (character.to_ascii_lowercase(), modifiers.shift) {
        ('c', false) => Command::Copy,
        ('v', false) => Command::Paste,
        ('a', false) => Command::SelectAll,
        ('k', false) => Command::ClearScrollback,
        ('=' | '+', _) => Command::BiggerText,
        ('-', false) => Command::SmallerText,
        ('0', false) => Command::DefaultTextSize,
        ('f', false) => Command::Find,
        ('g', false) => Command::FindNext,
        ('g', true) => Command::FindPrevious,
        ('t', false) => Command::NewTab,
        ('d', false) => Command::SplitSideBySide,
        ('d', true) => Command::SplitStacked,
        (']', false) => Command::NextPane,
        ('[', false) => Command::PreviousPane,
        (',', false) => Command::Settings,
        ('n', false) => Command::NewWindow,
        ('9', false) => Command::LastTab,
        (digit @ '1'..='8', false) => Command::SelectTab(usize::from(digit as u8 - b'1')),
        ('}' | ']', true) => Command::NextTab,
        ('{' | '[', true) => Command::PreviousTab,
        _ => return None,
    };
    Some(command)
}

const FONT_SIZES: [f32; 14] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 40.0,
];

pub fn next_font_size(size: f32, bigger: bool) -> f32 {
    let found = if bigger {
        FONT_SIZES.iter().find(|candidate| **candidate > size)
    } else {
        FONT_SIZES.iter().rev().find(|candidate| **candidate < size)
    };
    found.copied().unwrap_or(size)
}
