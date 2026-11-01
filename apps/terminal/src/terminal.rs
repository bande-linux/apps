use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{self, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{ClearMode, CursorShape, CursorStyle, Handler, Processor};
use bande_xdg::file_uri;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub range: Match,
    pub uri: String,
}

const URL_PATTERN: &str = "(https?://|ftp://|file:|mailto:|gemini://|gopher://|news:|magnet:|ipfs:|ipns:|git://|ssh:|sftp:)[^\\u{0000}-\\u{001F}\\u{007F}-\\u{009F}<>\"\\s{-}\\^⟨⟩`]+";
const TRAILING_PUNCTUATION: &[char] = &['.', ',', ':', ';', '!', '?', '\'', '"'];
const OPENABLE_SCHEMES: &[&str] = &[
    "http:", "https:", "ftp:", "file:", "mailto:", "gemini:", "gopher:", "news:", "magnet:",
    "ipfs:", "ipns:", "git:", "ssh:", "sftp:",
];

pub fn is_openable(uri: &str) -> bool {
    let lowercase = uri.to_ascii_lowercase();
    OPENABLE_SCHEMES
        .iter()
        .any(|scheme| lowercase.starts_with(scheme))
}

fn trimmed_url(text: &str) -> &str {
    let mut url = text;
    loop {
        let trimmed = url.trim_end_matches(TRAILING_PUNCTUATION);
        let unbalanced =
            trimmed.ends_with(')') && trimmed.matches(')').count() > trimmed.matches('(').count();
        let trimmed = if unbalanced {
            &trimmed[..trimmed.len() - 1]
        } else {
            trimmed
        };
        if trimmed.len() == url.len() {
            return url;
        }
        url = trimmed;
    }
}

const PROMPT_MARK: &str = "bande-prompt";

const PATH_DELIMITERS: &[char] = &[
    ' ', '\t', '\0', '"', '\'', '`', '(', ')', '[', ']', '{', '}', '<', '>', '|', ',', ';', '=',
];

pub fn written_path(text: &str) -> &str {
    let mut path = trimmed_url(text);
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((before, number))
                if !before.is_empty()
                    && !number.is_empty()
                    && number.chars().all(|digit| digit.is_ascii_digit()) =>
            {
                path = before;
            }
            _ => break,
        }
    }
    path
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

fn config(shape: CursorShape) -> Config {
    Config {
        scrolling_history: SCROLLBACK,
        default_cursor_style: CursorStyle {
            shape,
            blinking: false,
        },
        ..Config::default()
    }
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
    urls: Option<RegexSearch>,
    shift: f32,
    cursor_shape: CursorShape,
}

