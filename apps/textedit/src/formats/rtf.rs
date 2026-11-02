use std::fmt::Write;
use std::sync::Arc;

use bande_toolkit::Color;
use bande_toolkit::ui::{Alignment, ListMarker, ParagraphAttributes, RichText, TextAttributes};
use encoding_rs::{Encoding, WINDOWS_1252};

use super::{Builder, MAX_NESTING, paragraphs};

const SKIPPED: [&str; 14] = [
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "header",
    "footer",
    "headerl",
    "headerr",
    "footerl",
    "footerr",
    "listtable",
    "listoverridetable",
    "fldinst",
];
const DEFAULT_SIZE: f32 = 12.0;

#[derive(Clone, Default)]
struct State {
    attributes: TextAttributes,
    skip: bool,
    font_table: bool,
    color_table: bool,
    list_text: bool,
    unicode_skip: usize,
}

#[derive(Default)]
struct Reader {
    builder: Builder,
    fonts: Vec<(i32, String)>,
    font_name: Option<(i32, String)>,
    colors: Vec<Option<Color>>,
    color: [u32; 3],
    color_set: bool,
    codepage: Option<&'static Encoding>,
    pending_list: Option<ListMarker>,
    list_text: String,
    paragraph: ParagraphAttributes,
    pending_bytes: Vec<u8>,
}

impl Reader {
    fn flush_bytes(&mut self, state: &State) {
        if self.pending_bytes.is_empty() {
            return;
        }
        let encoding = self.codepage.unwrap_or(WINDOWS_1252);
        let (text, _) = encoding.decode_without_bom_handling(&self.pending_bytes);
        let text = text.into_owned();
        self.pending_bytes.clear();
        self.text(state, &text);
    }

    fn text(&mut self, state: &State, text: &str) {
        if state.skip {
            return;
        }
        if state.font_table {
            if let Some((_, name)) = &mut self.font_name {
                name.push_str(text);
            }
            return;
        }
        if state.list_text {
            self.list_text.push_str(text);
            return;
        }
        if self.builder.is_empty_paragraph() {
            *self.builder.paragraph() = self.paragraph;
        }
        self.builder.text(text, &state.attributes);
    }

    fn end_paragraph(&mut self) {
        let mut paragraph = self.paragraph;
        paragraph.list = self.pending_list.take().or(paragraph.list);
        if self.builder.is_empty_paragraph() {
            *self.builder.paragraph() = paragraph;
        }
        self.builder.end_paragraph(self.paragraph);
    }

    fn control(&mut self, state: &mut State, word: &str, parameter: Option<i32>) {
        let on = parameter != Some(0);
        let attributes = &mut state.attributes;
        match word {
            "par" | "line" | "sect" | "page" if !state.skip && !state.font_table => {
                self.flush_bytes(state);
                self.end_paragraph();
            }
            "pard" => {
                self.paragraph = ParagraphAttributes::default();
                self.pending_list = None;
            }
            "plain" => state.attributes = TextAttributes::default(),
            "b" => attributes.bold = on,
            "i" => attributes.italic = on,
            "ul" => attributes.underline = on,
            "ulnone" => attributes.underline = false,
            "strike" | "striked" => attributes.strikethrough = on,
            "fs" => {
                attributes.size = parameter
                    .map(|half| (half as f32 / 2.0).clamp(4.0, 400.0))
                    .filter(|size| (size - DEFAULT_SIZE).abs() > 0.1);
            }
            "f" if state.font_table => {
                self.font_name = parameter.map(|index| (index, String::new()));
            }
            "f" => {
                attributes.family = parameter.and_then(|index| {
                    self.fonts
                        .iter()
                        .find(|(known, _)| *known == index)
                        .and_then(|(_, name)| super::family(name))
                });
            }
            "cf" => {
                attributes.color = parameter
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| self.colors.get(index).copied().flatten());
            }
            "red" | "green" | "blue" if state.color_table => {
                let channel = match word {
                    "red" => 0,
                    "green" => 1,
                    _ => 2,
                };
                self.color[channel] = parameter.unwrap_or(0).clamp(0, 255) as u32;
                self.color_set = true;
            }
            "ql" => self.paragraph.alignment = Alignment::Start,
            "qc" => self.paragraph.alignment = Alignment::Center,
            "qr" => self.paragraph.alignment = Alignment::End,
            "qj" => self.paragraph.alignment = Alignment::Justify,
            "ilvl" => {
                self.paragraph.level = parameter.unwrap_or(0).clamp(0, 8) as u8;
            }
            "ls" => {
                self.paragraph.list.get_or_insert(ListMarker::Bullet);
            }
            "listtext" => {
                state.list_text = true;
                self.list_text.clear();
            }
            "ansicpg" => {
                self.codepage = parameter
                    .map(|page| match page {
                        10000 => "macintosh".to_owned(),
                        page => format!("windows-{page}"),
                    })
                    .and_then(|label| Encoding::for_label(label.as_bytes()));
            }
            "mac" => self.codepage = Encoding::for_label(b"macintosh"),
            "uc" => state.unicode_skip = parameter.unwrap_or(1).clamp(0, 8) as usize,
            "tab" => self.text(state, "\t"),
            "emdash" => self.text(state, "—"),
            "endash" => self.text(state, "–"),
            "bullet" => self.text(state, "•"),
            "lquote" => self.text(state, "‘"),
            "rquote" => self.text(state, "’"),
            "ldblquote" => self.text(state, "“"),
            "rdblquote" => self.text(state, "”"),
            word if SKIPPED.contains(&word) => match word {
                "fonttbl" => state.font_table = true,
                "colortbl" => state.color_table = true,
                _ => state.skip = true,
            },
            _ => {}
        }
    }

    fn end_group(&mut self, state: &State) {
        if state.list_text && !self.list_text.trim().is_empty() {
            let marker = self.list_text.trim();
            self.paragraph.list = Some(
                match marker.starts_with(|c: char| c.is_ascii_alphanumeric()) {
                    true => ListMarker::Number,
                    false => ListMarker::Bullet,
                },
            );
            self.pending_list = self.paragraph.list;
            self.list_text.clear();
        }
        if state.font_table
            && let Some((index, name)) = self.font_name.take()
        {
            let name = name.trim_end_matches(';').trim().to_owned();
            if !name.is_empty() {
                self.fonts.push((index, name));
            }
        }
    }
}

