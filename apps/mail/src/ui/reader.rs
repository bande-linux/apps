use std::collections::HashMap;

use bande_toolkit::{
    Image, Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle, TextSystem,
};
use jiff::tz::TimeZone;

use crate::L10N;
use crate::html::{BlockKind, Document};
use crate::message::{Address, Content};
use crate::picture::{Picture, decode};
use crate::settings::MailSettings;
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
const MAX_REMOTE: usize = 20;
const MAX_INLINE_PIXELS: u64 = 24_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    Blocked,
    Loading,
    Loaded,
}

pub struct Reading {
    pub content: Content,
    pub document: Document,
    pub downloaded: bool,
    pub pictures: HashMap<String, Image>,
    pub remote: Remote,
}

fn image_of(picture: Picture) -> Option<Image> {
    Image::from_rgba(picture.width, picture.height, picture.rgba)
}

impl Reading {
    pub fn new(content: Content, downloaded: bool, settings: &MailSettings) -> Self {
        let plain = content.plain.is_some() && (settings.plain_text || content.html.is_none());
        let mut document = content.document(settings.plain_text);
        if plain && settings.fixed_width {
            for block in &mut document.blocks {
                if block.kind == BlockKind::Paragraph {
                    block.kind = BlockKind::Preformatted;
                }
            }
        }
        let mut pictures = HashMap::new();
        let mut pixels = 0u64;
        for source in document
            .blocks
            .iter()
            .filter_map(|block| block.image.as_deref())
        {
            if pictures.len() >= MAX_REMOTE || pixels >= MAX_INLINE_PIXELS {
                break;
            }
            if pictures.contains_key(source) {
                continue;
            }
            let Some(picture) = content
                .inline_part(source)
                .and_then(|part| decode(&part.data))
            else {
                continue;
            };
            pixels += u64::from(picture.width) * u64::from(picture.height);
            if let Some(image) = image_of(picture) {
                pictures.insert(source.to_owned(), image);
            }
        }
        Self {
            content,
            document,
            downloaded,
            pictures,
            remote: Remote::Blocked,
        }
    }

    pub fn remote_sources(&self) -> Vec<String> {
        let mut sources = Vec::new();
        for source in self
            .document
            .blocks
            .iter()
            .filter_map(|block| block.image.as_deref())
        {
            let remote = source
                .get(..8)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
            if remote && !sources.iter().any(|known| known == source) && sources.len() < MAX_REMOTE
            {
                sources.push(source.to_owned());
            }
        }
        sources
    }

    pub fn add_picture(&mut self, source: String, picture: Picture) {
        if let Some(image) = image_of(picture) {
            self.pictures.insert(source, image);
        }
    }

    fn shown_inline(&self, index: usize) -> bool {
        self.content
            .inline
            .iter()
            .any(|(id, part)| *part == index && self.pictures.contains_key(&format!("cid:{id}")))
    }
}

#[derive(Default)]
pub struct Layout {
    pub items: Vec<Item>,
    pub height: f32,
    pub links: Vec<(Rect, String)>,
    pub attachments: Vec<(Rect, usize)>,
    pub load_remote: Option<Rect>,
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

fn banner(writer: &mut Writer<'_>, symbols: &mut Symbols, message: &str, action: Option<&str>) {
    let palette = *writer.palette;
    let frame = Rect::new(MARGIN, writer.y, writer.width - 2.0 * MARGIN, BANNER_HEIGHT);
    writer.layout.items.push(Item::Fill {
        shape: Shape::rounded(frame, bande_toolkit::Radius::S),
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
    let mut right = frame.x + frame.width - CHIP_PADDING;
    if let Some(action) = action {
        let button = writer.text.paragraph(
            action,
            TextStyle::role(TextRole::Callout, palette.accent_text).weight(600.0),
            None,
            TextAlign::Start,
        );
        let x = right - button.width();
        let target = Rect::new(
            x - CHIP_PADDING,
            frame.y,
            button.width() + 2.0 * CHIP_PADDING,
            BANNER_HEIGHT,
        );
        writer.layout.items.push(Item::Text {
            origin: Point::new(
                x,
                frame.y + ((BANNER_HEIGHT - button.height()) / 2.0).round(),
            ),
            paragraph: button,
        });
        writer.layout.load_remote = Some(target);
        right = x - 2.0 * CHIP_PADDING;
    }
    let text_x = frame.x + 2.0 * CHIP_PADDING + SYMBOL_SIZE;
    let paragraph = writer.paragraph(
        message,
        TextStyle::role(TextRole::Callout, palette.ink2),
        right - text_x,
    );
    let y = writer.y;
    writer.y = frame.y + ((BANNER_HEIGHT - paragraph.height()) / 2.0).round();
    writer.put(paragraph, text_x);
    writer.y = y + BANNER_HEIGHT + SECTION_GAP;
}

fn picture(writer: &mut Writer<'_>, image: &Image, x: f32, width: f32) {
    let natural = (image.width() as f32, image.height() as f32);
    let shown_width = natural.0.min(width).max(1.0);
    let shown_height = (natural.1 * shown_width / natural.0.max(1.0)).round();
    writer.layout.items.push(Item::Image {
        rect: Rect::new(x, writer.y, shown_width.round(), shown_height),
        image: image.clone(),
    });
    writer.y += shown_height + BLOCK_GAP;
}

fn body(writer: &mut Writer<'_>, document: &Document, pictures: &HashMap<String, Image>) {
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
        if block.kind == BlockKind::Image {
            if let Some(image) = block.image.as_ref().and_then(|source| pictures.get(source)) {
                picture(writer, image, x, width);
            }
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

fn attachments(writer: &mut Writer<'_>, symbols: &mut Symbols, reading: &Reading) {
    let content = &reading.content;
    if (0..content.attachments.len()).all(|index| reading.shown_inline(index)) {
        return;
    }
    let palette = *writer.palette;
    writer.y += SECTION_GAP / 2.0;
    writer.rule();
    writer.y += SECTION_GAP;
    let mut x = MARGIN;
    for (index, attachment) in content.attachments.iter().enumerate() {
        if reading.shown_inline(index) {
            continue;
        }
        let label = format!(
            "{}  {}",
            attachment.name,
            bande_fs::format_size(attachment.data.len() as u64)
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
            shape: Shape::rounded(chip, bande_toolkit::Radius::S),
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
        banner(&mut writer, symbols, &L10N.tr("downloading-message"), None);
    } else if reading.document.remote_images > 0 {
        match reading.remote {
            Remote::Blocked => banner(
                &mut writer,
                symbols,
                &L10N.tr("remote-content-blocked"),
                Some(&L10N.tr("load-remote-content")),
            ),
            Remote::Loading => banner(
                &mut writer,
                symbols,
                &L10N.tr("remote-content-loading"),
                None,
            ),
            Remote::Loaded => {}
        }
    }
    body(&mut writer, &reading.document, &reading.pictures);
    attachments(&mut writer, symbols, reading);
    writer.layout.height = writer.y + MARGIN;
    writer.layout
}
