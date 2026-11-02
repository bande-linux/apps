use std::cell::RefCell;

use bande_toolkit::Color;
use bande_toolkit::ui::{Alignment, ListMarker, ParagraphAttributes, RichText, TextAttributes};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use super::{Builder, MAX_NESTING, code, heading, heading_level, is_code, link, paragraphs};

const BLOCKS: [&str; 16] = [
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "blockquote",
    "tr",
    "dt",
    "dd",
    "figure",
    "table",
    "hr",
];

#[derive(Default)]
struct Reader {
    builder: Builder,
    styles: Vec<(String, TextAttributes)>,
    lists: Vec<ListMarker>,
    skipping: Option<String>,
    preformatted: usize,
    cell: bool,
    space: bool,
}

impl Reader {
    fn current(&self) -> TextAttributes {
        self.styles
            .last()
            .map(|(_, attributes)| attributes.clone())
            .unwrap_or_default()
    }

    fn block(&mut self) {
        if !self.builder.is_empty_paragraph() {
            self.builder.end_paragraph(ParagraphAttributes::default());
        }
        self.space = false;
        self.cell = false;
    }

    fn push(&mut self, name: &str, change: impl FnOnce(&mut TextAttributes)) {
        if self.styles.len() >= MAX_NESTING {
            return;
        }
        let mut attributes = self.current();
        change(&mut attributes);
        self.styles.push((name.to_owned(), attributes));
    }

    fn pop(&mut self, name: &str) {
        if let Some(index) = self.styles.iter().rposition(|(open, _)| open == name) {
            self.styles.truncate(index);
        }
    }

    fn text(&mut self, text: &str) {
        if self.skipping.is_some() {
            return;
        }
        let attributes = self.current();
        if self.preformatted > 0 {
            let mut lines = text.split('\n');
            if let Some(first) = lines.next() {
                self.builder.text(first, &attributes);
            }
            for line in lines {
                self.builder.end_paragraph(ParagraphAttributes::default());
                self.builder.text(line, &attributes);
            }
            return;
        }
        let mut collapsed = String::new();
        for character in text.chars() {
            match character.is_whitespace() {
                true if self.space || self.builder.is_empty_paragraph() && collapsed.is_empty() => {
                }
                true => {
                    collapsed.push(' ');
                    self.space = true;
                }
                false => {
                    collapsed.push(character);
                    self.space = false;
                }
            }
        }
        self.builder.text(&collapsed, &attributes);
    }

