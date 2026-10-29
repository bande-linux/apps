use std::collections::HashMap;

use alacritty_terminal::index::Side;
use alacritty_terminal::term::{self, cell::Flags};
use alacritty_terminal::vte::ansi::CursorShape;
use unbound_toolkit::{
    Appearance, Color, Item, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::tabs::{TAB_BAR_HEIGHT, TabBarLayout, TabTarget};
use crate::terminal::{CellPoint, GridSize, Highlights, Terminal};
use crate::theme::{Theme, dimmed};

pub const DEFAULT_FONT_SIZE: f32 = 11.0;
pub const PADDING: f32 = 4.0;
const PARAGRAPH_CACHE: usize = 8192;
const BOLD: f32 = 700.0;
const MEASURE_SAMPLE: &str = "MMMMMMMMMM";
const LABEL_SIZE: f32 = 12.0;
const SYMBOL_SIZE: f32 = 16.0;
const LABEL_CACHE: usize = 256;
const TAB_LABEL_INSET: f32 = 28.0;
const ELLIPSIS: char = '\u{2026}';
const CLOSE_SYMBOL: &str = "\u{00D7}";
const NEW_TAB_SYMBOL: &str = "+";
const HOVER_ALPHA: u8 = 0x14;

fn snap(value: f32, scale: f64) -> f32 {
    let scale = scale as f32;
    (value * scale).round() / scale
}

fn hairline(points: f32, scale: f64) -> f32 {
    let scale = scale as f32;
    (points * scale).round().max(1.0) / scale
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub cell_width: f32,
    pub cell_height: f32,
    pub scale: f64,
}

impl Metrics {
    fn measure(text: &mut TextSystem, font_size: f32, scale: f64) -> Self {
        let style = TextStyle::new(font_size, Color::hex(0)).monospace();
        let sample = text.paragraph(MEASURE_SAMPLE, style, None, TextAlign::Start);
        let scaled = scale as f32;
        Self {
            cell_width: sample.width() / MEASURE_SAMPLE.len() as f32,
            cell_height: (sample.height() * scaled).ceil() / scaled,
            scale,
        }
    }

    pub fn content_size(&self, size: GridSize) -> (f32, f32) {
        (
            size.columns as f32 * self.cell_width + 2.0 * PADDING,
            size.lines as f32 * self.cell_height + 2.0 * PADDING,
        )
    }

    pub fn grid_for(&self, width: f32, height: f32) -> GridSize {
        let fit =
            |length: f32, cell: f32| (((length - 2.0 * PADDING) / cell).floor() as usize).max(1);
        GridSize {
            columns: fit(width, self.cell_width).max(2),
            lines: fit(height, self.cell_height),
        }
    }

    pub fn cell_at(&self, point: Point) -> CellPoint {
        let x = ((point.x - PADDING) / self.cell_width).max(0.0);
        let y = ((point.y - PADDING) / self.cell_height).max(0.0);
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        CellPoint {
            column: x as usize,
            line: y as usize,
            side,
        }
    }

    pub fn pixel_cell_size(&self) -> (u16, u16) {
        let pixels = |points: f32| (f64::from(points) * self.scale).round() as u16;
        (pixels(self.cell_width), pixels(self.cell_height))
    }
}

type RunKey = (String, Color, bool);
type LabelKey = (String, Color, u32, u32);

pub struct TabBar<'a> {
    pub titles: &'a [String],
    pub active: usize,
    pub hover: Option<TabTarget>,
    pub focused: bool,
}

struct Run {
    line: usize,
    column: usize,
    text: String,
    color: Color,
    bold: bool,
}

struct Span {
    line: usize,
    start: usize,
    end: usize,
    color: Color,
}

fn push_span(spans: &mut Vec<Span>, line: usize, column: usize, cells: usize, color: Color) {
    match spans.last_mut() {
        Some(span) if span.line == line && span.end == column && span.color == color => {
            span.end = column + cells;
        }
        _ => spans.push(Span {
            line,
            start: column,
            end: column + cells,
            color,
        }),
    }
}

