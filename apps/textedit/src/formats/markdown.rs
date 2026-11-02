use bande_toolkit::ui::{ListMarker, ParagraphAttributes, RichText, TextAttributes};

use super::{Builder, Paragraph, code, heading, heading_level, is_code, link, paragraphs};

const SEARCH_LIMIT: usize = 4096;
const MAX_LEVEL: usize = 8;

struct Reader {
    builder: Builder,
    open: bool,
    gap: bool,
    listing: bool,
}

impl Reader {
    fn close(&mut self) {
        if self.open {
            self.builder.end_paragraph(ParagraphAttributes::default());
            self.open = false;
        }
    }

    fn start(&mut self, attributes: ParagraphAttributes, listed: bool) {
        self.close();
        if !self.builder.is_empty() && (self.gap || self.listing != listed) {
            self.builder.end_paragraph(ParagraphAttributes::default());
        }
        self.gap = false;
        self.listing = listed;
        *self.builder.paragraph() = attributes;
        self.open = true;
    }
}

fn list_item(line: &str) -> Option<(usize, ListMarker, &str)> {
    let content = line.trim_start_matches(' ');
    let indent = line.len() - content.len();
    let level = (indent / 2).min(MAX_LEVEL);
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = content.strip_prefix(bullet) {
            return Some((level, ListMarker::Bullet, rest));
        }
    }
    let digits = content.chars().take_while(char::is_ascii_digit).count();
    let rest = &content[digits..];
    match (
        digits,
        rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")),
    ) {
        (1..=9, Some(rest)) => Some((level, ListMarker::Number, rest)),
        _ => None,
    }
}

const DELIMITERS: [&str; 8] = ["`", "](", ")", "**", "__", "~~", "*", "_"];

struct Closings(Vec<Vec<usize>>);

impl Closings {
    fn new(text: &str) -> Self {
        Self(
            DELIMITERS
                .iter()
                .map(|delimiter| text.match_indices(delimiter).map(|(at, _)| at).collect())
                .collect(),
        )
    }

    fn after(&self, from: usize, delimiter: &str) -> Option<usize> {
        let index = DELIMITERS.iter().position(|known| *known == delimiter)?;
        let found = &self.0[index];
        found
            .get(found.partition_point(|at| *at < from))
            .copied()
            .filter(|at| *at - from <= SEARCH_LIMIT)
    }
}

fn inline(builder: &mut Builder, text: &str, base: &TextAttributes) {
    let closings = Closings::new(text);
    let mut attributes = base.clone();
    let mut plain = String::new();
    let mut at = 0;
    let flush = |builder: &mut Builder, plain: &mut String, attributes: &TextAttributes| {
        builder.text(plain, attributes);
        plain.clear();
    };
    while let Some(character) = text[at..].chars().next() {
        let rest = &text[at..];
        if character == '\\'
            && let Some(escaped) = rest[1..].chars().next()
            && escaped.is_ascii_punctuation()
        {
            plain.push(escaped);
            at += 1 + escaped.len_utf8();
            continue;
        }
        if character == '`'
            && let Some(end) = closings.after(at + 1, "`")
        {
            flush(builder, &mut plain, &attributes);
            let code = super::TextAttributes {
                family: super::code().family,
                ..attributes.clone()
            };
            builder.text(&text[at + 1..end], &code);
            at = end + 1;
            continue;
        }
        if character == '['
            && let Some(middle) = closings.after(at + 1, "](")
            && let Some(end) = closings.after(middle + 2, ")")
        {
            flush(builder, &mut plain, &attributes);
            let url = text[middle + 2..end].trim();
            let linked = TextAttributes {
                bold: attributes.bold,
                italic: attributes.italic,
                size: attributes.size,
                ..link(url)
            };
            inline(builder, &text[at + 1..middle], &linked);
            at = end + 1;
            continue;
        }
        let toggles = [("**", 0), ("__", 0), ("~~", 2), ("*", 1), ("_", 1)];
        let longest = toggles
            .iter()
            .find(|(delimiter, _)| rest.starts_with(delimiter));
        let toggle = longest.filter(|(delimiter, kind)| {
            let allowed = match kind {
                1 if *delimiter == "_" => {
                    let before = text[..at].chars().next_back();
                    attributes.italic || !before.is_some_and(char::is_alphanumeric)
                }
                _ => true,
            };
            let active = match kind {
                0 => attributes.bold,
                1 => attributes.italic,
                _ => attributes.strikethrough,
            };
            let after = rest[delimiter.len()..].chars().next();
            let before = text[..at].chars().next_back();
            let flanking = match active {
                true => before.is_some_and(|character| !character.is_whitespace()),
                false => after.is_some_and(|character| !character.is_whitespace()),
            };
            allowed
                && flanking
                && (active || closings.after(at + delimiter.len(), delimiter).is_some())
        });
        if let Some((delimiter, kind)) = toggle {
            flush(builder, &mut plain, &attributes);
            match kind {
                0 => attributes.bold = !attributes.bold,
                1 => attributes.italic = !attributes.italic,
                _ => attributes.strikethrough = !attributes.strikethrough,
            }
            at += delimiter.len();
            continue;
        }
        let literal = longest.map_or(character.len_utf8(), |(delimiter, _)| delimiter.len());
        plain.push_str(&rest[..literal]);
        at += literal;
    }
    flush(builder, &mut plain, &attributes);
}

