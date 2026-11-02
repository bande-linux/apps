use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write};

use bande_toolkit::Color;
use bande_toolkit::ui::{Alignment, ListMarker, ParagraphAttributes, RichText, TextAttributes};
use roxmltree::{Document, Node};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::{Builder, MAX_NESTING, Opened, heading, heading_level, link, paragraphs, part, xml};

const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
const XLINK: &str = "http://www.w3.org/1999/xlink";
const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const BASE_POINTS: f32 = 12.0;
const BASE_SIZE: f32 = 14.0;
const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

#[derive(Default, Clone)]
struct Style {
    text: TextAttributes,
    alignment: Option<Alignment>,
    bullet: Option<Vec<ListMarker>>,
}

struct Reader {
    builder: Builder,
    styles: HashMap<String, Style>,
    lossy: bool,
}

fn properties(style: Node<'_, '_>, base: &Style) -> Style {
    let mut parsed = base.clone();
    for properties in style.children().filter(Node::is_element) {
        let attribute = |namespace: &str, name: &str| properties.attribute((namespace, name));
        match properties.tag_name().name() {
            "text-properties" => {
                let text = &mut parsed.text;
                if let Some(weight) = attribute(FO, "font-weight") {
                    text.bold = matches!(weight, "bold" | "600" | "700" | "800" | "900");
                }
                if let Some(style) = attribute(FO, "font-style") {
                    text.italic = style == "italic" || style == "oblique";
                }
                if let Some(underline) = attribute(STYLE, "text-underline-style") {
                    text.underline = underline != "none";
                }
                if let Some(through) = attribute(STYLE, "text-line-through-style") {
                    text.strikethrough = through != "none";
                }
                if let Some(size) = attribute(FO, "font-size")
                    .and_then(|size| size.strip_suffix("pt"))
                    .and_then(|size| size.parse::<f32>().ok())
                {
                    let size = (size / BASE_POINTS * BASE_SIZE).clamp(6.0, 96.0);
                    text.size = Some(size).filter(|size| (size - BASE_SIZE).abs() > 0.5);
                }
                if let Some(color) = attribute(FO, "color")
                    .and_then(|color| color.strip_prefix('#'))
                    .and_then(|color| u32::from_str_radix(color, 16).ok())
                {
                    text.color = Some(Color::hex(color)).filter(|color| *color != Color::BLACK);
                }
                if let Some(family) = attribute(STYLE, "font-name") {
                    text.family = super::family(family);
                }
            }
            "paragraph-properties" => {
                parsed.alignment = attribute(FO, "text-align").map(|align| match align {
                    "center" => Alignment::Center,
                    "end" | "right" => Alignment::End,
                    "justify" => Alignment::Justify,
                    _ => Alignment::Start,
                });
            }
            _ => {}
        }
    }
    parsed
}

fn collect_styles(document: &Document<'_>, styles: &mut HashMap<String, Style>) {
    for style in document.descendants().filter(Node::is_element) {
        let Some(name) = style.attribute((STYLE, "name")) else {
            continue;
        };
        match style.tag_name().name() {
            "style" => {
                let base = style
                    .attribute((STYLE, "parent-style-name"))
                    .and_then(|parent| styles.get(parent))
                    .cloned()
                    .unwrap_or_default();
                let parsed = properties(style, &base);
                styles.insert(name.to_owned(), parsed);
            }
            "list-style" => {
                let markers = style
                    .children()
                    .filter(Node::is_element)
                    .map(|level| match level.tag_name().name() {
                        "list-level-style-number" => ListMarker::Number,
                        _ => ListMarker::Bullet,
                    })
                    .collect();
                styles.insert(
                    name.to_owned(),
                    Style {
                        bullet: Some(markers),
                        ..Style::default()
                    },
                );
            }
            _ => {}
        }
    }
}

impl Reader {
    fn style(&self, node: Node<'_, '_>) -> Style {
        node.attribute((TEXT, "style-name"))
            .and_then(|name| self.styles.get(name))
            .cloned()
            .unwrap_or_default()
    }

