use jiff::tz::TimeZone;
use unbound_toolkit::{
    Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle, TextSystem,
};

use crate::L10N;
use crate::html::{BlockKind, Document};
use crate::message::{Address, Content};
use crate::ui::format::{full_date, initials};
use crate::ui::symbols::{Symbol, Symbols};

const MARGIN: f32 = 28.0;
const AVATAR: f32 = 40.0;
const AVATAR_GAP: f32 = 12.0;
const LINE_GAP: f32 = 3.0;
const SECTION_GAP: f32 = 16.0;
const BLOCK_GAP: f32 = 10.0;
const QUOTE_INDENT: f32 = 14.0;
const QUOTE_BAR: f32 = 2.0;
const ITEM_INDENT: f32 = 12.0;
const BANNER_HEIGHT: f32 = 34.0;
const CHIP_HEIGHT: f32 = 30.0;
const CHIP_GAP: f32 = 8.0;
const CHIP_PADDING: f32 = 10.0;
const CHIP_MAX: f32 = 260.0;
const SYMBOL_SIZE: f32 = 16.0;
const SYMBOL_PIXELS: u32 = 32;
const BOLD: f32 = 700.0;

pub struct Reading {
    pub content: Content,
    pub document: Document,
    pub downloaded: bool,
}

impl Reading {
    pub fn new(content: Content, downloaded: bool) -> Self {
        let document = content.document(false);
        Self {
            content,
            document,
            downloaded,
        }
    }
}

#[derive(Default)]
pub struct Layout {
    pub items: Vec<Item>,
    pub height: f32,
    pub links: Vec<(Rect, String)>,
    pub attachments: Vec<(Rect, usize)>,
}

fn names(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(|address| address.shown().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

struct Writer<'a> {
    text: &'a mut TextSystem,
    palette: &'a Palette,
    layout: Layout,
    y: f32,
    width: f32,
}

impl Writer<'_> {
    fn paragraph(&mut self, text: &str, style: TextStyle, width: f32) -> Paragraph {
        self.text
            .paragraph(text, style, Some(width.max(1.0)), TextAlign::Start)
    }

    fn put(&mut self, paragraph: Paragraph, x: f32) -> f32 {
        let height = paragraph.height();
        self.layout.items.push(Item::Text {
            origin: Point::new(x, self.y),
            paragraph,
        });
        height
    }

    fn rule(&mut self) {
        self.layout.items.push(Item::Fill {
            shape: Shape::rect(Rect::new(MARGIN, self.y, self.width - 2.0 * MARGIN, 1.0)),
            paint: self.palette.hairline.into(),
        });
    }
}

fn header(writer: &mut Writer<'_>, reading: &Reading, zone: &TimeZone) {
    let palette = *writer.palette;
    let envelope = &reading.content.envelope;
    let sender = envelope.sender();
    let shown = match sender.shown() {
        "" => L10N.tr("no-sender"),
        shown => shown.to_owned(),
    };
    let avatar = Rect::new(MARGIN, MARGIN, AVATAR, AVATAR);
    writer.layout.items.push(Item::Fill {
        shape: Shape::rounded(avatar, AVATAR / 2.0),
        paint: palette.well.into(),
    });
    let letters = writer.text.paragraph(
        &initials(&shown),
        TextStyle::role(TextRole::BodyStrong, palette.ink2),
        Some(AVATAR),
        TextAlign::Center,
    );
    writer.layout.items.push(Item::Text {
        origin: Point::new(
            avatar.x,
            avatar.y + ((AVATAR - letters.height()) / 2.0).round(),
        ),
        paragraph: letters,
    });
    let text_x = MARGIN + AVATAR + AVATAR_GAP;
    let date = envelope
        .date
        .map(|date| full_date(date, zone))
        .unwrap_or_default();
    let date = writer.text.paragraph(
        &date,
        TextStyle::role(TextRole::Callout, palette.ink3),
        None,
        TextAlign::Start,
    );
    let date_x = writer.width - MARGIN - date.width();
    writer.y = MARGIN;
    writer.put(date, date_x);
    let name_width = (date_x - text_x - AVATAR_GAP).max(40.0);
    let name = writer.paragraph(
        &shown,
        TextStyle::role(TextRole::BodyStrong, palette.ink),
        name_width,
    );
    writer.y += writer.put(name, text_x) + LINE_GAP;
    let details_width = writer.width - text_x - MARGIN;
    for (label, addresses) in [("header-to", &envelope.to), ("header-cc", &envelope.cc)] {
        if addresses.is_empty() {
            continue;
        }
        let line = L10N.tr_args(label, [("names", names(addresses).into())]);
        let paragraph = writer.paragraph(
            &line,
            TextStyle::role(TextRole::Callout, palette.ink2),
            details_width,
        );
        writer.y += writer.put(paragraph, text_x) + LINE_GAP;
    }
    writer.y = writer.y.max(MARGIN + AVATAR) + SECTION_GAP / 2.0;
    let subject = match envelope.subject.as_str() {
        "" => L10N.tr("no-subject"),
        subject => subject.to_owned(),
    };
    let subject = writer.paragraph(
        &subject,
        TextStyle::role(TextRole::Heading, palette.ink),
        writer.width - 2.0 * MARGIN,
    );
    writer.y += writer.put(subject, MARGIN) + SECTION_GAP;
    writer.rule();
    writer.y += SECTION_GAP;
}