impl Terminal {
    pub fn new(size: GridSize) -> Self {
        let events = Listener::default();
        Self {
            term: Term::new(config(CursorShape::Block), &size, events.clone()),
            parser: Processor::new(),
            events,
            size,
            cell_size: (0, 0),
            search: None,
            urls: None,
            shift: 0.0,
            cursor_shape: CursorShape::Block,
        }
    }

    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        if self.cursor_shape != shape {
            self.cursor_shape = shape;
            self.term.set_options(config(shape));
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
        self.shift = 0.0;
        if size != self.size {
            self.size = size;
            self.term.resize(size);
        }
        true
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    fn scroll_display(&mut self, scroll: Scroll) {
        self.shift = 0.0;
        self.term.scroll_display(scroll);
    }

    pub fn scroll_shift(&self) -> f32 {
        self.shift
    }

    pub fn scroll_smoothly(&mut self, points: f32, line_height: f32) {
        let shift = self.shift + points;
        let lines = (shift / line_height).floor();
        let mut shift = shift - lines * line_height;
        let history = self.term.grid().history_size() as i32;
        let current = self.display_offset() as i32;
        let wanted = current + lines as i32;
        let offset = wanted.clamp(0, history);
        if wanted < 0 || offset == history {
            shift = 0.0;
        }
        self.term.scroll_display(Scroll::Delta(offset - current));
        self.shift = shift;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_display(Scroll::Bottom);
    }

    pub fn scroll_pages(&mut self, pages: i32) {
        let scroll = if pages > 0 {
            Scroll::PageUp
        } else {
            Scroll::PageDown
        };
        for _ in 0..pages.unsigned_abs() {
            self.scroll_display(scroll);
        }
    }

    pub fn scroll_lines(&mut self, lines: i32) {
        self.scroll_display(Scroll::Delta(lines));
    }

    pub fn mark_prompt(&mut self) {
        let start = self.term.line_search_left(self.term.grid().cursor.point);
        let cell = &mut self.term.grid_mut()[start.line][Column(0)];
        if cell.hyperlink().is_none() {
            cell.set_hyperlink(Some(Hyperlink::new(Some(PROMPT_MARK), String::new())));
        }
    }

    fn prompt_marks(&self) -> Vec<Line> {
        let grid = self.term.grid();
        (grid.topmost_line().0..=grid.bottommost_line().0)
            .map(Line)
            .filter(|line| {
                grid[*line][Column(0)]
                    .hyperlink()
                    .is_some_and(|hyperlink| hyperlink.id() == PROMPT_MARK)
            })
            .collect()
    }

    pub fn jump_to_prompt(&mut self, older: bool) -> bool {
        let current = self.display_offset() as i32;
        let top = Line(-current);
        let marks = self.prompt_marks();
        let target = match older {
            true => marks.into_iter().rev().find(|line| *line < top),
            false => marks.into_iter().find(|line| *line > top),
        };
        let offset = match target {
            Some(line) => (-line.0).max(0),
            None if older => return false,
            None => 0,
        };
        if offset == current {
            return false;
        }
        self.scroll_display(Scroll::Delta(offset - current));
        true
    }

    pub fn clear_scrollback(&mut self) {
        self.shift = 0.0;
        self.term.clear_screen(ClearMode::Saved);
    }

    fn grid_point(&self, cell: CellPoint) -> GridPoint {
        let column = cell.column.min(self.size.columns - 1);
        let line = cell.line.min(self.size.lines - 1);
        term::viewport_to_point(self.display_offset(), GridPoint::new(line, Column(column)))
    }

    pub fn cursor_cell(&self) -> Option<(usize, usize)> {
        let cursor = self.term.grid().cursor.point;
        term::point_to_viewport(self.display_offset(), cursor)
            .map(|point| (point.column.0, point.line))
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
            self.shift = 0.0;
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

    pub fn link_at(&mut self, cell: CellPoint) -> Option<Link> {
        let point = self.grid_point(cell);
        let first = self.term.line_search_left(point);
        let last = self.term.line_search_right(point);
        if let Some(hyperlink) = self.term.grid()[point]
            .hyperlink()
            .filter(|hyperlink| hyperlink.id() != PROMPT_MARK)
        {
            let term = &self.term;
            let same = |at: GridPoint| term.grid()[at].hyperlink().as_ref() == Some(&hyperlink);
            let mut start = point;
            while start > first && same(start.sub(term, Boundary::None, 1)) {
                start = start.sub(term, Boundary::None, 1);
            }
            let mut end = point;
            while end < last && same(end.add(term, Boundary::None, 1)) {
                end = end.add(term, Boundary::None, 1);
            }
            return Some(Link {
                range: start..=end,
                uri: hyperlink.uri().to_owned(),
            });
        }
        let urls = match &mut self.urls {
            Some(urls) => urls,
            None => self.urls.insert(RegexSearch::new(URL_PATTERN).ok()?),
        };
        let found = RegexIter::new(first, last, Direction::Right, &self.term, urls)
            .find(|candidate| candidate.contains(&point))?;
        let text = self.term.bounds_to_string(*found.start(), *found.end());
        let url = trimmed_url(&text);
        let trimmed = text[url.len()..].chars().count();
        let end = found.end().sub(&self.term, Boundary::None, trimmed);
        (point <= end).then(|| Link {
            range: *found.start()..=end,
            uri: url.to_owned(),
        })
    }

    pub fn path_at(&self, cell: CellPoint, directory: &Path, home: &Path) -> Option<Link> {
        let point = self.grid_point(cell);
        let term = &self.term;
        let first = term.line_search_left(point);
        let last = term.line_search_right(point);
        let part_of_path = |at: GridPoint| !PATH_DELIMITERS.contains(&term.grid()[at].c);
        if !part_of_path(point) {
            return None;
        }
        let mut start = point;
        while start > first && part_of_path(start.sub(term, Boundary::None, 1)) {
            start = start.sub(term, Boundary::None, 1);
        }
        let mut end = point;
        while end < last && part_of_path(end.add(term, Boundary::None, 1)) {
            end = end.add(term, Boundary::None, 1);
        }
        let text = term.bounds_to_string(start, end);
        let typed = written_path(&text);
        let path = match typed.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => directory.join(typed),
        };
        let dropped = text[typed.len()..].chars().count();
        let end = end.sub(term, Boundary::None, dropped);
        (point <= end && path.symlink_metadata().is_ok()).then(|| Link {
            range: start..=end,
            uri: file_uri(&path),
        })
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