    fn inline(&mut self, node: Node<'_, '_>, attributes: &TextAttributes, depth: usize) {
        if depth > MAX_NESTING {
            return;
        }
        for child in node.children() {
            if child.is_text() {
                let text = child
                    .text()
                    .unwrap_or_default()
                    .replace(['\n', '\r', '\t'], " ");
                self.builder.text(&text, attributes);
                continue;
            }
            match child.tag_name().name() {
                "span" => {
                    let style = self.style(child);
                    let mut merged = attributes.clone();
                    let text = style.text;
                    merged.bold |= text.bold;
                    merged.italic |= text.italic;
                    merged.underline |= text.underline;
                    merged.strikethrough |= text.strikethrough;
                    merged.size = text.size.or(merged.size);
                    merged.color = text.color.or(merged.color);
                    merged.family = text.family.or(merged.family);
                    self.inline(child, &merged, depth + 1);
                }
                "a" => {
                    let linked = match child.attribute((XLINK, "href")) {
                        Some(url) => TextAttributes {
                            bold: attributes.bold,
                            size: attributes.size,
                            ..link(url)
                        },
                        None => attributes.clone(),
                    };
                    self.inline(child, &linked, depth + 1);
                }
                "s" => {
                    let count = child
                        .attribute((TEXT, "c"))
                        .and_then(|count| count.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(64);
                    self.builder.text(&" ".repeat(count), attributes);
                }
                "tab" => self.builder.text("\t", attributes),
                "line-break" => self.builder.line_break(),
                "frame" | "image" | "object" => {
                    self.lossy = true;
                    self.builder.text("[image]", attributes);
                }
                "note" | "annotation" | "bookmark" | "bookmark-start" | "bookmark-end" => {}
                _ => self.inline(child, attributes, depth + 1),
            }
        }
    }

    fn block(&mut self, node: Node<'_, '_>, list: Option<(&[ListMarker], u8)>, depth: usize) {
        if depth > MAX_NESTING {
            return;
        }
        for child in node.children().filter(Node::is_element) {
            match child.tag_name().name() {
                name @ ("p" | "h") => {
                    let style = self.style(child);
                    let mut attributes = ParagraphAttributes {
                        alignment: style.alignment.unwrap_or_default(),
                        ..ParagraphAttributes::default()
                    };
                    if let Some((markers, level)) = list {
                        attributes.list = Some(
                            markers
                                .get(usize::from(level))
                                .copied()
                                .unwrap_or(ListMarker::Bullet),
                        );
                        attributes.level = level;
                    }
                    let text = match name {
                        "h" => {
                            let level = child
                                .attribute((TEXT, "outline-level"))
                                .and_then(|level| level.parse::<usize>().ok())
                                .unwrap_or(1);
                            heading(level.clamp(1, 6))
                        }
                        _ => style.text,
                    };
                    *self.builder.paragraph() = attributes;
                    self.inline(child, &text, depth + 1);
                    self.builder.end_paragraph(ParagraphAttributes::default());
                }
                "list" => {
                    let markers = self
                        .style(child)
                        .bullet
                        .or_else(|| list.map(|(markers, _)| markers.to_vec()))
                        .unwrap_or_default();
                    let level = list.map_or(0, |(_, level)| (level + 1).min(8));
                    for item in child.children().filter(Node::is_element) {
                        self.block(item, Some((&markers, level)), depth + 1);
                    }
                }
                "table" => {
                    self.lossy = true;
                    for row in child
                        .descendants()
                        .filter(|node| node.has_tag_name((TABLE, "table-row")))
                    {
                        let mut first = true;
                        for cell in row
                            .children()
                            .filter(|node| node.has_tag_name((TABLE, "table-cell")))
                        {
                            if !std::mem::take(&mut first) {
                                self.builder.text("\t", &TextAttributes::default());
                            }
                            for paragraph in cell.children().filter(Node::is_element) {
                                self.inline(paragraph, &TextAttributes::default(), depth + 1);
                            }
                        }
                        self.builder.end_paragraph(ParagraphAttributes::default());
                    }
                }
                "section" | "list-item" | "list-header" => self.block(child, list, depth + 1),
                "frame" => {
                    self.lossy = true;
                }
                _ => {}
            }
        }
    }
}

pub fn read(bytes: &[u8]) -> Result<Opened, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let content = part(&mut archive, "content.xml")?;
    let shared = part(&mut archive, "styles.xml").unwrap_or_default();
    let mut styles = HashMap::new();
    if let Ok(document) = Document::parse_with_options(&shared, super::options()) {
        collect_styles(&document, &mut styles);
    }
    let document = Document::parse_with_options(&content, super::options())
        .map_err(|error| error.to_string())?;
    collect_styles(&document, &mut styles);
    let mut reader = Reader {
        builder: Builder::default(),
        styles,
        lossy: false,
    };
    if let Some(text) = document
        .descendants()
        .find(|node| node.has_tag_name((OFFICE, "text")))
    {
        reader.block(text, None, 0);
    }
    Ok(Opened {
        rich: reader.builder.finish(),
        lossy: reader.lossy,
    })
}

fn text_style(attributes: &TextAttributes) -> String {
    let mut properties = String::new();
    if attributes.bold {
        properties.push_str(" fo:font-weight=\"bold\"");
    }
    if attributes.italic {
        properties.push_str(" fo:font-style=\"italic\"");
    }
    if attributes.underline {
        properties.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"");
    }
    if attributes.strikethrough {
        properties.push_str(" style:text-line-through-style=\"solid\"");
    }
    if let Some(size) = attributes.size {
        let _ = write!(
            properties,
            " fo:font-size=\"{:.1}pt\"",
            size / BASE_SIZE * BASE_POINTS
        );
    }
    if let Some(color) = attributes.color {
        let _ = write!(
            properties,
            " fo:color=\"#{:02x}{:02x}{:02x}\"",
            color.r, color.g, color.b
        );
    }
    if let Some(family) = &attributes.family {
        let _ = write!(
            properties,
            " style:font-name=\"{0}\" fo:font-family=\"{0}\"",
            xml(family)
        );
    }
    properties
}

