pub mod docx;
pub mod html;
pub mod markdown;
pub mod odt;
pub mod rtf;

use std::io::{Read, Seek};
use std::sync::Arc;

use bande_toolkit::Color;
use bande_toolkit::ui::{ParagraphAttributes, RichText, TextAttributes};

pub const MONOSPACE: &str = "JetBrains Mono";
pub const HEADINGS: [f32; 6] = [26.0, 22.0, 18.0, 16.0, 15.0, 14.0];
pub const LINK: Color = Color::hex(0x0066D9);
pub const MAX_NESTING: usize = 64;
const MAX_PART_BYTES: u64 = 64 << 20;
const MAX_XML_NODES: u32 = 4_000_000;

pub struct Opened {
    pub rich: RichText,
    pub lossy: bool,
}

pub fn options<'input>() -> roxmltree::ParsingOptions<'input> {
    roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..roxmltree::ParsingOptions::default()
    }
}

pub fn part(archive: &mut zip::ZipArchive<impl Read + Seek>, name: &str) -> Result<String, String> {
    let file = archive.by_name(name).map_err(|error| error.to_string())?;
    if file.size() > MAX_PART_BYTES {
        return Err(format!("{name} is too large"));
    }
    let mut text = String::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    match text.len() as u64 > MAX_PART_BYTES {
        true => Err(format!("{name} is too large")),
        false => Ok(text),
    }
}

pub fn xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            character if (character as u32) < 0x20 && character != '\t' => {}
            character => escaped.push(character),
        }
    }
    escaped
}

pub fn heading(level: usize) -> TextAttributes {
    TextAttributes {
        bold: true,
        size: HEADINGS.get(level.saturating_sub(1)).copied(),
        ..TextAttributes::default()
    }
}

pub fn heading_level(attributes: &TextAttributes) -> Option<usize> {
    let size = attributes.size?;
    HEADINGS
        .iter()
        .position(|heading| (heading - size).abs() < 0.5)
        .filter(|_| attributes.bold)
        .map(|index| index + 1)
}

pub const SERIF: &str = "Noto Serif";

pub fn family(name: &str) -> Option<Arc<str>> {
    let lower = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    match () {
        _ if has(&[
            "mono", "courier", "menlo", "monaco", "consolas", "code", "fixed",
        ]) =>
        {
            Some(Arc::from(MONOSPACE))
        }
        _ if has(&[
            "sans",
            "helvetica",
            "arial",
            "calibri",
            "inter",
            "verdana",
            "segoe",
            "tahoma",
            "lucida grande",
            "aptos",
            "carlito",
            "liberation sans",
        ]) =>
        {
            None
        }
        _ if has(&[
            "serif",
            "times",
            "georgia",
            "garamond",
            "cambria",
            "palatino",
            "baskerville",
            "caladea",
        ]) =>
        {
            Some(Arc::from(SERIF))
        }
        _ if name.trim().is_empty() => None,
        _ => Some(Arc::from(name.trim())),
    }
}

pub fn code() -> TextAttributes {
    TextAttributes {
        family: Some(Arc::from(MONOSPACE)),
        ..TextAttributes::default()
    }
}

pub fn is_code(attributes: &TextAttributes) -> bool {
    attributes.family.as_deref() == Some(MONOSPACE)
}

pub fn link(url: &str) -> TextAttributes {
    TextAttributes {
        underline: true,
        color: Some(LINK),
        link: Some(Arc::from(url)),
        ..TextAttributes::default()
    }
}

#[derive(Default)]
pub struct Builder {
    rich: RichText,
    paragraph: ParagraphAttributes,
}

impl Builder {
    pub fn text(&mut self, text: &str, attributes: &TextAttributes) {
        let text = text.replace(['\n', '\r'], " ");
        if text.is_empty() {
            return;
        }
        let start = self.rich.text.len();
        self.rich.text.push_str(&text);
        if *attributes != TextAttributes::default() {
            match self.rich.runs.last_mut() {
                Some((range, last)) if range.end == start && last == attributes => {
                    range.end = self.rich.text.len();
                }
                _ => self
                    .rich
                    .runs
                    .push((start..self.rich.text.len(), attributes.clone())),
            }
        }
    }

    pub fn paragraph(&mut self) -> &mut ParagraphAttributes {
        &mut self.paragraph
    }

    pub fn is_empty_paragraph(&self) -> bool {
        self.rich.text.is_empty() || self.rich.text.ends_with('\n')
    }

    pub fn is_empty(&self) -> bool {
        self.rich.text.is_empty()
    }

    pub fn line_break(&mut self) {
        self.end_paragraph(self.paragraph);
    }

