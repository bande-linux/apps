use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write};

use bande_toolkit::Color;
use bande_toolkit::ui::{Alignment, ListMarker, ParagraphAttributes, RichText, TextAttributes};
use roxmltree::{Document, Node};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::{Builder, MAX_NESTING, Opened, heading, heading_level, link, paragraphs, part, xml};

const MAIN: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const BASE_HALF_POINTS: f32 = 22.0;
const BASE_SIZE: f32 = 14.0;

fn value<'a>(node: Node<'a, '_>, child: &str) -> Option<&'a str> {
    let found = node
        .children()
        .find(|candidate| candidate.has_tag_name((MAIN, child)))?;
    Some(found.attribute((MAIN, "val")).unwrap_or("true"))
}

fn on(node: Node<'_, '_>, child: &str) -> Option<bool> {
    value(node, child).map(|value| !matches!(value, "0" | "false" | "none"))
}

struct Reader {
    builder: Builder,
    links: HashMap<String, String>,
    numbering: HashMap<(String, u8), ListMarker>,
    styles: HashMap<String, usize>,
    lossy: bool,
}

impl Reader {
    fn run_attributes(&self, run: Node<'_, '_>, base: &TextAttributes) -> TextAttributes {
        let mut attributes = base.clone();
        let Some(properties) = run
            .children()
            .find(|child| child.has_tag_name((MAIN, "rPr")))
        else {
            return attributes;
        };
        if let Some(bold) = on(properties, "b") {
            attributes.bold = bold;
        }
        if let Some(italic) = on(properties, "i") {
            attributes.italic = italic;
        }
        if let Some(underline) = on(properties, "u") {
            attributes.underline = underline;
        }
        if let Some(strike) = on(properties, "strike") {
            attributes.strikethrough = strike;
        }
        if let Some(size) = value(properties, "sz").and_then(|size| size.parse::<f32>().ok()) {
            let size = (size / BASE_HALF_POINTS * BASE_SIZE).clamp(6.0, 96.0);
            attributes.size = Some(size).filter(|size| (size - BASE_SIZE).abs() > 0.5);
        }
        if let Some(color) = value(properties, "color")
            .filter(|color| color.len() == 6)
            .and_then(|color| u32::from_str_radix(color, 16).ok())
        {
            attributes.color = Some(Color::hex(color)).filter(|color| *color != Color::BLACK);
        }
        if let Some(fonts) = properties
            .children()
            .find(|child| child.has_tag_name((MAIN, "rFonts")))
            && let Some(family) = fonts.attribute((MAIN, "ascii"))
        {
            attributes.family = super::family(family);
        }
        attributes
    }

    fn paragraph(&mut self, paragraph: Node<'_, '_>) {
        let mut attributes = ParagraphAttributes::default();
        let mut base = TextAttributes::default();
        if let Some(properties) = paragraph
            .children()
            .find(|child| child.has_tag_name((MAIN, "pPr")))
        {
            attributes.alignment = match value(properties, "jc") {
                Some("center") => Alignment::Center,
                Some("right" | "end") => Alignment::End,
                Some("both" | "distribute") => Alignment::Justify,
                _ => Alignment::Start,
            };
            if let Some(numbering) = properties
                .children()
                .find(|child| child.has_tag_name((MAIN, "numPr")))
            {
                let level = value(numbering, "ilvl")
                    .and_then(|level| level.parse::<u8>().ok())
                    .unwrap_or(0)
                    .min(8);
                let id = value(numbering, "numId").unwrap_or("0").to_owned();
                if id != "0" {
                    attributes.list = Some(
                        self.numbering
                            .get(&(id, level))
                            .copied()
                            .unwrap_or(ListMarker::Bullet),
                    );
                    attributes.level = level;
                }
            }
            if let Some(level) =
                value(properties, "pStyle").and_then(|style| self.styles.get(style))
            {
                base = heading(*level);
            }
        }
        *self.builder.paragraph() = attributes;
        self.inline(paragraph, &base, 0);
        self.builder.end_paragraph(ParagraphAttributes::default());
    }