fn banner(writer: &mut Writer<'_>, symbols: &mut Symbols, message: &str) {
    let palette = *writer.palette;
    let frame = Rect::new(MARGIN, writer.y, writer.width - 2.0 * MARGIN, BANNER_HEIGHT);
    writer.layout.items.push(Item::Fill {
        shape: Shape::rounded(frame, unbound_toolkit::Radius::S),
        paint: palette.well.into(),
    });
    if let Some(image) = symbols.get(Symbol::Remote, SYMBOL_PIXELS, palette.ink2) {
        writer.layout.items.push(Item::Image {
            rect: Rect::new(
                frame.x + CHIP_PADDING,
                frame.y + ((BANNER_HEIGHT - SYMBOL_SIZE) / 2.0).round(),
                SYMBOL_SIZE,
                SYMBOL_SIZE,
            ),
            image,
        });
    }
    let text_x = frame.x + 2.0 * CHIP_PADDING + SYMBOL_SIZE;
    let paragraph = writer.paragraph(
        message,
        TextStyle::role(TextRole::Callout, palette.ink2),
        frame.x + frame.width - text_x - CHIP_PADDING,
    );
    let y = writer.y;
    writer.y = frame.y + ((BANNER_HEIGHT - paragraph.height()) / 2.0).round();
    writer.put(paragraph, text_x);
    writer.y = y + BANNER_HEIGHT + SECTION_GAP;
}

fn body(writer: &mut Writer<'_>, document: &Document) {
    let palette = *writer.palette;
    for block in &document.blocks {
        let quote = f32::from(block.quote) * QUOTE_INDENT;
        let indent = quote
            + match block.kind {
                BlockKind::Item => ITEM_INDENT,
                _ => 0.0,
            };
        let x = MARGIN + indent;
        let width = writer.width - 2.0 * MARGIN - indent;
        if block.kind == BlockKind::Rule {
            writer.rule();
            writer.y += BLOCK_GAP;
            continue;
        }
        let color = match block.quote {
            0 => palette.ink,
            _ => palette.ink2,
        };
        let style = match block.kind {
            BlockKind::Heading => TextStyle::role(TextRole::BodyStrong, color),
            BlockKind::Preformatted => TextStyle::role(TextRole::Mono, color),
            _ => TextStyle::role(TextRole::Body, color),
        };
        let emphasis = block
            .bold
            .iter()
            .map(|range| (range.clone(), BOLD))
            .collect::<Vec<_>>();
        let paragraph = writer.text.emphasized(
            &block.text,
            style,
            &emphasis,
            Some(width.max(1.0)),
            TextAlign::Start,
        );
        for (range, url) in &block.links {
            for rect in paragraph.span_rects(range.clone()) {
                let rect = Rect::new(rect.x + x, rect.y + writer.y, rect.width, rect.height);
                writer.layout.items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        rect.x,
                        rect.y + rect.height - 2.0,
                        rect.width,
                        1.0,
                    )),
                    paint: palette.accent_text.into(),
                });
                writer.layout.links.push((rect, url.clone()));
            }
        }
        let top = writer.y;
        let height = writer.put(paragraph, x);
        for level in 0..block.quote {
            writer.layout.items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        MARGIN + f32::from(level) * QUOTE_INDENT,
                        top,
                        QUOTE_BAR,
                        height,
                    ),
                    QUOTE_BAR / 2.0,
                ),
                paint: palette.accent.with_opacity(0.45).into(),
            });
        }
        writer.y += height + BLOCK_GAP;
    }
}