pub fn read(text: &str) -> RichText {
    let mut reader = Reader {
        builder: Builder::default(),
        open: false,
        gap: false,
        listing: false,
    };
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if fenced {
            if trimmed.trim_start().starts_with("```") {
                fenced = false;
                reader.gap = true;
                continue;
            }
            reader.start(ParagraphAttributes::default(), false);
            reader.builder.text(trimmed, &code());
            reader.close();
            continue;
        }
        let content = trimmed.trim_start();
        if content.is_empty() {
            reader.close();
            reader.gap = true;
            continue;
        }
        if content.starts_with("```") {
            reader.close();
            reader.gap = !reader.builder.is_empty();
            reader.start(ParagraphAttributes::default(), false);
            reader.open = false;
            fenced = true;
            continue;
        }
        if content
            .chars()
            .all(|character| matches!(character, '-' | '*' | '_' | ' '))
            && content
                .chars()
                .filter(|character| *character != ' ')
                .count()
                >= 3
        {
            reader.close();
            reader.gap = true;
            continue;
        }
        let hashes = content
            .chars()
            .take_while(|character| *character == '#')
            .count();
        if (1..=6).contains(&hashes) && content[hashes..].starts_with(' ') {
            reader.start(ParagraphAttributes::default(), false);
            let title = content[hashes..].trim().trim_end_matches('#').trim_end();
            inline(&mut reader.builder, title, &heading(hashes));
            reader.close();
            reader.gap = true;
            continue;
        }
        if let Some((level, marker, rest)) = list_item(trimmed) {
            let attributes = ParagraphAttributes {
                list: Some(marker),
                level: level as u8,
                ..ParagraphAttributes::default()
            };
            reader.start(attributes, true);
            inline(&mut reader.builder, rest, &TextAttributes::default());
            reader.close();
            continue;
        }
        if let Some(quoted) = content.strip_prefix('>') {
            let attributes = ParagraphAttributes {
                level: 1,
                ..ParagraphAttributes::default()
            };
            if !reader.open {
                reader.start(attributes, false);
            } else {
                reader.builder.text(" ", &TextAttributes::default());
            }
            let italic = TextAttributes {
                italic: true,
                ..TextAttributes::default()
            };
            inline(&mut reader.builder, quoted.trim(), &italic);
            continue;
        }
        match reader.open && !reader.listing {
            true => reader.builder.text(" ", &TextAttributes::default()),
            false => reader.start(ParagraphAttributes::default(), false),
        }
        inline(&mut reader.builder, content, &TextAttributes::default());
    }
    reader.close();
    reader.builder.finish()
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        if matches!(character, '\\' | '*' | '_' | '`' | '[' | ']' | '~') {
            out.push('\\');
        }
        out.push(character);
    }
}