fn hex(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|digit| digit as u8)
}

pub fn read(bytes: &[u8]) -> RichText {
    let mut reader = Reader::default();
    let mut stack: Vec<State> = Vec::new();
    let mut state = State {
        unicode_skip: 1,
        ..State::default()
    };
    let mut at = 0;
    let mut skipping_unicode = 0;
    let mut overflow = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        at += 1;
        match byte {
            b'{' => {
                reader.flush_bytes(&state);
                match stack.len() < MAX_NESTING {
                    true => stack.push(state.clone()),
                    false => overflow += 1,
                }
                state.skip |= overflow > 0;
                state.list_text = false;
            }
            b'}' if overflow > 0 => overflow -= 1,
            b'}' => {
                reader.flush_bytes(&state);
                reader.end_group(&state);
                if let Some(previous) = stack.pop() {
                    state = previous;
                }
            }
            b'\\' => {
                let Some(&next) = bytes.get(at) else {
                    break;
                };
                if next.is_ascii_alphabetic() {
                    let start = at;
                    while bytes.get(at).is_some_and(u8::is_ascii_alphabetic) && at - start < 32 {
                        at += 1;
                    }
                    let word = std::str::from_utf8(&bytes[start..at]).unwrap_or_default();
                    let number_start = at;
                    if bytes.get(at) == Some(&b'-') {
                        at += 1;
                    }
                    while bytes.get(at).is_some_and(u8::is_ascii_digit) && at - number_start < 10 {
                        at += 1;
                    }
                    let parameter = std::str::from_utf8(&bytes[number_start..at])
                        .ok()
                        .and_then(|number| number.parse::<i32>().ok());
                    if bytes.get(at) == Some(&b' ') {
                        at += 1;
                    }
                    if word == "u" {
                        reader.flush_bytes(&state);
                        let unit = parameter.unwrap_or(0).rem_euclid(65536) as u32;
                        let character = char::from_u32(unit).unwrap_or('\u{FFFD}');
                        reader.text(&state, &character.to_string());
                        skipping_unicode = state.unicode_skip;
                        continue;
                    }
                    if word != "bin" {
                        reader.flush_bytes(&state);
                        reader.control(&mut state, word, parameter);
                    } else {
                        at =
                            at.saturating_add(usize::try_from(parameter.unwrap_or(0)).unwrap_or(0));
                    }
                    continue;
                }
                at += 1;
                match next {
                    b'\'' => {
                        let value = bytes
                            .get(at..at + 2)
                            .and_then(|pair| Some(hex(pair[0])? * 16 + hex(pair[1])?));
                        at += 2;
                        if skipping_unicode > 0 {
                            skipping_unicode -= 1;
                        } else if let Some(value) = value {
                            reader.pending_bytes.push(value);
                        }
                    }
                    b'*' => state.skip = true,
                    b'~' => reader.text(&state, "\u{a0}"),
                    b'_' => reader.text(&state, "\u{2011}"),
                    b'\n' | b'\r' => {
                        reader.flush_bytes(&state);
                        reader.end_paragraph();
                    }
                    other => {
                        reader.flush_bytes(&state);
                        reader.text(&state, &(other as char).to_string());
                    }
                }
            }
            b'\r' | b'\n' => {}
            b';' if state.color_table => {
                reader.colors.push(reader.color_set.then(|| {
                    let [red, green, blue] = reader.color;
                    Color::hex(red << 16 | green << 8 | blue)
                }));
                reader.color = [0; 3];
                reader.color_set = false;
            }
            _ if skipping_unicode > 0 => skipping_unicode -= 1,
            byte if byte.is_ascii() => {
                reader.flush_bytes(&state);
                let end = bytes[at..]
                    .iter()
                    .position(|byte| {
                        matches!(byte, b'{' | b'}' | b'\\' | b'\r' | b'\n' | b';')
                            || !byte.is_ascii()
                    })
                    .map_or(bytes.len(), |offset| at + offset);
                let text = std::str::from_utf8(&bytes[at - 1..end])
                    .unwrap_or_default()
                    .to_owned();
                at = end;
                reader.text(&state, &text);
            }
            byte => reader.pending_bytes.push(byte),
        }
    }
    reader.flush_bytes(&state);
    reader.builder.finish()
}

