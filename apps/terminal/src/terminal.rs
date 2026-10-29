use std::cell::RefCell;
use std::rc::Rc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{self, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{ClearMode, Handler, Processor};

use crate::SCROLLBACK;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPoint {
    pub column: usize,
    pub line: usize,
    pub side: Side,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Characters,
    Words,
    Lines,
    Block,
}

impl SelectionKind {
    pub fn for_clicks(clicks: u8, block: bool) -> Self {
        match clicks {
            2 => Self::Words,
            3.. => Self::Lines,
            _ if block => Self::Block,
            _ => Self::Characters,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlights {
    pub matches: Vec<Match>,
    pub current: Option<Match>,
    pub link: Option<Match>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    Older,
    Newer,
}

struct Search {
    regex: RegexSearch,
    current: Option<Match>,
}

const REGEX_SPECIAL: &str = "\\.+*?()|[]{}^$#&-~";

pub fn escape_regex(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if REGEX_SPECIAL.contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[derive(Clone, Default)]
pub struct Listener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.borrow_mut().push(event);
    }
}

pub struct Terminal {
    pub(crate) term: Term<Listener>,
    parser: Processor,
    events: Listener,
    size: GridSize,
    cell_size: (u16, u16),
    search: Option<Search>,
}

impl Terminal {
    pub fn new(size: GridSize) -> Self {
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
            cell_size: (0, 0),
            search: None,
        }
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: u16::try_from(self.size.lines).unwrap_or(u16::MAX),
            num_cols: u16::try_from(self.size.columns).unwrap_or(u16::MAX),
            cell_width: self.cell_size.0,
            cell_height: self.cell_size.1,
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

    pub fn is_focused(&self) -> bool {
        self.term.is_focused
    }

    pub fn set_focus(&mut self, focused: bool) -> Option<&'static [u8]> {
        let changed = self.term.is_focused != focused;
        self.term.is_focused = focused;
        let reports = changed && self.mode().contains(TermMode::FOCUS_IN_OUT);
        reports.then_some(if focused { b"\x1b[I" } else { b"\x1b[O" })
    }

    pub fn resize(&mut self, size: GridSize, cell_size: (u16, u16)) -> bool {
        if size == self.size && cell_size == self.cell_size {
            return false;
        }
        self.cell_size = cell_size;
        if size != self.size {
            self.size = size;
            self.term.resize(size);
        }
        true
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
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

    pub fn scroll_lines(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn clear_scrollback(&mut self) {
        self.term.clear_screen(ClearMode::Saved);
    }

    fn grid_point(&self, cell: CellPoint) -> GridPoint {
        let column = cell.column.min(self.size.columns - 1);
        let line = cell.line.min(self.size.lines - 1);
        term::viewport_to_point(self.display_offset(), GridPoint::new(line, Column(column)))
    }

    pub fn start_selection(&mut self, cell: CellPoint, kind: SelectionKind) {
        let point = self.grid_point(cell);
        let kind = match kind {
            SelectionKind::Characters => SelectionType::Simple,
            SelectionKind::Words => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
            SelectionKind::Block => SelectionType::Block,
        };
        self.term.selection = Some(Selection::new(kind, point, cell.side));
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.is_some()
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn extend_selection(&mut self, cell: CellPoint) {
        let point = self.grid_point(cell);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, cell.side);
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

    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let start = GridPoint::new(grid.topmost_line(), Column(0));
        let end = GridPoint::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        self.term.selection = Some(selection);
    }

    fn viewport_end(&self) -> GridPoint {
        let line = Line(self.size.lines as i32 - 1 - self.display_offset() as i32);
        GridPoint::new(line, self.term.grid().last_column())
    }

    fn viewport_start(&self) -> GridPoint {
        GridPoint::new(Line(-(self.display_offset() as i32)), Column(0))
    }

    pub fn find(&mut self, query: &str) -> bool {
        self.search = None;
        if query.is_empty() {
            return true;
        }
        let Ok(regex) = RegexSearch::new(&escape_regex(query)) else {
            return false;
        };
        let mut search = Search {
            regex,
            current: None,
        };
        let origin = self.viewport_end();
        search.current = self.term.search_next(
            &mut search.regex,
            origin,
            Direction::Left,
            Side::Right,
            None,
        );
        let found = search.current.is_some();
        self.search = Some(search);
        self.reveal_match();
        found
    }

    pub fn find_again(&mut self, direction: SearchDirection) -> bool {
        let fallback = self.viewport_end();
        let Some(search) = &mut self.search else {
            return false;
        };
        let (origin, direction, side) = match (&search.current, direction) {
            (Some(current), SearchDirection::Older) => (
                current.start().sub(&self.term, Boundary::None, 1),
                Direction::Left,
                Side::Left,
            ),
            (Some(current), SearchDirection::Newer) => (
                current.end().add(&self.term, Boundary::None, 1),
                Direction::Right,
                Side::Right,
            ),
            (None, SearchDirection::Older) => (fallback, Direction::Left, Side::Right),
            (None, SearchDirection::Newer) => (fallback, Direction::Right, Side::Left),
        };
        search.current = self
            .term
            .search_next(&mut search.regex, origin, direction, side, None);
        let found = search.current.is_some();
        self.reveal_match();
        found
    }

    fn reveal_match(&mut self) {
        if let Some(current) = self
            .search
            .as_ref()
            .and_then(|search| search.current.clone())
        {
            self.term.scroll_to_point(*current.start());
            self.term.scroll_to_point(*current.end());
        }
    }

    pub fn end_find(&mut self) {
        self.search = None;
    }

    pub fn highlights(&mut self) -> Highlights {
        let (start, end) = (self.viewport_start(), self.viewport_end());
        let Some(search) = &mut self.search else {
            return Highlights::default();
        };
        let matches =
            RegexIter::new(start, end, Direction::Right, &self.term, &mut search.regex).collect();
        Highlights {
            matches,
            current: search.current.clone(),
            link: None,
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
}