fn is_code_block(paragraph: &Paragraph<'_>) -> bool {
    paragraph.attributes.list.is_none()
        && !paragraph.is_empty()
        && paragraph
            .spans
            .iter()
            .all(|(_, attributes)| is_code(attributes))
}

fn line(paragraph: &Paragraph<'_>, number: usize) -> String {
    let mut out = String::new();
    let attributes = paragraph.attributes;
    let level = heading_level(&paragraph.first());
    match (attributes.list, level) {
        (Some(ListMarker::Bullet), _) => {
            out.push_str(&"  ".repeat(usize::from(attributes.level)));
            out.push_str("- ");
        }
        (Some(ListMarker::Number), _) => {
            out.push_str(&"  ".repeat(usize::from(attributes.level)));
            out.push_str(&format!("{number}. "));
        }
        (None, Some(level)) => {
            out.push_str(&"#".repeat(level));
            out.push(' ');
        }
        (None, None) if attributes.level > 0 => out.push_str("> "),
        (None, None) => {}
    }
    for (text, span) in &paragraph.spans {
        if text.is_empty() {
            continue;
        }
        let mut open = String::new();
        if level.is_none() && span.bold {
            open.push_str("**");
        }
        if span.italic && attributes.level == 0 || span.italic && attributes.list.is_some() {
            open.push('*');
        }
        if span.strikethrough {
            open.push_str("~~");
        }
        let close: String = open.chars().rev().collect();
        out.push_str(&open);
        match (&span.link, is_code(span)) {
            (_, true) => {
                out.push('`');
                out.push_str(&text.replace('`', "'"));
                out.push('`');
            }
            (Some(url), false) => {
                out.push('[');
                escape(text, &mut out);
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
            (None, false) => escape(text, &mut out),
        }
        out.push_str(&close);
    }
    out
}

pub fn write(rich: &RichText) -> String {
    let paragraphs = paragraphs(rich);
    let mut out = String::new();
    let mut previous_list = false;
    let mut gap = false;
    let mut in_code = false;
    let mut numbers = [0usize; MAX_LEVEL + 1];
    for (index, paragraph) in paragraphs.iter().enumerate() {
        let next_code = paragraphs[index + 1..]
            .iter()
            .find(|paragraph| !paragraph.is_empty())
            .is_some_and(is_code_block);
        if in_code {
            if is_code_block(paragraph) || paragraph.is_empty() && next_code {
                let text: String = paragraph.spans.iter().map(|(text, _)| *text).collect();
                out.push_str(&text);
                out.push('\n');
                continue;
            }
            out.push_str("```\n");
            in_code = false;
        }
        if paragraph.is_empty() {
            gap = true;
            continue;
        }
        let listed = paragraph.attributes.list.is_some();
        if !out.is_empty() && (gap || !(previous_list && listed)) {
            out.push('\n');
        }
        let gap_before = std::mem::take(&mut gap);
        if is_code_block(paragraph) {
            out.push_str("```\n");
            in_code = true;
            let text: String = paragraph.spans.iter().map(|(text, _)| *text).collect();
            out.push_str(&text);
            out.push('\n');
            previous_list = false;
            continue;
        }
        let level = usize::from(paragraph.attributes.level).min(MAX_LEVEL);
        if paragraph.attributes.list != Some(ListMarker::Number) || gap_before {
            numbers[level] = 0;
        }
        let number = match paragraph.attributes.list {
            Some(_) => {
                numbers[level] += 1;
                numbers[level + 1..].fill(0);
                numbers[level]
            }
            None => {
                numbers = [0; MAX_LEVEL + 1];
                0
            }
        };
        out.push_str(&line(paragraph, number));
        out.push('\n');
        previous_list = listed;
    }
    if in_code {
        out.push_str("```\n");
    }
    out
}