fn escaped(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(character);
            }
            '\t' => out.push_str("\\tab "),
            character if character.is_ascii() => out.push(character),
            character => {
                let mut units = [0u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{}?", *unit as i16);
                }
            }
        }
    }
}

pub fn write(rich: &RichText) -> String {
    let paragraphs = paragraphs(rich);
    let mut fonts: Vec<Arc<str>> = Vec::new();
    let mut colors: Vec<Color> = Vec::new();
    for paragraph in &paragraphs {
        for (_, attributes) in &paragraph.spans {
            if let Some(family) = &attributes.family
                && !fonts.contains(family)
            {
                fonts.push(Arc::clone(family));
            }
            if let Some(color) = attributes.color
                && !colors.contains(&color)
            {
                colors.push(color);
            }
        }
    }
    let mut out =
        String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1\n{\\fonttbl{\\f0\\fswiss Helvetica;}");
    for (index, family) in fonts.iter().enumerate() {
        out.push_str(&format!("{{\\f{}\\fnil ", index + 1));
        escaped(family, &mut out);
        out.push_str(";}");
    }
    out.push_str("}\n{\\colortbl;");
    for color in &colors {
        let _ = write!(out, "\\red{}\\green{}\\blue{};", color.r, color.g, color.b);
    }
    out.push_str("}\n");
    let mut numbers = [0usize; 9];
    for paragraph in &paragraphs {
        let attributes = paragraph.attributes;
        out.push_str("\\pard");
        out.push_str(match attributes.alignment {
            Alignment::Start => "\\ql",
            Alignment::Center => "\\qc",
            Alignment::End => "\\qr",
            Alignment::Justify => "\\qj",
        });
        let level = usize::from(attributes.level.min(8));
        if attributes.list != Some(ListMarker::Number) {
            numbers[level] = 0;
        }
        match attributes.list {
            Some(list) => {
                numbers[level] += 1;
                numbers[level + 1..].fill(0);
                let indent = 360 * (level + 1);
                let _ = write!(
                    out,
                    "\\li{indent}\\fi-360\\ls1\\ilvl{level} {{\\listtext\\tab "
                );
                match list {
                    ListMarker::Bullet => out.push_str("\\u8226?"),
                    ListMarker::Number => {
                        let _ = write!(out, "{}.", numbers[level]);
                    }
                }
                out.push_str("\\tab}");
            }
            None => {
                numbers = [0; 9];
                out.push(' ');
            }
        }
        for (text, attributes) in &paragraph.spans {
            out.push('{');
            let opened = out.len();
            if attributes.bold {
                out.push_str("\\b");
            }
            if attributes.italic {
                out.push_str("\\i");
            }
            if attributes.underline {
                out.push_str("\\ul");
            }
            if attributes.strikethrough {
                out.push_str("\\strike");
            }
            if let Some(family) = &attributes.family
                && let Some(index) = fonts.iter().position(|known| known == family)
            {
                let _ = write!(out, "\\f{}", index + 1);
            }
            if let Some(size) = attributes.size {
                let _ = write!(out, "\\fs{}", (size * 2.0).round() as i32);
            }
            if let Some(color) = attributes.color
                && let Some(index) = colors.iter().position(|known| *known == color)
            {
                let _ = write!(out, "\\cf{}", index + 1);
            }
            if out.len() > opened {
                out.push(' ');
            }
            escaped(text, &mut out);
            out.push('}');
        }
        out.push_str("\\par\n");
    }
    out.push('}');
    out
}