    fn start(&mut self, tag: &Tag) {
        let name = tag.name.to_string();
        let attribute = |wanted: &str| {
            tag.attrs
                .iter()
                .find(|attribute| &*attribute.name.local == wanted)
                .map(|attribute| attribute.value.to_string())
        };
        let style = attribute("style").unwrap_or_default().to_ascii_lowercase();
        let alignment = attribute("align")
            .map(|align| format!("text-align:{align}"))
            .unwrap_or_default()
            + &style;
        match name.as_str() {
            "script" | "style" | "title" | "template" | "noscript" | "head"
                if !tag.self_closing =>
            {
                self.skipping = Some(name);
            }
            "b" | "strong" => self.push(&name, |attributes| attributes.bold = true),
            "i" | "em" | "cite" | "var" => self.push(&name, |attributes| attributes.italic = true),
            "u" | "ins" => self.push(&name, |attributes| attributes.underline = true),
            "s" | "strike" | "del" => {
                self.push(&name, |attributes| attributes.strikethrough = true)
            }
            "code" | "tt" | "kbd" | "samp" => {
                self.push(&name, |attributes| attributes.family = code().family)
            }
            "a" => {
                let href = attribute("href").unwrap_or_default();
                self.push(&name, |attributes| {
                    if !href.is_empty() {
                        *attributes = TextAttributes {
                            bold: attributes.bold,
                            italic: attributes.italic,
                            size: attributes.size,
                            ..link(&href)
                        };
                    }
                });
            }
            "span" | "font" => {
                let color = attribute("color").and_then(|color| parse_color(&color));
                let face = attribute("face");
                self.push(&name, |attributes| {
                    if style.contains("font-weight:bold")
                        || style.contains("font-weight: bold")
                        || style.contains("font-weight:700")
                    {
                        attributes.bold = true;
                    }
                    if style.contains("italic") {
                        attributes.italic = true;
                    }
                    if style.contains("underline") {
                        attributes.underline = true;
                    }
                    if style.contains("line-through") {
                        attributes.strikethrough = true;
                    }
                    let styled = style
                        .split(';')
                        .find_map(|rule| rule.trim().strip_prefix("color:"))
                        .and_then(|color| parse_color(color.trim()));
                    if let Some(color) = styled.or(color) {
                        attributes.color = Some(color);
                    }
                    let named = style
                        .split(';')
                        .find_map(|rule| rule.trim().strip_prefix("font-family:"))
                        .and_then(|families| families.split(',').next())
                        .map(|family| family.trim().trim_matches(['\'', '"']).to_owned())
                        .or(face);
                    if let Some(named) = named {
                        attributes.family = super::family(&named);
                    }
                });
            }
            tag @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                self.block();
                let level = usize::from(tag.as_bytes()[1] - b'0');
                self.align(&alignment);
                self.push(&name, |attributes| *attributes = heading(level));
            }
            "br" => {
                self.builder.line_break();
                self.space = false;
            }
            "ul" | "menu" => {
                self.block();
                if self.lists.len() < MAX_NESTING {
                    self.lists.push(ListMarker::Bullet);
                }
            }
            "ol" => {
                self.block();
                if self.lists.len() < MAX_NESTING {
                    self.lists.push(ListMarker::Number);
                }
            }
            "li" => {
                self.block();
                let paragraph = self.builder.paragraph();
                paragraph.list = Some(self.lists.last().copied().unwrap_or(ListMarker::Bullet));
                paragraph.level = self.lists.len().saturating_sub(1).min(8) as u8;
            }
            "pre" => {
                self.block();
                self.preformatted += 1;
                self.push(&name, |attributes| attributes.family = code().family);
            }
            "td" | "th" => {
                if std::mem::replace(&mut self.cell, true) {
                    self.builder.text("\t", &TextAttributes::default());
                }
                self.space = true;
                if name == "th" {
                    self.push(&name, |attributes| attributes.bold = true);
                }
            }
            "img" => {
                if let Some(alt) = attribute("alt").filter(|alt| !alt.trim().is_empty()) {
                    self.builder
                        .text(&format!("[{}]", alt.trim()), &self.current());
                }
            }
            block if BLOCKS.contains(&block) => {
                self.block();
                if block == "blockquote" {
                    self.builder.paragraph().level = 1;
                }
                self.align(&alignment);
            }
            _ => {}
        }
    }

    fn align(&mut self, style: &str) {
        let alignment = if style.contains("text-align:center")
            || style.contains("text-align: center")
        {
            Alignment::Center
        } else if style.contains("text-align:right") || style.contains("text-align: right") {
            Alignment::End
        } else if style.contains("text-align:justify") || style.contains("text-align: justify") {
            Alignment::Justify
        } else {
            return;
        };
        self.builder.paragraph().alignment = alignment;
    }

    fn end(&mut self, name: &str) {
        if self.skipping.as_deref() == Some(name) {
            self.skipping = None;
            return;
        }
        match name {
            "ul" | "ol" | "menu" => {
                self.block();
                self.lists.pop();
            }
            "pre" => {
                self.block();
                self.preformatted = self.preformatted.saturating_sub(1);
                self.pop(name);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.pop(name);
                self.block();
            }
            "li" => self.block(),
            block if BLOCKS.contains(&block) => self.block(),
            _ => self.pop(name),
        }
    }
}

fn parse_color(color: &str) -> Option<Color> {
    let hex = color.strip_prefix('#')?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(Color::hex(value)),
        3 => {
            let [red, green, blue] =
                [value >> 8, value >> 4, value].map(|channel| (channel & 0xf) * 17);
            Some(Color::hex(red << 16 | green << 8 | blue))
        }
        _ => None,
    }
}

struct Sink(RefCell<Reader>);

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut reader = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => {
                    reader.start(&tag);
                    match tag.name.as_ref() {
                        "script" | "style" if !tag.self_closing => {
                            return TokenSinkResult::RawData(RawKind::ScriptData);
                        }
                        "title" | "textarea" if !tag.self_closing => {
                            return TokenSinkResult::RawData(RawKind::Rcdata);
                        }
                        _ => {}
                    }
                }
                TagKind::EndTag => reader.end(tag.name.as_ref()),
            },
            Token::CharacterTokens(text) => reader.text(&text),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

pub fn read(html: &str) -> RichText {
    let tokenizer = Tokenizer::new(
        Sink(RefCell::new(Reader::default())),
        TokenizerOpts::default(),
    );
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.0.into_inner().builder.finish()
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            character => out.push(character),
        }
    }
}

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

