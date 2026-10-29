use std::collections::HashMap;

use alacritty_terminal::index::Side;
use alacritty_terminal::term::{self, cell::Flags};
use alacritty_terminal::vte::ansi::CursorShape;
use unbound_toolkit::{
    Appearance, Color, Item, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::terminal::{CellPoint, GridSize, Terminal};
use crate::theme::{Theme, dimmed};

pub const DEFAULT_FONT_SIZE: f32 = 11.0;
pub const PADDING: f32 = 4.0;
const PARAGRAPH_CACHE: usize = 8192;
const BOLD: f32 = 700.0;
const MEASURE_SAMPLE: &str = "MMMMMMMMMM";

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

    pub fn scene(&mut self, terminal: &Terminal, width: u32, height: u32) -> Scene {
        Scene {
            width,
            height,
            background: self.theme.background,
            items: self.terminal(terminal, Point::default()),
        }
    }

    pub fn terminal(&mut self, terminal: &Terminal, origin: Point) -> Vec<Item> {
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
            if cell.flags.intersects(Flags::ALL_UNDERLINES) {
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