    fn inline(&mut self, node: Node<'_, '_>, base: &TextAttributes, depth: usize) {
        if depth > MAX_NESTING {
            return;
        }
        for child in node.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "r" => {
                    let attributes = self.run_attributes(child, base);
                    for piece in child.children().filter(Node::is_element) {
                        match piece.tag_name().name() {
                            "t" => self
                                .builder
                                .text(piece.text().unwrap_or_default(), &attributes),
                            "tab" => self.builder.text("\t", &attributes),
                            "br" | "cr" => self.builder.line_break(),
                            "drawing" | "pict" | "object" => {
                                self.lossy = true;
                                self.builder.text("[image]", &attributes);
                            }
                            _ => {}
                        }
                    }
                }
                "hyperlink" => {
                    let target = child
                        .attribute((RELATIONSHIPS, "id"))
                        .and_then(|id| self.links.get(id))
                        .cloned();
                    let attributes = match target {
                        Some(url) => TextAttributes {
                            bold: base.bold,
                            size: base.size,
                            ..link(&url)
                        },
                        None => base.clone(),
                    };
                    self.inline(child, &attributes, depth + 1);
                }
                "ins" | "smartTag" | "sdt" | "sdtContent" | "fldSimple" => {
                    self.inline(child, base, depth + 1)
                }
                _ => {}
            }
        }
    }

    fn body(&mut self, node: Node<'_, '_>, depth: usize) {
        if depth > MAX_NESTING {
            return;
        }
        for child in node.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "p" => self.paragraph(child),
                "tbl" => {
                    self.lossy = true;
                    for row in child
                        .children()
                        .filter(|row| row.has_tag_name((MAIN, "tr")))
                    {
                        let mut first = true;
                        for cell in row
                            .children()
                            .filter(|cell| cell.has_tag_name((MAIN, "tc")))
                        {
                            if !std::mem::take(&mut first) {
                                self.builder.text("\t", &TextAttributes::default());
                            }
                            for paragraph in cell
                                .descendants()
                                .filter(|node| node.has_tag_name((MAIN, "p")))
                            {
                                let attributes = TextAttributes::default();
                                self.inline(paragraph, &attributes, depth + 1);
                                self.builder.text(" ", &attributes);
                            }
                        }
                        self.builder.end_paragraph(ParagraphAttributes::default());
                    }
                }
                "sdt" | "sdtContent" | "customXml" => self.body(child, depth + 1),
                _ => {}
            }
        }
    }
}

fn numbering(xml: &str) -> HashMap<(String, u8), ListMarker> {
    let mut markers = HashMap::new();
    let Ok(document) = Document::parse_with_options(xml, super::options()) else {
        return markers;
    };
    let abstracts: HashMap<&str, Node<'_, '_>> = document
        .descendants()
        .filter(|node| node.has_tag_name((MAIN, "abstractNum")))
        .filter_map(|node| Some((node.attribute((MAIN, "abstractNumId"))?, node)))
        .collect();
    for number in document
        .descendants()
        .filter(|node| node.has_tag_name((MAIN, "num")))
    {
        let Some(id) = number.attribute((MAIN, "numId")) else {
            continue;
        };
        let Some(definition) = value(number, "abstractNumId").and_then(|id| abstracts.get(id))
        else {
            continue;
        };
        for level in definition
            .children()
            .filter(|node| node.has_tag_name((MAIN, "lvl")))
        {
            let index = level
                .attribute((MAIN, "ilvl"))
                .and_then(|level| level.parse().ok())
                .unwrap_or(0);
            let marker = match value(level, "numFmt") {
                Some("bullet" | "none") | None => ListMarker::Bullet,
                Some(_) => ListMarker::Number,
            };
            markers.insert((id.to_owned(), index), marker);
        }
    }
    markers
}