pub fn write(rich: &RichText, title: &str) -> String {
    let mut out =
        String::from("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>");
    escape(title, &mut out);
    out.push_str("</title>\n</head>\n<body>\n");
    let mut lists: Vec<ListMarker> = Vec::new();
    let mut table = false;
    for paragraph in paragraphs(rich) {
        let attributes = paragraph.attributes;
        let row = attributes.list.is_none()
            && paragraph.spans.iter().any(|(text, _)| text.contains('\t'));
        if table && !row {
            out.push_str("</table>\n");
        }
        let depth = match attributes.list {
            Some(_) => usize::from(attributes.level) + 1,
            None => 0,
        };
        while lists.len() > depth
            || lists.len() == depth && depth > 0 && lists.last().copied() != attributes.list
        {
            out.push_str(match lists.pop() {
                Some(ListMarker::Number) => "</ol>\n",
                _ => "</ul>\n",
            });
        }
        while lists.len() < depth {
            let marker = attributes.list.unwrap_or(ListMarker::Bullet);
            out.push_str(match marker {
                ListMarker::Number => "<ol>\n",
                ListMarker::Bullet => "<ul>\n",
            });
            lists.push(marker);
        }
        if row && !std::mem::replace(&mut table, true) {
            out.push_str("<table>\n");
        }
        table = row;
        let level = heading_level(&paragraph.first()).filter(|_| depth == 0);
        let tag = match (depth, level) {
            _ if row => "tr".to_owned(),
            (0, Some(level)) => format!("h{level}"),
            (0, None)
                if paragraph.spans.iter().all(|(_, span)| is_code(span))
                    && !paragraph.is_empty() =>
            {
                "pre".to_owned()
            }
            (0, None) => "p".to_owned(),
            _ => "li".to_owned(),
        };
        let align = match attributes.alignment {
            Alignment::Start => "",
            Alignment::Center => " style=\"text-align:center\"",
            Alignment::End => " style=\"text-align:right\"",
            Alignment::Justify => " style=\"text-align:justify\"",
        };
        out.push_str(&format!("<{tag}{align}>"));
        if row {
            out.push_str("<td>");
        }
        if paragraph.is_empty() && tag == "p" {
            out.push_str("<br>");
        }
        for (text, span) in &paragraph.spans {
            if text.is_empty() {
                continue;
            }
            let mut open = String::new();
            let mut close = Vec::new();
            let mut wrap = |start: String, end: &'static str| {
                open.push_str(&start);
                close.push(end);
            };
            if let Some(url) = &span.link {
                let mut href = String::new();
                escape(url, &mut href);
                wrap(format!("<a href=\"{href}\">"), "</a>");
            }
            if span.bold && level.is_none() {
                wrap("<b>".into(), "</b>");
            }
            if span.italic {
                wrap("<i>".into(), "</i>");
            }
            if span.underline && span.link.is_none() {
                wrap("<u>".into(), "</u>");
            }
            if span.strikethrough {
                wrap("<s>".into(), "</s>");
            }
            if is_code(span) && tag != "pre" {
                wrap("<code>".into(), "</code>");
            }
            let mut css = String::new();
            if let Some(family) = span.family.as_ref().filter(|_| !is_code(span)) {
                css.push_str(&format!(
                    "font-family:'{}';",
                    family.replace(['\'', '"', '<', '>'], "")
                ));
            }
            if let Some(size) = span.size.filter(|_| level.is_none()) {
                css.push_str(&format!("font-size:{size}px;"));
            }
            if let Some(color) = span.color.filter(|_| span.link.is_none()) {
                css.push_str(&format!("color:{};", hex(color)));
            }
            if !css.is_empty() {
                wrap(format!("<span style=\"{css}\">"), "</span>");
            }
            for (index, piece) in text.split('\t').enumerate() {
                if index > 0 {
                    out.push_str("</td><td>");
                }
                out.push_str(&open);
                escape(piece, &mut out);
                for end in close.iter().rev() {
                    out.push_str(end);
                }
            }
        }
        if row {
            out.push_str("</td>");
        }
        out.push_str(&format!("</{tag}>\n"));
    }
    while let Some(marker) = lists.pop() {
        out.push_str(match marker {
            ListMarker::Number => "</ol>\n",
            ListMarker::Bullet => "</ul>\n",
        });
    }
    if table {
        out.push_str("</table>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}
