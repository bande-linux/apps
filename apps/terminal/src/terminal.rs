use std::cell::RefCell;
use std::rc::Rc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
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

    pub fn start_selection(&mut self, cell: CellPoint) {
        let point = self.grid_point(cell);
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, cell.side));
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