    pub fn end_paragraph(&mut self, next: ParagraphAttributes) {
        self.rich.paragraphs.push(self.paragraph);
        self.rich.text.push('\n');
        self.paragraph = next;
    }

    pub fn finish(mut self) -> RichText {
        while self.rich.text.ends_with("\n\n") {
            self.rich.text.pop();
            self.rich.paragraphs.pop();
        }
        if self.rich.text.ends_with('\n') {
            self.rich.text.pop();
        } else {
            self.rich.paragraphs.push(self.paragraph);
        }
        if self.rich.paragraphs.is_empty() {
            self.rich.paragraphs.push(self.paragraph);
        }
        self.rich
    }
}

pub struct Paragraph<'a> {
    pub attributes: ParagraphAttributes,
    pub spans: Vec<(&'a str, TextAttributes)>,
}

impl Paragraph<'_> {
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|(text, _)| text.is_empty())
    }

    pub fn first(&self) -> TextAttributes {
        self.spans
            .iter()
            .find(|(text, _)| !text.trim().is_empty())
            .map(|(_, attributes)| attributes.clone())
            .unwrap_or_default()
    }
}

pub fn paragraphs(rich: &RichText) -> Vec<Paragraph<'_>> {
    let mut paragraphs = Vec::new();
    let mut runs = rich.runs.iter().peekable();
    let mut start = 0;
    for (index, line) in rich.text.split('\n').enumerate() {
        let end = start + line.len();
        let mut spans = Vec::new();
        let mut at = start;
        while at < end {
            while runs.peek().is_some_and(|(range, _)| range.end <= at) {
                runs.next();
            }
            let (until, attributes) = match runs.peek() {
                Some((range, attributes)) if range.start <= at => {
                    (range.end.min(end), (*attributes).clone())
                }
                Some((range, _)) => (range.start.min(end), TextAttributes::default()),
                None => (end, TextAttributes::default()),
            };
            spans.push((&rich.text[at..until], attributes));
            at = until;
        }
        paragraphs.push(Paragraph {
            attributes: rich.paragraphs.get(index).copied().unwrap_or_default(),
            spans,
        });
        start = end + 1;
    }
    paragraphs
}

#[cfg(test)]
mod tests {
    use super::*;
    use bande_toolkit::ui::{Alignment, ListMarker};

    #[test]
    fn rich_formats_read_styles_and_write_them_back() {
        let source = "# Title\n\nSome **bold**, *italic*, `code`, ~~gone~~ and a [link](https://bande.org).\n\n- one\n  - nested\n\n1. first\n2. second\n\n```\nfn main() {}\n\nlet x = 1;\n```\n\n> quoted\n";
        let rich = markdown::read(source);
        assert_eq!(
            rich.text,
            "Title\n\nSome bold, italic, code, gone and a link.\n\none\nnested\n\nfirst\nsecond\n\nfn main() {}\n\nlet x = 1;\n\nquoted"
        );
        assert_eq!(markdown::write(&rich), source);
        assert_eq!(
            markdown::read("2 * 3 and snake_case **open").runs,
            Vec::new()
        );

        let rtf = rtf::read(
            br"{\rtf1\ansi{\fonttbl{\f1 Courier;}}{\colortbl;\red255\green0\blue0;}
\pard\qc{\b Bold \i both}\par
\pard{\listtext \'95\tab}\ls1 caf\'e9 \u8364? {\f1 code}{\cf1 red}\par
{\*\generator skipped}\pard plain\par}",
        );
        assert_eq!(rtf.text, "Bold both\ncafé € codered\nplain");
        assert_eq!(rtf.paragraphs[0].alignment, Alignment::Center);
        assert_eq!(rtf.paragraphs[1].list, Some(ListMarker::Bullet));
        assert_eq!(rtf::read(rtf::write(&rtf).as_bytes()), rtf);

        let page = "<head><title>T</title><script>alert(1)</script></head><h1>Heading</h1>\
            <p align=center>Some <b>bold</b> and <a href=\"https://bande.org\">a link</a> &amp; more</p>\
            <ul><li>one<ol><li>deep</li></ol></li></ul><table><tr><td>a</td><td>b</td></tr></table>";
        let html = html::read(page);
        assert_eq!(
            html.text,
            "Heading\nSome bold and a link & more\none\ndeep\na\tb"
        );
        assert_eq!(html.paragraphs[3].list, Some(ListMarker::Number));
        assert_eq!(html::read(&html::write(&html, "T")), html);
        for office in [docx::write(&html), odt::write(&html)] {
            let bytes = office.unwrap_or_default();
            let read = docx::read(&bytes)
                .or_else(|_| odt::read(&bytes))
                .map(|opened| opened.rich);
            assert_eq!(read.ok(), Some(html.clone()));
        }
    }
}
