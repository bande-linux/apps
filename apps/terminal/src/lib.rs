use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{self, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    ClearMode, Color as AnsiColor, CursorShape, Handler, NamedColor, Processor, Rgb,
};
use unbound_toolkit::ui::{Key, Modifiers};
use unbound_toolkit::{
    Color, Item, Paragraph, Point, Rect, Scene, Scheme, Shape, TextAlign, TextStyle, TextSystem,
};

pub const FONT_SIZE: f32 = 11.0;
pub const PADDING: f32 = 4.0;
pub const COLUMNS: usize = 80;
pub const ROWS: usize = 24;
pub const SCROLLBACK: usize = 10_000;
const PARAGRAPH_CACHE: usize = 8192;
const BOLD: f32 = 700.0;

const BASIC: [u32; 16] = [
    0x000000, 0x990000, 0x00A600, 0x999900, 0x0000B2, 0xB200B2, 0x00A6B2, 0xBFBFBF, 0x666666,
    0xE50000, 0x00D900, 0xE5E500, 0x0000FF, 0xE500E5, 0x00E5E5, 0xE5E5E5,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone, Default)]
pub struct Listener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.borrow_mut().push(event);
    }
}

#[derive(Debug, Clone, Copy)]
struct Theme {
    foreground: Color,
    background: Color,
    selection: Color,
}

impl Theme {
    fn for_scheme(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Light => Self {
                foreground: Color::hex(0x000000),
                background: Color::hex(0xFFFFFF),
                selection: Color::hex(0xB4D5FE),
            },
            Scheme::Dark => Self {
                foreground: Color::hex(0xFFFFFF),
                background: Color::hex(0x1E1E1E),
                selection: Color::hex(0x3F638B),
            },
        }
    }
}

fn from_rgb(rgb: Rgb) -> Color {
    Color {
        r: rgb.r,
        g: rgb.g,
        b: rgb.b,
        a: 255,
    }
}

fn indexed(index: u8) -> Color {
    match index {
        0..16 => Color::hex(BASIC[usize::from(index)]),
        16..232 => {
            let cube = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            Color {
                r: level(cube / 36),
                g: level(cube / 6 % 6),
                b: level(cube % 6),
                a: 255,
            }
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Color {
                r: gray,
                g: gray,
                b: gray,
                a: 255,
            }
        }
    }
}

fn dimmed(color: Color) -> Color {
    let scale = |channel: u8| (u16::from(channel) * 2 / 3) as u8;
    Color {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
        a: color.a,
    }
}

