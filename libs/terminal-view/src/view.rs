use std::cell::RefCell;

use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Event, Key, Modifiers, Size, Widget, WidgetId};
use bande_toolkit::{Appearance, Item, Rect, Shape};

use crate::keys::{Command, bracketed, encode_key, shortcut, wants_scroll_as_arrows};
use crate::render::{DEFAULT_FONT_SIZE, Painter};
use crate::terminal::{GridSize, Highlights, SelectionKind, Terminal};
use crate::theme::Profile;

const WHEEL_LINE: f32 = 15.0;
const INITIAL_GRID: GridSize = GridSize {
    columns: 80,
    lines: 24,
};

type InputSink = Box<dyn FnMut(&[u8])>;

pub struct TerminalView {
    id: WidgetId,
    label: String,
    painter: RefCell<Painter>,
    terminal: RefCell<Terminal>,
    frame: Rect,
    scroll: f32,
    selecting: bool,
    on_input: InputSink,
    on_resize: Box<dyn FnMut(GridSize)>,
}

impl TerminalView {
    pub fn new(
        label: &str,
        appearance: Appearance,
        on_input: impl FnMut(&[u8]) + 'static,
        on_resize: impl FnMut(GridSize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            painter: RefCell::new(Painter::new(
                appearance,
                Profile::default(),
                DEFAULT_FONT_SIZE,
                1.0,
            )),
            terminal: RefCell::new(Terminal::new(INITIAL_GRID)),
            frame: Rect::default(),
            scroll: 0.0,
            selecting: false,
            on_input: Box::new(on_input),
            on_resize: Box::new(on_resize),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let terminal = self.terminal.get_mut();
        terminal.feed(bytes);
        terminal.take_events();
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.painter.get_mut().set_appearance(appearance);
    }

    fn command(&mut self, cx: &mut Context, command: Command) -> bool {
        match command {
            Command::Copy => {
                if let Some(text) = self.terminal.get_mut().selected_text() {
                    cx.copy(&text);
                }
                true
            }
            Command::Paste => {
                cx.request_paste();
                true
            }
            Command::SelectAll => {
                self.terminal.get_mut().select_all();
                true
            }
            _ => false,
        }
    }

    fn key(&mut self, cx: &mut Context, key: Key, modifiers: Modifiers) -> bool {
        if let Some(command) = shortcut(key, modifiers) {
            return self.command(cx, command);
        }
        let terminal = self.terminal.get_mut();
        let Some(bytes) = encode_key(key, modifiers, terminal.mode()) else {
            return false;
        };
        terminal.scroll_to_bottom();
        terminal.clear_selection();
        (self.on_input)(&bytes);
        true
    }

    fn scroll(&mut self, delta: f32) {
        self.scroll += delta;
        let lines = (self.scroll / WHEEL_LINE).trunc();
        if lines == 0.0 {
            return;
        }
        self.scroll -= lines * WHEEL_LINE;
        let terminal = self.terminal.get_mut();
        if wants_scroll_as_arrows(terminal.mode()) {
            let key = match lines > 0.0 {
                true => Key::Down,
                false => Key::Up,
            };
            let mode = terminal.mode();
            if let Some(bytes) = encode_key(key, Modifiers::default(), mode) {
                for _ in 0..lines.abs() as usize {
                    (self.on_input)(&bytes);
                }
            }
        } else {
            terminal.scroll_lines(-(lines as i32));
        }
    }
}

impl Widget for TerminalView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let painter = self.painter.get_mut();
        let scale = cx.display_scale;
        let size = painter.font_size();
        painter.set_metrics(size, scale);
        let grid = painter.metrics.grid_for(frame.width, frame.height);
        let cell = painter.metrics.pixel_cell_size();
        if self.terminal.get_mut().resize(grid, cell) {
            (self.on_resize)(grid);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        let mut painter = self.painter.borrow_mut();
        items.push(Item::Fill {
            shape: Shape::rect(self.frame),
            paint: painter.theme.background.into(),
        });
        let mut terminal = self.terminal.borrow_mut();
        let highlights = Highlights::default();
        let origin = bande_toolkit::Point::new(self.frame.x, self.frame.y);
        let drawn = painter.terminal(&terminal, origin, &highlights);
        terminal.take_events();
        items.push(Item::Clip {
            shape: Shape::rect(self.frame),
            items: drawn,
        });
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Terminal);
        node.set_label(self.label.clone());
        node.set_value(self.terminal.borrow().screen_text().join("\n"));
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let inside = |view: &Self, point: bande_toolkit::Point| view.frame.contains(point);
        let cell = |view: &Self, point: bande_toolkit::Point| {
            let local = bande_toolkit::Point::new(point.x - view.frame.x, point.y - view.frame.y);
            view.painter.borrow().metrics.cell_at(local)
        };
        match event {
            Event::Key { key, modifiers } => self.key(cx, *key, *modifiers),
            Event::Commit(text) => {
                let bytes = bracketed(text, self.terminal.get_mut().mode());
                (self.on_input)(&bytes);
                true
            }
            Event::PointerDown(pointer) if inside(self, pointer.position) => {
                let at = cell(self, pointer.position);
                let terminal = self.terminal.get_mut();
                terminal.clear_selection();
                terminal.start_selection(at, SelectionKind::for_clicks(pointer.count, false));
                self.selecting = true;
                true
            }
            Event::PointerMove(pointer) if self.selecting => {
                let at = cell(self, pointer.position);
                self.terminal.get_mut().extend_selection(at);
                true
            }
            Event::PointerUp(_) if self.selecting => {
                self.selecting = false;
                self.terminal.get_mut().finish_selection();
                true
            }
            Event::Scroll(scroll) if inside(self, scroll.position) => {
                self.scroll(scroll.delta.y);
                true
            }
            _ => false,
        }
    }
}
