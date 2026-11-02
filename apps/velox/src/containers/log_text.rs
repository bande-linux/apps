use std::collections::VecDeque;
use std::ops::Range;

use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Event, ScrollState, Size, Widget, WidgetId};
use bande_toolkit::{Item, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle};

use super::ansi::{Pen, Styled};

pub const LIMIT: usize = 10_000;
const PADDING: f32 = 8.0;
const SPOKEN_LINES: usize = 100;
const FAINT: f32 = 0.6;

struct Line {
    text: String,
    chars: usize,
    spans: Vec<(Range<usize>, Pen)>,
}

pub struct LogText {
    id: WidgetId,
    label: String,
    lines: VecDeque<Line>,
    filter: String,
    shown: Vec<usize>,
    wrap: bool,
    follow: bool,
    pin: bool,
    rows: Vec<(usize, usize)>,
    columns: usize,
    widest: usize,
    stale: bool,
    cell: Size,
    scroll: ScrollState,
    shift: f32,
    texts: Vec<(Point, Paragraph)>,
    frame: Rect,
}

fn style(cx: &Context, pen: Pen) -> TextStyle {
    let ink = pen
        .tone
        .map_or(cx.palette.ink, |tone| tone.color(&cx.palette));
    let ink = match pen.faint {
        true => ink.with_opacity(FAINT),
        false => ink,
    };
    let style = TextStyle::role(TextRole::Mono, ink);
    match pen.bold {
        true => style.weight(TextStyle::SEMIBOLD),
        false => style,
    }
}

fn byte_at(text: &str, char: usize) -> usize {
    text.char_indices()
        .nth(char)
        .map_or(text.len(), |(index, _)| index)
}

impl LogText {
    pub fn new(label: &str, wrap: bool, follow: bool) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            lines: VecDeque::new(),
            filter: String::new(),
            shown: Vec::new(),
            wrap,
            follow,
            pin: follow,
            rows: Vec::new(),
            columns: 0,
            widest: 0,
            stale: true,
            cell: Size::default(),
            scroll: ScrollState::default(),
            shift: 0.0,
            texts: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn matches(&self, line: &Line) -> bool {
        self.filter.is_empty() || line.text.to_lowercase().contains(&self.filter)
    }

    fn reshow(&mut self) {
        self.shown = (0..self.lines.len())
            .filter(|index| self.matches(&self.lines[*index]))
            .collect();
        self.stale = true;
    }

    pub fn push(&mut self, lines: impl IntoIterator<Item = Styled>) {
        let mut trimmed = false;
        for styled in lines {
            let line = Line {
                chars: styled.text.chars().count(),
                text: styled.text,
                spans: styled.spans,
            };
            if self.lines.len() == LIMIT {
                self.lines.pop_front();
                trimmed = true;
            }
            if !trimmed && self.matches(&line) {
                self.shown.push(self.lines.len());
            }
            self.lines.push_back(line);
        }
        if trimmed {
            self.reshow();
        }
        self.stale = true;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.reshow();
        self.shift = 0.0;
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.trim().to_lowercase();
        self.reshow();
    }

    pub fn set_wrap(&mut self, wrap: bool) {
        self.wrap = wrap;
        self.shift = 0.0;
        self.stale = true;
    }