fn styles(xml: &str) -> HashMap<String, usize> {
    let mut headings = HashMap::new();
    let Ok(document) = Document::parse_with_options(xml, super::options()) else {
        return headings;
    };
    for style in document
        .descendants()
        .filter(|node| node.has_tag_name((MAIN, "style")))
    {
        let (Some(id), Some(name)) = (style.attribute((MAIN, "styleId")), value(style, "name"))
        else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        let level = match name.as_str() {
            "title" => Some(1),
            "subtitle" => Some(3),
            _ => name
                .strip_prefix("heading ")
                .and_then(|level| level.parse::<usize>().ok())
                .map(|level| level.clamp(1, 6)),
        };
        if let Some(level) = level {
            headings.insert(id.to_owned(), level);
        }
    }
    headings
}

fn links(xml: &str) -> HashMap<String, String> {
    let Ok(document) = Document::parse_with_options(xml, super::options()) else {
        return HashMap::new();
    };
    document
        .descendants()
        .filter(|node| node.tag_name().name() == "Relationship")
        .filter(|node| node.attribute("TargetMode") == Some("External"))
        .filter_map(|node| {
            Some((
                node.attribute("Id")?.to_owned(),
                node.attribute("Target")?.to_owned(),
            ))
        })
        .collect()
}

pub fn read(bytes: &[u8]) -> Result<Opened, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let document = part(&mut archive, "word/document.xml")?;
    let numbering = part(&mut archive, "word/numbering.xml")
        .map(|xml| numbering(&xml))
        .unwrap_or_default();
    let styles = part(&mut archive, "word/styles.xml")
        .map(|xml| styles(&xml))
        .unwrap_or_default();
    let links = part(&mut archive, "word/_rels/document.xml.rels")
        .map(|xml| links(&xml))
        .unwrap_or_default();
    let parsed = Document::parse_with_options(&document, super::options())
        .map_err(|error| error.to_string())?;
    let mut reader = Reader {
        builder: Builder::default(),
        links,
        numbering,
        styles,
        lossy: false,
    };
    if let Some(body) = parsed
        .descendants()
        .find(|node| node.has_tag_name((MAIN, "body")))
    {
        reader.body(body, 0);
    }
    Ok(Opened {
        rich: reader.builder.finish(),
        lossy: reader.lossy,
    })
}

fn run(out: &mut String, text: &str, attributes: &TextAttributes, heading: bool) {
    out.push_str("<w:r><w:rPr>");
    if let Some(family) = attributes.family.as_deref() {
        let _ = write!(
            out,
            "<w:rFonts w:ascii=\"{0}\" w:hAnsi=\"{0}\"/>",
            xml(family)
        );
    }
    if attributes.bold && !heading {
        out.push_str("<w:b/>");
    }
    if attributes.italic {
        out.push_str("<w:i/>");
    }
    if attributes.strikethrough {
        out.push_str("<w:strike/>");
    }
    if let Some(color) = attributes.color {
        let _ = write!(
            out,
            "<w:color w:val=\"{:02X}{:02X}{:02X}\"/>",
            color.r, color.g, color.b
        );
    }
    if let Some(size) = attributes.size.filter(|_| !heading) {
        let _ = write!(
            out,
            "<w:sz w:val=\"{}\"/>",
            (size / BASE_SIZE * BASE_HALF_POINTS).round() as u32
        );
    }
    if attributes.underline {
        out.push_str("<w:u w:val=\"single\"/>");
    }
    out.push_str("</w:rPr>");
    for (index, piece) in text.split('\t').enumerate() {
        if index > 0 {
            out.push_str("<w:tab/>");
        }
        if !piece.is_empty() {
            let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", xml(piece));
        }
    }
    out.push_str("</w:r>");
}