impl Theme {
    fn resolve(&self, color: AnsiColor, colors: &Colors, bold: bool) -> Color {
        match color {
            AnsiColor::Spec(rgb) => from_rgb(rgb),
            AnsiColor::Indexed(index) => {
                let index = if bold && index < 8 { index + 8 } else { index };
                colors[usize::from(index)].map_or_else(|| indexed(index), from_rgb)
            }
            AnsiColor::Named(named) => {
                if let Some(rgb) = colors[named] {
                    return from_rgb(rgb);
                }
                let number = named as usize;
                match named {
                    _ if number < 8 && bold => indexed(number as u8 + 8),
                    _ if number < 16 => indexed(number as u8),
                    NamedColor::Background => self.background,
                    NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => {
                        self.foreground
                    }
                    NamedColor::DimForeground => dimmed(self.foreground),
                    _ => {
                        let dim = number.saturating_sub(NamedColor::DimBlack as usize);
                        dimmed(indexed((dim % 8) as u8))
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub cell_width: f32,
    pub cell_height: f32,
}

impl Metrics {
    pub fn measure(text: &mut TextSystem) -> Self {
        let style = TextStyle::new(FONT_SIZE, Color::hex(0)).monospace();
        let sample = text.paragraph("MMMMMMMMMM", style, None, TextAlign::Start);
        Self {
            cell_width: sample.width() / 10.0,
            cell_height: sample.height().ceil(),
        }
    }

    pub fn window_size(&self, size: GridSize) -> (u32, u32) {
        (
            (size.columns as f32 * self.cell_width + 2.0 * PADDING).ceil() as u32,
            (size.lines as f32 * self.cell_height + 2.0 * PADDING).ceil() as u32,
        )
    }

    pub fn grid_for(&self, width: u32, height: u32) -> GridSize {
        let fit = |pixels: u32, cell: f32| {
            (((pixels as f32 - 2.0 * PADDING) / cell).floor() as usize).max(1)
        };
        GridSize {
            columns: fit(width, self.cell_width).max(2),
            lines: fit(height, self.cell_height),
        }
    }

    pub fn cell_at(&self, point: Point) -> (usize, usize) {
        let column = ((point.x - PADDING) / self.cell_width).max(0.0) as usize;
        let line = ((point.y - PADDING) / self.cell_height).max(0.0) as usize;
        (column, line)
    }
}

type RunKey = (String, Color, bool);

pub struct Terminal {
    term: Term<Listener>,
    parser: Processor,
    events: Listener,
    size: GridSize,
    pub metrics: Metrics,
    theme: Theme,
    text: TextSystem,
    paragraphs: HashMap<RunKey, Paragraph>,
    scroll_remainder: f32,
}

struct Run {
    column: usize,
    text: String,
    color: Color,
    bold: bool,
}

impl Terminal {
    pub fn new(size: GridSize, scheme: Scheme) -> Self {
        let mut text = TextSystem::new();
        let metrics = Metrics::measure(&mut text);
        let events = Listener::default();
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &size, events.clone()),
            parser: Processor::new(),
            events,
            size,
            metrics,
            theme: Theme::for_scheme(scheme),
            text,
            paragraphs: HashMap::new(),
            scroll_remainder: 0.0,
        }
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: u16::try_from(self.size.lines).unwrap_or(u16::MAX),
            num_cols: u16::try_from(self.size.columns).unwrap_or(u16::MAX),
            cell_width: self.metrics.cell_width.round() as u16,
            cell_height: self.metrics.cell_height.round() as u16,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut *self.events.0.borrow_mut())
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn set_scheme(&mut self, scheme: Scheme) {
        self.theme = Theme::for_scheme(scheme);
    }

    pub fn set_focus(&mut self, focused: bool) {
        self.term.is_focused = focused;
    }

    pub fn resize(&mut self, size: GridSize) -> bool {
        if size == self.size {
            return false;
        }
        self.size = size;
        self.term.resize(size);
        true
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn scroll_pages(&mut self, pages: i32) {
        let scroll = if pages > 0 {
            Scroll::PageUp
        } else {
            Scroll::PageDown
        };
        for _ in 0..pages.unsigned_abs() {
            self.term.scroll_display(scroll);
        }
    }

    pub fn scroll_pixels(&mut self, dy: f32) -> i32 {
        self.scroll_remainder += dy / self.metrics.cell_height;
        let lines = self.scroll_remainder.trunc();
        self.scroll_remainder -= lines;
        lines as i32
    }

    pub fn scroll_lines(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn clear_scrollback(&mut self) {
        self.term.clear_screen(ClearMode::Saved);
    }

    fn grid_point(&self, at: Point) -> (GridPoint, Side) {
        let (column, line) = self.metrics.cell_at(at);
        let column = column.min(self.size.columns - 1);
        let line = line.min(self.size.lines - 1);
        let offset = self.term.grid().display_offset();
        let point = term::viewport_to_point(offset, GridPoint::new(line, Column(column)));
        let within = (at.x - PADDING) / self.metrics.cell_width - column as f32;
        let side = if within < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (point, side)
    }

    pub fn start_selection(&mut self, at: Point) {
        let (point, side) = self.grid_point(at);
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    }

    pub fn extend_selection(&mut self, at: Point) {
        let (point, side) = self.grid_point(at);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
    }

    pub fn finish_selection(&mut self) {
        if self
            .term
            .selection
            .as_ref()
            .is_some_and(Selection::is_empty)
        {
            self.term.selection = None;
        }
    }

    pub fn screen_text(&self) -> Vec<String> {
        let grid = self.term.grid();
        let offset = grid.display_offset();
        (0..self.size.lines)
            .map(|line| {
                let point = term::viewport_to_point(offset, GridPoint::new(line, Column(0)));
                let row = &grid[point.line];
                (0..self.size.columns)
                    .map(|column| row[Column(column)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
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
        let style = TextStyle::new(FONT_SIZE, run.color)
            .weight(weight)
            .monospace();
        let paragraph = self
            .text
            .paragraph(&run.text, style, None, TextAlign::Start);
        self.paragraphs.insert(key, paragraph.clone());
        paragraph
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let content = self.term.renderable_content();
        let colors = content.colors;
        let theme = self.theme;
        let offset = content.display_offset;
        let selection = content.selection;
        let cursor = content.cursor;
        let focused = self.term.is_focused;
        let (cell_width, cell_height) = (self.metrics.cell_width, self.metrics.cell_height);
        let x_of = |column: usize| PADDING + column as f32 * cell_width;
        let y_of = |line: usize| PADDING + line as f32 * cell_height;

        let mut backgrounds: Vec<Item> = Vec::new();
        let mut decorations: Vec<Item> = Vec::new();
        let mut runs: Vec<(usize, Run)> = Vec::new();
        let mut cursor_item = None;
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
                background = theme.selection;
            }
            let is_cursor = cursor.shape != CursorShape::Hidden && cursor.point == indexed.point;
            if is_cursor && focused {
                std::mem::swap(&mut foreground, &mut background);
                if background == foreground {
                    background = theme.foreground;
                    foreground = theme.background;
                }
            }
            let rect = Rect::new(
                x_of(column),
                y_of(line),
                cells as f32 * cell_width,
                cell_height,
            );
            if background != theme.background {
                backgrounds.push(Item::Fill {
                    shape: Shape::rect(rect),
                    paint: background.into(),
                });
            }
            if is_cursor && !focused {
                cursor_item = Some(Item::Stroke {
                    shape: Shape::rect(rect.inset(0.5)),
                    width: 1.0,
                    paint: theme.foreground.into(),
                });
            }
            if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                decorations.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        rect.x,
                        rect.y + cell_height - 2.0,
                        rect.width,
                        1.0,
                    )),
                    paint: foreground.into(),
                });
            }
            let character = cell.c;
            if character == ' ' || character == '\0' {
                continue;
            }
            let joins = character.is_ascii()
                && runs.last().is_some_and(|(run_line, run)| {
                    *run_line == line
                        && run.color == foreground
                        && run.bold == bold
                        && run.text.is_ascii()
                        && run.column + run.text.chars().count() == column
                });
            match runs.last_mut() {
                Some((_, run)) if joins => run.text.push(character),
                _ => runs.push((
                    line,
                    Run {
                        column,
                        text: character.to_string(),
                        color: foreground,
                        bold,
                    },
                )),
            }
        }

        let mut items = backgrounds;
        for (line, run) in &runs {
            let paragraph = self.paragraph(run);
            items.push(Item::Text {
                origin: Point::new(x_of(run.column), y_of(*line)),
                paragraph,
            });
        }
        items.extend(decorations);
        items.extend(cursor_item);
        Scene {
            width,
            height,
            background: theme.background,
            items,
        }
    }
}

fn modifier_parameter(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.control)
}

fn cursor_key(letter: char, modifiers: Modifiers, mode: TermMode) -> Vec<u8> {
    let parameter = modifier_parameter(modifiers);
    if parameter > 1 {
        format!("\x1b[1;{parameter}{letter}").into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn tilde_key(number: u8, modifiers: Modifiers) -> Vec<u8> {
    let parameter = modifier_parameter(modifiers);
    if parameter > 1 {
        format!("\x1b[{number};{parameter}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

fn control_byte(character: char) -> Option<u8> {
    match character.to_ascii_lowercase() {
        letter @ 'a'..='z' => Some(letter as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn with_meta(alt: bool, bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        let mut prefixed = vec![0x1b];
        prefixed.extend(bytes);
        prefixed
    } else {
        bytes
    }
}

pub fn encode_key(key: Key, modifiers: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    if modifiers.logo {
        return None;
    }
    let plain = Modifiers {
        alt: false,
        ..modifiers
    };
    let bytes = match key {
        Key::Left if modifiers.alt && plain == Modifiers::default() => b"\x1bb".to_vec(),
        Key::Right if modifiers.alt && plain == Modifiers::default() => b"\x1bf".to_vec(),
        Key::Up => cursor_key('A', modifiers, mode),
        Key::Down => cursor_key('B', modifiers, mode),
        Key::Right => cursor_key('C', modifiers, mode),
        Key::Left => cursor_key('D', modifiers, mode),
        Key::Home => cursor_key('H', modifiers, mode),
        Key::End => cursor_key('F', modifiers, mode),
        Key::Insert => tilde_key(2, modifiers),
        Key::Delete => tilde_key(3, modifiers),
        Key::PageUp => tilde_key(5, modifiers),
        Key::PageDown => tilde_key(6, modifiers),
        Key::Function(number @ 1..=4) => {
            let letter = (b'P' + number - 1) as char;
            let parameter = modifier_parameter(modifiers);
            if parameter > 1 {
                format!("\x1b[1;{parameter}{letter}").into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        Key::Function(number @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(number - 5)];
            tilde_key(code, modifiers)
        }
        Key::Function(_) => return None,
        Key::Enter => with_meta(modifiers.alt, b"\r".to_vec()),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => with_meta(modifiers.alt, b"\t".to_vec()),
        Key::Escape => with_meta(modifiers.alt, b"\x1b".to_vec()),
        Key::Backspace if modifiers.control => with_meta(modifiers.alt, vec![0x08]),
        Key::Backspace => with_meta(modifiers.alt, vec![0x7f]),
        Key::Space if modifiers.control => with_meta(modifiers.alt, vec![0]),
        Key::Space => with_meta(modifiers.alt, b" ".to_vec()),
        Key::Character(character) => {
            let bytes = match control_byte(character).filter(|_| modifiers.control) {
                Some(byte) => vec![byte],
                None => character.to_string().into_bytes(),
            };
            with_meta(modifiers.alt, bytes)
        }
    };
    Some(bytes)
}

pub fn bracketed(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let cleaned = text.replace('\x1b', "");
        format!("\x1b[200~{cleaned}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

pub fn wants_scroll_as_arrows(mode: TermMode) -> bool {
    mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL)
}