pub fn write(rich: &RichText) -> Result<Vec<u8>, String> {
    let mut automatic = String::new();
    let mut spans: Vec<String> = Vec::new();
    let mut body = String::new();
    let mut open: Vec<ListMarker> = Vec::new();
    for paragraph in paragraphs(rich) {
        let attributes = paragraph.attributes;
        let depth = attributes
            .list
            .map_or(0, |_| usize::from(attributes.level) + 1);
        while open.len() > depth
            || open.len() == depth && depth > 0 && open.last().copied() != attributes.list
        {
            open.pop();
            body.push_str("</text:list-item></text:list>");
        }
        if open.len() == depth && depth > 0 {
            body.push_str("</text:list-item><text:list-item>");
        }
        while open.len() < depth {
            let marker = attributes.list.unwrap_or(ListMarker::Bullet);
            let style = match marker {
                ListMarker::Bullet => "Bullets",
                ListMarker::Number => "Numbers",
            };
            let _ = write!(
                body,
                "<text:list text:style-name=\"{style}\"><text:list-item>"
            );
            open.push(marker);
        }
        let alignment = match attributes.alignment {
            Alignment::Start => "P",
            Alignment::Center => "Centre",
            Alignment::End => "Right",
            Alignment::Justify => "Justify",
        };
        let level = heading_level(&paragraph.first()).filter(|_| depth == 0);
        match level {
            Some(level) => {
                let _ = write!(
                    body,
                    "<text:h text:style-name=\"{alignment}\" text:outline-level=\"{level}\">"
                );
            }
            None => {
                let _ = write!(body, "<text:p text:style-name=\"{alignment}\">");
            }
        }
        for (text, span) in &paragraph.spans {
            if text.is_empty() {
                continue;
            }
            let span_style = text_style(&TextAttributes {
                bold: span.bold && level.is_none(),
                size: span.size.filter(|_| level.is_none()),
                ..span.clone()
            });
            let mut content = String::new();
            for (index, piece) in text.split('\t').enumerate() {
                if index > 0 {
                    content.push_str("<text:tab/>");
                }
                content.push_str(&xml(piece));
            }
            if let Some(url) = &span.link {
                content = format!(
                    "<text:a xlink:type=\"simple\" xlink:href=\"{}\">{content}</text:a>",
                    xml(url)
                );
            }
            match span_style.is_empty() {
                true => body.push_str(&content),
                false => {
                    let index = match spans.iter().position(|known| *known == span_style) {
                        Some(index) => index,
                        None => {
                            spans.push(span_style.clone());
                            let _ = write!(
                                automatic,
                                "<style:style style:name=\"T{}\" style:family=\"text\"><style:text-properties{span_style}/></style:style>",
                                spans.len()
                            );
                            spans.len() - 1
                        }
                    };
                    let _ = write!(
                        body,
                        "<text:span text:style-name=\"T{}\">{content}</text:span>",
                        index + 1
                    );
                }
            }
        }
        body.push_str(match level {
            Some(_) => "</text:h>",
            None => "</text:p>",
        });
    }
    for _ in open.drain(..) {
        body.push_str("</text:list-item></text:list>");
    }
    let bullets: String = (1..=10)
        .map(|level| {
            format!("<text:list-level-style-bullet text:level=\"{level}\" text:bullet-char=\"•\"/>")
        })
        .collect();
    let numbers: String = (1..=10)
        .map(|level| format!("<text:list-level-style-number text:level=\"{level}\" style:num-format=\"1\" style:num-suffix=\".\"/>"))
        .collect();
    let paragraph_styles: String = [("P", "start"), ("Centre", "center"), ("Right", "end"), ("Justify", "justify")]
        .iter()
        .map(|(name, align)| {
            format!("<style:style style:name=\"{name}\" style:family=\"paragraph\"><style:paragraph-properties fo:text-align=\"{align}\"/></style:style>")
        })
        .collect();
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content xmlns:office=\"{OFFICE}\" xmlns:style=\"{STYLE}\" xmlns:text=\"{TEXT}\" xmlns:fo=\"{FO}\" xmlns:xlink=\"{XLINK}\" xmlns:table=\"{TABLE}\" office:version=\"1.3\">\
         <office:automatic-styles>{paragraph_styles}{automatic}<text:list-style style:name=\"Bullets\">{bullets}</text:list-style><text:list-style style:name=\"Numbers\">{numbers}</text:list-style></office:automatic-styles>\
         <office:body><office:text>{body}</office:text></office:body></office:document-content>"
    );
    let manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\
         <manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{MIMETYPE}\"/>\
         <manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>"
    );
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, data, options) in [
        ("mimetype", MIMETYPE.to_owned(), stored),
        (
            "META-INF/manifest.xml",
            manifest,
            SimpleFileOptions::default(),
        ),
        ("content.xml", content, SimpleFileOptions::default()),
    ] {
        zip.start_file(name, options)
            .map_err(|error| error.to_string())?;
        zip.write_all(data.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    Ok(zip
        .finish()
        .map_err(|error| error.to_string())?
        .into_inner())
}