pub fn write(rich: &RichText) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    let mut links: Vec<String> = Vec::new();
    for paragraph in paragraphs(rich) {
        let attributes = paragraph.attributes;
        let level = heading_level(&paragraph.first()).filter(|_| attributes.list.is_none());
        body.push_str("<w:p><w:pPr>");
        if let Some(level) = level {
            let _ = write!(body, "<w:pStyle w:val=\"Heading{level}\"/>");
        }
        if let Some(list) = attributes.list {
            let id = match list {
                ListMarker::Bullet => 1,
                ListMarker::Number => 2,
            };
            let _ = write!(
                body,
                "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{id}\"/></w:numPr>",
                attributes.level.min(8)
            );
        }
        let justification = match attributes.alignment {
            Alignment::Start => None,
            Alignment::Center => Some("center"),
            Alignment::End => Some("right"),
            Alignment::Justify => Some("both"),
        };
        if let Some(justification) = justification {
            let _ = write!(body, "<w:jc w:val=\"{justification}\"/>");
        }
        body.push_str("</w:pPr>");
        for (text, span) in &paragraph.spans {
            if text.is_empty() {
                continue;
            }
            match &span.link {
                Some(url) => {
                    links.push(url.to_string());
                    let _ = write!(body, "<w:hyperlink r:id=\"link{}\">", links.len());
                    run(&mut body, text, span, level.is_some());
                    body.push_str("</w:hyperlink>");
                }
                None => run(&mut body, text, span, level.is_some()),
            }
        }
        body.push_str("</w:p>");
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{MAIN}\" xmlns:r=\"{RELATIONSHIPS}\"><w:body>{body}<w:sectPr/></w:body></w:document>"
    );
    let mut relationships = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"styles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
         <Relationship Id=\"numbering\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>",
    );
    for (index, url) in links.iter().enumerate() {
        let _ = write!(
            relationships,
            "<Relationship Id=\"link{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            index + 1,
            xml(url)
        );
    }
    relationships.push_str("</Relationships>");
    let mut styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{MAIN}\"><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault></w:docDefaults>"
    );
    for (level, size) in super::HEADINGS.iter().enumerate() {
        let _ = write!(
            styles,
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{0}\"><w:name w:val=\"heading {0}\"/><w:pPr><w:outlineLvl w:val=\"{1}\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"{2}\"/></w:rPr></w:style>",
            level + 1,
            level,
            (size / BASE_SIZE * BASE_HALF_POINTS).round() as u32
        );
    }
    styles.push_str("</w:styles>");
    let level = |index: usize, format: &str, text: &str| {
        format!(
            "<w:lvl w:ilvl=\"{index}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{text}\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>",
            720 * (index + 1)
        )
    };
    let bullets: String = (0..9).map(|index| level(index, "bullet", "•")).collect();
    let numbers: String = (0..9)
        .map(|index| level(index, "decimal", &format!("%{}.", index + 1)))
        .collect();
    let numbering = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{MAIN}\">\
         <w:abstractNum w:abstractNumId=\"1\">{bullets}</w:abstractNum><w:abstractNum w:abstractNumId=\"2\">{numbers}</w:abstractNum>\
         <w:num w:numId=\"1\"><w:abstractNumId w:val=\"1\"/></w:num><w:num w:numId=\"2\"><w:abstractNumId w:val=\"2\"/></w:num></w:numbering>"
    );
    let parts = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
             <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
             <Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/></Types>"
                .to_owned(),
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"document\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>"
                .to_owned(),
        ),
        ("word/document.xml", document),
        ("word/_rels/document.xml.rels", relationships),
        ("word/styles.xml", styles),
        ("word/numbering.xml", numbering),
    ];
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in parts {
        zip.start_file(name, SimpleFileOptions::default())
            .map_err(|error| error.to_string())?;
        zip.write_all(content.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    Ok(zip
        .finish()
        .map_err(|error| error.to_string())?
        .into_inner())
}
