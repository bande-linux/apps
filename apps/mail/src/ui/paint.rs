use bande_toolkit::ui::{Scroll, Size};
use bande_toolkit::{
    Item, Paragraph, Point, Radius, Rect, Scene, Shape, TextAlign, TextRole, TextStyle,
};
use jiff::Timestamp;

use crate::L10N;
use crate::store::Summary;
use crate::ui::format::list_date;
use crate::ui::reader::lay_out;
use crate::ui::sidebar::Row;
use crate::ui::symbols::Symbol;
use crate::ui::view::{Hit, LIST_WIDTH, MailView, Pane, SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
use crate::worker::Status;

const SIDEBAR_INSET: f32 = 8.0;
const SIDEBAR_ROW: f32 = 28.0;
const SIDEBAR_HEADING: f32 = 30.0;
const SIDEBAR_TEXT_X: f32 = 18.0;
const SIDEBAR_INDENT: f32 = 14.0;
const ICON_SIZE: f32 = 16.0;
const ICON_PIXELS: u32 = 32;
const ICON_GAP: f32 = 8.0;
const LIST_PADDING: f32 = 10.0;
const LIST_TEXT_X: f32 = 26.0;
const LIST_TRAILING: f32 = 14.0;
const DOT: f32 = 8.0;
const ROW_INSET: f32 = 6.0;
const PREVIEW_LINES: f32 = 2.0;
const BADGE_GAP: f32 = 10.0;
const SMALL_SYMBOL: f32 = 13.0;
const PAGE_OVERLAP: f32 = 40.0;

impl MailView {
    fn scale(&mut self) -> f32 {
        self.ui.context().text.scale()
    }

    pub(crate) fn sheet_frame(&self) -> Rect {
        Rect::new(
            SIDEBAR_WIDTH,
            TOOLBAR_HEIGHT,
            (self.size.0 - SIDEBAR_WIDTH - self.sheet_inset).max(0.0),
            (self.size.1 - TOOLBAR_HEIGHT - self.sheet_inset).max(0.0),
        )
    }

    pub(crate) fn sidebar_frame(&self) -> Rect {
        Rect::new(
            0.0,
            TOOLBAR_HEIGHT,
            SIDEBAR_WIDTH,
            (self.size.1 - TOOLBAR_HEIGHT).max(0.0),
        )
    }

    pub(crate) fn list_frame(&self) -> Rect {
        let sheet = self.sheet_frame();
        Rect::new(sheet.x, sheet.y, LIST_WIDTH.min(sheet.width), sheet.height)
    }

    pub(crate) fn reader_frame(&self) -> Rect {
        let sheet = self.sheet_frame();
        let list = self.list_frame();
        Rect::new(
            list.x + list.width + 1.0,
            sheet.y,
            (sheet.width - list.width - 1.0).max(0.0),
            sheet.height,
        )
    }

    pub(crate) fn row_height(&mut self) -> f32 {
        let scale = self.scale();
        (2.0 * LIST_PADDING + (18.0 + 18.0 + 16.0 * PREVIEW_LINES) * scale).round()
    }

    fn sidebar_row_height(&mut self) -> f32 {
        (SIDEBAR_ROW * self.scale()).round()
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.hits.clear();
        self.layout_toolbar();
        self.ui.layout(Size::new(self.size.0, self.size.1));
        let (controls, sheet) = self.ui.paint_layers(width, height);
        let mut items = Vec::new();
        self.paint_sidebar(&mut items);
        let sheet_frame = self.sheet_frame();
        let radius = match self.sheet_inset > 0.0 {
            true => Radius::concentric(Radius::XL, self.sheet_inset),
            false => 0.0,
        };
        let mut content = Vec::new();
        self.paint_list(&mut content);
        self.paint_reader(&mut content);
        items.push(Item::Clip {
            shape: Shape::rounded(sheet_frame, radius),
            items: content,
        });
        items.extend(controls);
        items.extend(sheet);
        Scene {
            width,
            height,
            background: self.palette.sheet,
            items,
        }
    }

    fn text_line(&mut self, text: &str, style: TextStyle, width: f32) -> Paragraph {
        let paragraph = self
            .ui
            .context()
            .text
            .paragraph(text, style, None, TextAlign::Start);
        match paragraph.width() <= width {
            true => paragraph,
            false => {
                let mut shortened = text.chars().collect::<Vec<_>>();
                while !shortened.is_empty() {
                    shortened.pop();
                    let candidate = format!("{}…", shortened.iter().collect::<String>().trim_end());
                    let paragraph =
                        self.ui
                            .context()
                            .text
                            .paragraph(&candidate, style, None, TextAlign::Start);
                    if paragraph.width() <= width {
                        return paragraph;
                    }
                }
                paragraph
            }
        }
    }

    fn put_symbol(
        &mut self,
        items: &mut Vec<Item>,
        symbol: Symbol,
        rect: Rect,
        color: bande_toolkit::Color,
    ) {
        if let Some(image) = self.symbols.get(symbol, ICON_PIXELS, color) {
            items.push(Item::Image { rect, image });
        }
    }

    fn paint_sidebar(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let window_active = self.ui.context().window_active;
        let frame = self.sidebar_frame();
        let row_height = self.sidebar_row_height();
        let heading_height = (SIDEBAR_HEADING * self.scale()).round();
        let content = self
            .sidebar
            .iter()
            .map(|row| match row {
                Row::Heading(_) => heading_height,
                Row::Place { .. } => row_height,
            })
            .sum::<f32>()
            + SIDEBAR_INSET;
        let offset = {
            let scroll = &mut self.scrolls[0];
            scroll.set_extent(content, frame.height);
            scroll.offset(self.ui.context())
        };
        let mut drawn = Vec::new();
        let mut y = frame.y + SIDEBAR_INSET / 2.0 - offset;
        for (index, row) in self.sidebar.clone().iter().enumerate() {
            match row {
                Row::Heading(title) => {
                    let style = TextStyle::role(TextRole::Label, palette.ink3);
                    let paragraph =
                        self.text_line(title, style, SIDEBAR_WIDTH - 2.0 * SIDEBAR_TEXT_X);
                    let text_y = y + heading_height - paragraph.height() - 4.0;
                    drawn.push(Item::Text {
                        origin: Point::new(SIDEBAR_TEXT_X, text_y),
                        paragraph,
                    });
                    let offline = self
                        .accounts
                        .iter()
                        .find(|(account, _)| account.email == *title)
                        .is_some_and(|(account, _)| {
                            self.statuses.iter().any(|(known, status)| {
                                *known == account.id && *status == Status::Offline
                            })
                        });
                    if offline {
                        let paragraph = self.text_line(
                            &L10N.tr("account-offline"),
                            TextStyle::role(TextRole::Callout, palette.ink3),
                            80.0,
                        );
                        drawn.push(Item::Text {
                            origin: Point::new(
                                SIDEBAR_WIDTH - SIDEBAR_INSET - 8.0 - paragraph.width(),
                                y + heading_height - paragraph.height() - 4.0,
                            ),
                            paragraph,
                        });
                    }
                    y += heading_height;
                }
                Row::Place {
                    place,
                    title,
                    symbol,
                    unread,
                    depth,
                } => {
                    let rect = Rect::new(
                        SIDEBAR_INSET,
                        y,
                        SIDEBAR_WIDTH - 2.0 * SIDEBAR_INSET,
                        row_height,
                    );
                    let selected = *place == self.place;
                    if selected {
                        drawn.push(Item::Fill {
                            shape: Shape::rounded(rect, Radius::S),
                            paint: palette
                                .sidebar_selection(window_active, self.focus == Pane::Sidebar)
                                .into(),
                        });
                    } else if self.hover == Some(Hit::Place(index)) {
                        drawn.push(Item::Fill {
                            shape: Shape::rounded(rect, Radius::S),
                            paint: palette.well.with_opacity(0.5).into(),
                        });
                    }
                    let x = SIDEBAR_TEXT_X + *depth as f32 * SIDEBAR_INDENT;
                    let tint = match selected {
                        true => palette.accent,
                        false => palette.ink2,
                    };
                    let icon = Rect::new(
                        x,
                        y + ((row_height - ICON_SIZE) / 2.0).round(),
                        ICON_SIZE,
                        ICON_SIZE,
                    );
                    self.put_symbol(&mut drawn, *symbol, icon, tint);
                    let mut right = rect.x + rect.width - 8.0;
                    if *unread > 0 {
                        let badge = self.ui.context().text.paragraph(
                            &unread.to_string(),
                            TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
                            None,
                            TextAlign::End,
                        );
                        right -= badge.width();
                        drawn.push(Item::Text {
                            origin: Point::new(
                                right,
                                y + ((row_height - badge.height()) / 2.0).round(),
                            ),
                            paragraph: badge,
                        });
                        right -= BADGE_GAP;
                    }
                    let text_x = x + ICON_SIZE + ICON_GAP;
                    let paragraph = self.text_line(
                        title,
                        TextStyle::role(TextRole::Body, palette.ink),
                        right - text_x,
                    );
                    drawn.push(Item::Text {
                        origin: Point::new(
                            text_x,
                            y + ((row_height - paragraph.height()) / 2.0).round(),
                        ),
                        paragraph,
                    });
                    if rect.y + rect.height > frame.y && rect.y < frame.y + frame.height {
                        self.hits.push((rect, Hit::Place(index)));
                    }
                    y += row_height;
                }
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: drawn,
        });
        let scroll = &self.scrolls[0];
        let cx = self.ui.context();
        scroll.paint_scrollbar(cx, frame, items);
    }

    fn paint_list(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let frame = self.list_frame();
        let row_height = self.row_height();
        let offset = {
            let content = self.messages.len() as f32 * row_height;
            let scroll = &mut self.scrolls[1];
            scroll.set_extent(content, frame.height);
            scroll.offset(self.ui.context())
        };
        let mut drawn = Vec::new();
        if self.messages.is_empty() {
            let message = match self.search.is_empty() {
                true => L10N.tr("no-messages"),
                false => L10N.tr("no-results"),
            };
            let paragraph = self.ui.context().text.paragraph(
                &message,
                TextStyle::role(TextRole::Body, palette.ink3),
                Some(frame.width),
                TextAlign::Center,
            );
            drawn.push(Item::Text {
                origin: Point::new(
                    frame.x,
                    frame.y + (frame.height / 2.0 - paragraph.height()).round(),
                ),
                paragraph,
            });
        }
        let first = (offset / row_height).floor().max(0.0) as usize;
        let visible = (frame.height / row_height).ceil() as usize + 1;
        let now = self.now.unwrap_or_else(Timestamp::now);
        let shown = self
            .messages
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(index, summary)| (index, summary.clone()))
            .collect::<Vec<_>>();
        for (index, summary) in shown {
            let y = frame.y + index as f32 * row_height - offset;
            let rect = Rect::new(frame.x, y, frame.width, row_height);
            self.paint_row(&mut drawn, &summary, rect, now);
            self.hits.push((rect, Hit::Message(summary.id)));
        }
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: drawn,
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(frame.x + frame.width, frame.y, 1.0, frame.height)),
            paint: palette.hairline.into(),
        });
        let scroll = &self.scrolls[1];
        let cx = self.ui.context();
        scroll.paint_scrollbar(cx, frame, items);
    }

    fn paint_row(&mut self, items: &mut Vec<Item>, summary: &Summary, rect: Rect, now: Timestamp) {
        let palette = self.palette;
        let selected = self.selected == Some(summary.id);
        let active = selected
            && self.focus == Pane::List
            && !self.ui.has_sheet()
            && self.ui.context().window_active;
        let highlight = Rect::new(
            rect.x + ROW_INSET,
            rect.y + 1.0,
            rect.width - 2.0 * ROW_INSET,
            rect.height - 2.0,
        );
        let (ink, ink2, ink3) = match (selected, active) {
            (true, true) => (
                palette.on_accent,
                palette.on_accent.with_opacity(0.85),
                palette.on_accent.with_opacity(0.75),
            ),
            _ => (palette.ink, palette.ink2, palette.ink3),
        };
        if selected {
            items.push(Item::Fill {
                shape: Shape::rounded(highlight, Radius::M),
                paint: match active {
                    true => palette.accent.into(),
                    false => palette.well.into(),
                },
            });
        } else {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    rect.x + LIST_TEXT_X,
                    rect.y + rect.height - 1.0,
                    rect.width - LIST_TEXT_X - LIST_TRAILING,
                    1.0,
                )),
                paint: palette.hairline.into(),
            });
        }
        let scale = self.scale();
        let text_x = rect.x + LIST_TEXT_X;
        let right = rect.x + rect.width - LIST_TRAILING;
        let mut y = rect.y + LIST_PADDING;
        let date = self.ui.context().text.paragraph(
            &list_date(summary.date, now, &self.zone),
            TextStyle::role(TextRole::Callout, ink3),
            None,
            TextAlign::End,
        );
        let date_x = right - date.width();
        let line = (18.0 * scale).round();
        items.push(Item::Text {
            origin: Point::new(date_x, y + ((line - date.height()) / 2.0).round()),
            paragraph: date,
        });
        if !summary.flags.seen {
            let dot = Rect::new(
                rect.x + (LIST_TEXT_X - DOT) / 2.0,
                y + ((line - DOT) / 2.0).round(),
                DOT,
                DOT,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(dot, DOT / 2.0),
                paint: match active {
                    true => palette.on_accent.into(),
                    false => palette.accent.into(),
                },
            });
        }
        let sender = match summary.sender.shown() {
            "" => L10N.tr("no-sender"),
            shown => shown.to_owned(),
        };
        let sender = self.text_line(
            &sender,
            TextStyle::role(TextRole::BodyStrong, ink),
            date_x - text_x - 8.0,
        );
        items.push(Item::Text {
            origin: Point::new(text_x, y),
            paragraph: sender,
        });
        y += line;
        let mut subject_right = right;
        let symbols = [
            (
                summary.flags.flagged,
                Symbol::Flag,
                match active {
                    true => ink,
                    false => palette.warning,
                },
            ),
            (summary.attachments, Symbol::Attachment, ink3),
        ];
        for (shown, symbol, color) in symbols {
            if shown {
                subject_right -= SMALL_SYMBOL;
                let icon = Rect::new(
                    subject_right,
                    y + ((line - SMALL_SYMBOL) / 2.0).round(),
                    SMALL_SYMBOL,
                    SMALL_SYMBOL,
                );
                self.put_symbol(items, symbol, icon, color);
                subject_right -= 6.0;
            }
        }
        let subject = match summary.subject.as_str() {
            "" => L10N.tr("no-subject"),
            subject => subject.to_owned(),
        };
        let subject = self.text_line(
            &subject,
            TextStyle::role(TextRole::Body, ink),
            subject_right - text_x,
        );
        items.push(Item::Text {
            origin: Point::new(text_x, y),
            paragraph: subject,
        });
        y += line;
        let preview_height = (16.0 * PREVIEW_LINES * scale).round();
        let preview = self.ui.context().text.paragraph(
            &summary.preview,
            TextStyle::role(TextRole::Callout, ink2),
            Some(right - text_x),
            TextAlign::Start,
        );
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(text_x, y, right - text_x, preview_height)),
            items: vec![Item::Text {
                origin: Point::new(text_x, y),
                paragraph: preview,
            }],
        });
    }

    fn paint_reader(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let frame = self.reader_frame();
        if frame.width <= 0.0 {
            return;
        }
        let Some(reading) = &self.reading else {
            let message = match self.messages.len() {
                0 => String::new(),
                _ => L10N.tr("no-message-selected"),
            };
            let paragraph = self.ui.context().text.paragraph(
                &message,
                TextStyle::role(TextRole::Heading, palette.ink3),
                Some(frame.width),
                TextAlign::Center,
            );
            items.push(Item::Text {
                origin: Point::new(
                    frame.x,
                    frame.y + (frame.height / 2.0 - paragraph.height()).round(),
                ),
                paragraph,
            });
            return;
        };
        if self
            .layout
            .as_ref()
            .is_none_or(|(width, _)| *width != frame.width)
        {
            let zone = self.zone.clone();
            let layout = lay_out(
                reading,
                frame.width,
                &mut self.ui.context().text,
                &palette,
                &mut self.symbols,
                &zone,
            );
            self.layout = Some((frame.width, layout));
        }
        let Some((_, layout)) = &self.layout else {
            return;
        };
        let height = layout.height;
        let offset = {
            let scroll = &mut self.scrolls[2];
            scroll.set_extent(height, frame.height);
            scroll.offset(self.ui.context())
        };
        let origin = Point::new(frame.x, frame.y - offset);
        let Some((_, layout)) = &self.layout else {
            return;
        };
        let moved = |rect: &Rect| {
            Rect::new(
                rect.x + origin.x,
                rect.y + origin.y,
                rect.width,
                rect.height,
            )
        };
        let links = layout
            .links
            .iter()
            .map(|(rect, url)| (moved(rect), Hit::Link(url.clone())))
            .chain(
                layout
                    .attachments
                    .iter()
                    .map(|(rect, index)| (moved(rect), Hit::Attachment(*index))),
            )
            .chain(
                layout
                    .load_remote
                    .iter()
                    .map(|rect| (moved(rect), Hit::LoadRemote)),
            )
            .filter(|(rect, _)| rect.y + rect.height > frame.y && rect.y < frame.y + frame.height)
            .collect::<Vec<_>>();
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Translate {
                offset: origin,
                items: layout.items.clone(),
            }],
        });
        self.hits.extend(links);
        let scroll = &self.scrolls[2];
        let cx = self.ui.context();
        scroll.paint_scrollbar(cx, frame, items);
    }

    pub(crate) fn scroll(&mut self, scroll: &Scroll) -> bool {
        let at = scroll.position;
        let index = if self.sidebar_frame().contains(at) {
            0
        } else if self.list_frame().contains(at) {
            1
        } else if self.reader_frame().contains(at) {
            2
        } else {
            return false;
        };
        let state = &mut self.scrolls[index];
        let cx = self.ui.context();
        state.scroll(cx, scroll)
    }

    pub(crate) fn page_reader(&mut self) -> bool {
        let frame = self.reader_frame();
        let state = &mut self.scrolls[2];
        let cx = self.ui.context();
        let target = state.offset(cx) + frame.height - PAGE_OVERLAP;
        state.scroll_to(cx, target)
    }

    pub(crate) fn reveal_selected(&mut self) {
        let Some(index) = self
            .selected
            .and_then(|id| self.messages.iter().position(|summary| summary.id == id))
        else {
            return;
        };
        let row_height = self.row_height();
        let top = index as f32 * row_height;
        let state = &mut self.scrolls[1];
        let cx = self.ui.context();
        state.reveal(cx, top, top + row_height);
    }
}