pub struct Painter {
    text: TextSystem,
    paragraphs: HashMap<RunKey, Paragraph>,
    labels: HashMap<LabelKey, Paragraph>,
    pub metrics: Metrics,
    pub theme: Theme,
    font_size: f32,
}

impl Painter {
    pub fn new(appearance: Appearance, font_size: f32, scale: f64) -> Self {
        let mut text = TextSystem::new();
        let metrics = Metrics::measure(&mut text, font_size, scale);
        Self {
            text,
            paragraphs: HashMap::new(),
            labels: HashMap::new(),
            metrics,
            theme: Theme::new(appearance),
            font_size,
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.theme = Theme::new(appearance);
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    pub fn set_metrics(&mut self, font_size: f32, scale: f64) -> bool {
        if font_size == self.font_size && scale == self.metrics.scale {
            return false;
        }
        self.font_size = font_size;
        self.metrics = Metrics::measure(&mut self.text, font_size, scale);
        self.paragraphs.clear();
        true
    }

    pub fn text_system(&mut self) -> &mut TextSystem {
        &mut self.text
    }

    fn paragraph(&mut self, run: &Run) -> Paragraph {
        let key = (run.text.clone(), run.color, run.bold);
        if let Some(paragraph) = self.paragraphs.get(&key) {
            return paragraph.clone();
        }
        if self.paragraphs.len() >= PARAGRAPH_CACHE {
            self.paragraphs.clear();
        }
        let weight = if run.bold { BOLD } else { TextStyle::REGULAR };
        let style = TextStyle::new(self.font_size, run.color)
            .weight(weight)
            .monospace();
        let paragraph = self
            .text
            .paragraph(&run.text, style, None, TextAlign::Start);
        self.paragraphs.insert(key, paragraph.clone());
        paragraph
    }

    pub fn label(&mut self, text: &str, size: f32, color: Color, max_width: f32) -> Paragraph {
        let key = (text.to_owned(), color, size.to_bits(), max_width.to_bits());
        if let Some(paragraph) = self.labels.get(&key) {
            return paragraph.clone();
        }
        if self.labels.len() >= LABEL_CACHE {
            self.labels.clear();
        }
        let style = TextStyle::new(size, color);
        let mut paragraph = self.text.paragraph(text, style, None, TextAlign::Start);
        let mut characters: Vec<char> = text.chars().collect();
        while paragraph.width() > max_width && !characters.is_empty() {
            characters.pop();
            let shortened: String = characters.iter().chain([&ELLIPSIS]).collect();
            paragraph = self
                .text
                .paragraph(shortened.trim_start(), style, None, TextAlign::Start);
        }
        self.labels.insert(key, paragraph.clone());
        paragraph
    }

    fn centred(&mut self, text: &str, size: f32, color: Color, rect: Rect) -> Item {
        let paragraph = self.label(text, size, color, rect.width);
        Item::Text {
            origin: Point::new(
                snap(
                    rect.x + (rect.width - paragraph.width()) / 2.0,
                    self.metrics.scale,
                ),
                snap(
                    rect.y + (rect.height - paragraph.height()) / 2.0,
                    self.metrics.scale,
                ),
            ),
            paragraph,
        }
    }

    pub fn tab_bar(&mut self, bar: &TabBar<'_>, width: f32) -> Vec<Item> {
        let palette = self.theme.palette;
        let line = hairline(1.0, self.metrics.scale);
        let layout = TabBarLayout::new(bar.titles.len(), width);
        let hover_paint = palette.label_primary.with_alpha(HOVER_ALPHA);
        let label_color = |active: bool| {
            if active && bar.focused {
                palette.label_primary
            } else {
                palette.label_muted
            }
        };
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, 0.0, width, TAB_BAR_HEIGHT)),
                paint: palette.surface_background.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, TAB_BAR_HEIGHT - line, width, line)),
                paint: palette.separator_color.into(),
            },
        ];
        for (index, (tab, title)) in layout.tabs.iter().zip(bar.titles).enumerate() {
            let active = index == bar.active;
            let hovered = matches!(
                bar.hover,
                Some(TabTarget::Select(hovered) | TabTarget::Close(hovered)) if hovered == index
            );
            if active {
                items.push(Item::Fill {
                    shape: Shape::rect(*tab),
                    paint: self.theme.background.into(),
                });
            } else if hovered {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(tab.x, tab.y, tab.width, tab.height - line)),
                    paint: hover_paint.into(),
                });
            }
            if index + 1 < layout.tabs.len() {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        tab.x + tab.width - line,
                        0.0,
                        line,
                        TAB_BAR_HEIGHT,
                    )),
                    paint: palette.separator_color.into(),
                });
            }
            let label_area = Rect::new(
                tab.x + TAB_LABEL_INSET,
                tab.y,
                (tab.width - 2.0 * TAB_LABEL_INSET).max(0.0),
                tab.height,
            );
            items.push(self.centred(title, LABEL_SIZE, label_color(active), label_area));
            if hovered {
                let close = TabBarLayout::close_button(*tab);
                if bar.hover == Some(TabTarget::Close(index)) {
                    items.push(Item::Fill {
                        shape: Shape::rounded(close, close.width / 2.0),
                        paint: palette.label_primary.with_alpha(2 * HOVER_ALPHA).into(),
                    });
                }
                items.push(self.centred(CLOSE_SYMBOL, SYMBOL_SIZE, palette.label_muted, close));
            }
        }
        if bar.hover == Some(TabTarget::NewTab) {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    layout.new_tab.x,
                    0.0,
                    layout.new_tab.width,
                    TAB_BAR_HEIGHT - line,
                )),
                paint: hover_paint.into(),
            });
        }
        items.push(self.centred(
            NEW_TAB_SYMBOL,
            SYMBOL_SIZE,
            palette.label_muted,
            layout.new_tab,
        ));
        items
    }

    pub fn scene(&mut self, terminal: &Terminal, width: u32, height: u32) -> Scene {
        Scene {
            width,
            height,
            background: self.theme.background,
            items: self.terminal(terminal, Point::default(), &Highlights::default()),
        }
    }

    pub fn terminal(
        &mut self,
        terminal: &Terminal,
        origin: Point,
        highlights: &Highlights,
    ) -> Vec<Item> {
        let content = terminal.term.renderable_content();
        let colors = content.colors;
        let theme = self.theme;
        let metrics = self.metrics;
        let offset = content.display_offset;
        let selection = content.selection;
        let cursor = content.cursor;
        let focused = terminal.is_focused();
        let x_of = |column: usize| origin.x + PADDING + column as f32 * metrics.cell_width;
        let y_of = |line: usize| origin.y + PADDING + line as f32 * metrics.cell_height;
        let rect_of = |span: &Span| {
            Rect::new(
                x_of(span.start),
                y_of(span.line),
                x_of(span.end) - x_of(span.start),
                metrics.cell_height,
            )
        };

        let mut backgrounds: Vec<Span> = Vec::new();
        let mut selected: Vec<Span> = Vec::new();
        let mut found: Vec<Span> = Vec::new();
        let mut underlines: Vec<Span> = Vec::new();
        let mut strikeouts: Vec<Span> = Vec::new();
        let mut runs: Vec<Run> = Vec::new();
        let mut cursor_items = Vec::new();
        for indexed in content.display_iter {
            let Some(viewport) = term::point_to_viewport(offset, indexed.point) else {
                continue;
            };
            let (line, column) = (viewport.line, viewport.column.0);
            let cell = indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let bold = cell.flags.contains(Flags::BOLD);
            let mut foreground = theme.resolve(cell.fg, colors, bold);
            let mut background = theme.resolve(cell.bg, colors, false);
            if cell.flags.contains(Flags::DIM) {
                foreground = dimmed(foreground);
            }
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            if cell.flags.contains(Flags::HIDDEN) {
                foreground = background;
            }
            let cells = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let point = indexed.point;
            if highlights
                .current
                .as_ref()
                .is_some_and(|current| current.contains(&point))
            {
                push_span(&mut found, line, column, cells, theme.current_match);
            } else if highlights
                .matches
                .iter()
                .any(|candidate| candidate.contains(&point))
            {
                push_span(&mut found, line, column, cells, theme.match_highlight);
            }
            let in_link = highlights
                .link
                .as_ref()
                .is_some_and(|link| link.contains(&point));
            if in_link {
                foreground = theme.link;
            }
            if selection.is_some_and(|range| range.contains(indexed.point)) {
                let color = if focused {
                    theme.selection
                } else {
                    theme.inactive_selection
                };
                push_span(&mut selected, line, column, cells, color);
            }
            let is_cursor = cursor.shape != CursorShape::Hidden && cursor.point == indexed.point;
            let block_cursor = is_cursor && focused && cursor.shape == CursorShape::Block;
            if block_cursor {
                std::mem::swap(&mut foreground, &mut background);
                if background == foreground {
                    background = theme.foreground;
                    foreground = theme.background;
                }
            }
            if background != theme.background {
                push_span(&mut backgrounds, line, column, cells, background);
            }
            if is_cursor && !block_cursor {
                let rect = Rect::new(
                    x_of(column),
                    y_of(line),
                    cells as f32 * metrics.cell_width,
                    metrics.cell_height,
                );
                cursor_items.push(self.cursor(cursor.shape, focused, rect));
            }
            if in_link || cell.flags.intersects(Flags::ALL_UNDERLINES) {
                push_span(&mut underlines, line, column, cells, foreground);
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                push_span(&mut strikeouts, line, column, cells, foreground);
            }
            let character = cell.c;
            if character == ' ' || character == '\0' {
                continue;
            }
            let joins = character.is_ascii()
                && runs.last().is_some_and(|run| {
                    run.line == line
                        && run.color == foreground
                        && run.bold == bold
                        && run.text.is_ascii()
                        && run.column + run.text.len() == column
                });
            match runs.last_mut() {
                Some(run) if joins => run.text.push(character),
                _ => runs.push(Run {
                    line,
                    column,
                    text: character.to_string(),
                    color: foreground,
                    bold,
                }),
            }
        }

        let line_width = hairline(1.0, metrics.scale);
        let mut items: Vec<Item> = backgrounds
            .iter()
            .chain(&found)
            .chain(&selected)
            .map(|span| Item::Fill {
                shape: Shape::rect(rect_of(span)),
                paint: span.color.into(),
            })
            .collect();
        for run in &runs {
            let paragraph = self.paragraph(run);
            items.push(Item::Text {
                origin: Point::new(x_of(run.column), y_of(run.line)),
                paragraph,
            });
        }
        let underline_offset = metrics.cell_height - snap(2.0, metrics.scale);
        let strikeout_offset = snap(metrics.cell_height / 2.0, metrics.scale);
        for (spans, offset) in [
            (&underlines, underline_offset),
            (&strikeouts, strikeout_offset),
        ] {
            items.extend(spans.iter().map(|span| {
                let rect = rect_of(span);
                Item::Fill {
                    shape: Shape::rect(Rect::new(rect.x, rect.y + offset, rect.width, line_width)),
                    paint: span.color.into(),
                }
            }));
        }
        items.extend(cursor_items);
        items
    }

    fn cursor(&self, shape: CursorShape, focused: bool, cell: Rect) -> Item {
        let scale = self.metrics.scale;
        let paint = self.theme.foreground.into();
        match shape {
            CursorShape::Beam if focused => Item::Fill {
                shape: Shape::rect(Rect::new(cell.x, cell.y, hairline(2.0, scale), cell.height)),
                paint,
            },
            CursorShape::Underline if focused => {
                let height = hairline(2.0, scale);
                Item::Fill {
                    shape: Shape::rect(Rect::new(
                        cell.x,
                        cell.y + cell.height - height,
                        cell.width,
                        height,
                    )),
                    paint,
                }
            }
            _ => {
                let width = hairline(1.0, scale);
                Item::Stroke {
                    shape: Shape::rect(cell.inset(width / 2.0)),
                    width,
                    paint,
                }
            }
        }
    }
}