fn attachments(writer: &mut Writer<'_>, symbols: &mut Symbols, content: &Content) {
    if content.attachments.is_empty() {
        return;
    }
    let palette = *writer.palette;
    writer.y += SECTION_GAP / 2.0;
    writer.rule();
    writer.y += SECTION_GAP;
    let mut x = MARGIN;
    for (index, attachment) in content.attachments.iter().enumerate() {
        let label = format!(
            "{}  {}",
            attachment.name,
            unbound_fs::format_size(attachment.data.len() as u64)
        );
        let paragraph = writer.text.paragraph(
            &label,
            TextStyle::role(TextRole::Callout, palette.ink),
            Some(CHIP_MAX - 3.0 * CHIP_PADDING - SYMBOL_SIZE),
            TextAlign::Start,
        );
        let width = (paragraph.width() + 3.0 * CHIP_PADDING + SYMBOL_SIZE).min(CHIP_MAX);
        if x + width > writer.width - MARGIN && x > MARGIN {
            x = MARGIN;
            writer.y += CHIP_HEIGHT + CHIP_GAP;
        }
        let chip = Rect::new(x, writer.y, width, CHIP_HEIGHT);
        writer.layout.items.push(Item::Fill {
            shape: Shape::rounded(chip, unbound_toolkit::Radius::S),
            paint: palette.well.into(),
        });
        if let Some(image) = symbols.get(Symbol::Attachment, SYMBOL_PIXELS, palette.ink2) {
            writer.layout.items.push(Item::Image {
                rect: Rect::new(
                    chip.x + CHIP_PADDING,
                    chip.y + ((CHIP_HEIGHT - SYMBOL_SIZE) / 2.0).round(),
                    SYMBOL_SIZE,
                    SYMBOL_SIZE,
                ),
                image,
            });
        }
        let text_y = chip.y + ((CHIP_HEIGHT - paragraph.height()) / 2.0).round();
        writer.layout.items.push(Item::Clip {
            shape: Shape::rect(chip),
            items: vec![Item::Text {
                origin: Point::new(chip.x + 2.0 * CHIP_PADDING + SYMBOL_SIZE, text_y),
                paragraph,
            }],
        });
        writer.layout.attachments.push((chip, index));
        x += width + CHIP_GAP;
    }
    writer.y += CHIP_HEIGHT;
}

pub fn lay_out(
    reading: &Reading,
    width: f32,
    text: &mut TextSystem,
    palette: &Palette,
    symbols: &mut Symbols,
    zone: &TimeZone,
) -> Layout {
    let mut writer = Writer {
        text,
        palette,
        layout: Layout::default(),
        y: 0.0,
        width,
    };
    header(&mut writer, reading, zone);
    if !reading.downloaded {
        banner(&mut writer, symbols, &L10N.tr("downloading-message"));
    } else if reading.document.remote_images > 0 {
        banner(&mut writer, symbols, &L10N.tr("remote-content-blocked"));
    }
    body(&mut writer, &reading.document);
    attachments(&mut writer, symbols, &reading.content);
    writer.layout.height = writer.y + MARGIN;
    writer.layout
}