    pub fn set_follow(&mut self, follow: bool) {
        self.follow = follow;
        self.pin = follow;
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.shown.len(), self.lines.len())
    }

    pub fn shown_text(&self) -> String {
        self.shown
            .iter()
            .map(|index| self.lines[*index].text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn relayout(&mut self) {
        self.rows.clear();
        self.widest = 0;
        for (shown, index) in self.shown.iter().enumerate() {
            let chars = self.lines[*index].chars;
            self.widest = self.widest.max(chars);
            match self.wrap {
                true => {
                    let parts = chars.div_ceil(self.columns).max(1);
                    self.rows
                        .extend((0..parts).map(|part| (shown, part * self.columns)));
                }
                false => self.rows.push((shown, 0)),
            }
        }
        self.stale = false;
    }

    fn content_height(&self) -> f32 {
        self.rows.len() as f32 * self.cell.height + 2.0 * PADDING
    }

    fn max_shift(&self) -> f32 {
        match self.wrap {
            true => 0.0,
            false => {
                (self.widest as f32 * self.cell.width + 2.0 * PADDING - self.frame.width).max(0.0)
            }
        }
    }

    fn row_texts(&self, cx: &mut Context, row: usize, origin: Point) -> Vec<(Point, Paragraph)> {
        let Some(&(shown, start)) = self.rows.get(row) else {
            return Vec::new();
        };
        let line = &self.lines[self.shown[shown]];
        let end = match self.wrap {
            true => (start + self.columns).min(line.chars),
            false => line.chars,
        };
        let (first, last) = (byte_at(&line.text, start), byte_at(&line.text, end));
        let mut texts = Vec::new();
        for (range, pen) in &line.spans {
            let (from, to) = (range.start.max(first), range.end.min(last));
            if from >= to {
                continue;
            }
            let Some(text) = line.text.get(from..to) else {
                continue;
            };
            let column = line.text[first..from].chars().count();
            let paragraph = cx
                .text
                .paragraph(text, style(cx, *pen), None, TextAlign::Start);
            texts.push((
                Point::new(origin.x + column as f32 * self.cell.width, origin.y),
                paragraph,
            ));
        }
        texts
    }
}

impl Widget for LogText {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let sample = cx.text.paragraph(
            "M",
            TextStyle::role(TextRole::Mono, cx.palette.ink),
            None,
            TextAlign::Start,
        );
        self.cell = Size::new(sample.width().max(1.0), sample.height().ceil().max(1.0));
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        let columns = ((frame.width - 2.0 * PADDING) / self.cell.width)
            .floor()
            .max(1.0) as usize;
        if self.stale || columns != self.columns {
            self.columns = columns;
            self.relayout();
        }
        let at_bottom = self.scroll.offset(cx) >= self.scroll.limit() - 1.0;
        self.frame = frame;
        self.scroll.set_extent(self.content_height(), frame.height);
        if self.pin || (self.follow && at_bottom) {
            let bottom = self.content_height();
            self.scroll.follow(cx, bottom, bottom);
            self.pin = false;
        }
        self.shift = self.shift.clamp(0.0, self.max_shift());
        let offset = self.scroll.offset(cx);
        let height = self.cell.height;
        let first = ((offset - PADDING) / height).floor().max(0.0) as usize;
        let last = (((offset + frame.height - PADDING) / height).ceil().max(0.0) as usize)
            .min(self.rows.len());
        self.texts = (first..last)
            .flat_map(|row| {
                let origin = Point::new(
                    frame.x + PADDING - self.shift,
                    frame.y + PADDING + row as f32 * height - offset,
                );
                self.row_texts(cx, row, origin)
            })
            .collect();
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut inside: Vec<Item> = self
            .texts
            .iter()
            .map(|(origin, paragraph)| Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            })
            .collect();
        self.scroll.paint_scrollbar(cx, self.frame, &mut inside);
        items.push(Item::Clip {
            shape: Shape::rect(self.frame),
            items: inside,
        });
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::Scroll(scroll) => {
                let sideways = match self.wrap {
                    true => false,
                    false => {
                        let shift = (self.shift + scroll.delta.x).clamp(0.0, self.max_shift());
                        let moved = shift != self.shift;
                        self.shift = shift;
                        moved
                    }
                };
                self.scroll.scroll(cx, scroll) | sideways
            }
            event => self.scroll.pointer(cx, self.frame, event),
        }
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::Log);
        node.set_label(self.label.clone());
        let spoken = &self.shown[self.shown.len().saturating_sub(SPOKEN_LINES)..];
        node.set_value(
            spoken
                .iter()
                .map(|index| self.lines[*index].text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        self.scroll.describe(cx, &mut node);
        node
    }
}
