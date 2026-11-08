use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_l10n::tr;

use crate::engine::{Angle, Base, Engine, Input, Mode, WORD_SIZES};
use crate::keys::{self, KeySpec, Tone, binding_label};
use accesskit::{Action, ActionRequest, Node, Role, Toggled};
use bande_toolkit::ui::{Context, Event, Key, Parts, PointerButton, Size, Widget, WidgetId};
use bande_toolkit::{Color, Item, Paint, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

const FLASH: Duration = Duration::from_millis(120);
const GAP: f32 = 1.0;
const PADDING: f32 = 14.0;
const DISPLAY_SHARE: f32 = 0.28;
const DISPLAY_MIN: f32 = 96.0;
const PROGRAMMER_DISPLAY: f32 = 196.0;
const VALUE_SIZE: f32 = 52.0;
const SMALLEST_VALUE: f32 = 14.0;
const LINE_SIZE: f32 = 15.0;
const CHIP_SIZE: f32 = 11.0;
const CHIP_HEIGHT: f32 = 20.0;
const CHIP_PADDING: f32 = 8.0;
const BIT_SIZE: f32 = 12.0;
const BIT_ROW: f32 = 30.0;
const NIBBLE_GAP: f32 = 6.0;
const INDEX_SIZE: f32 = 9.0;
const KEY_TEXT_SHARE: f32 = 0.42;
const LARGEST_KEY_TEXT: f32 = 24.0;
const FUNCTION_TEXT_SHARE: f32 = 0.75;
const PRESSED_MIX: f32 = 0.22;
const STACK_LINES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Key(usize),
    Base(Base),
    Word(u32),
    Bit(u32),
}

struct Placed {
    target: Target,
    rect: Rect,
    label: String,
    toggled: Option<bool>,
}

pub struct Keypad {
    id: WidgetId,
    engine: Rc<RefCell<Engine>>,
    frame: Rect,
    keys: Vec<KeySpec>,
    placed: Vec<Placed>,
    display: String,
    items: Vec<Item>,
    pressed: Option<Target>,
    inside: bool,
    flash: Option<(Target, Instant)>,
}

fn text_size(paragraph: &Paragraph) -> (f32, f32) {
    (paragraph.width(), paragraph.height())
}

fn centered(paragraph: Paragraph, rect: Rect, items: &mut Vec<Item>) {
    let (width, height) = text_size(&paragraph);
    items.push(Item::Text {
        origin: Point::new(
            (rect.x + (rect.width - width) / 2.0).round(),
            (rect.y + (rect.height - height) / 2.0).round(),
        ),
        paragraph,
    });
}

fn fill(rect: Rect, color: Color, items: &mut Vec<Item>) {
    items.push(Item::Fill {
        shape: Shape::rect(rect),
        paint: Paint::Solid(color),
    });
}

fn rounded(rect: Rect, color: Color, items: &mut Vec<Item>) {
    items.push(Item::Fill {
        shape: Shape::rounded(rect, rect.height / 2.0),
        paint: Paint::Solid(color),
    });
}

fn fitted(cx: &mut Context, text: &str, size: f32, color: Color, width: f32) -> Paragraph {
    let style = |size: f32| TextStyle::new(size, color).tabular();
    let paragraph = cx.text.paragraph(text, style(size), None, TextAlign::Start);
    if paragraph.width() <= width || paragraph.width() <= 0.0 {
        return paragraph;
    }
    let smaller = (size * width / paragraph.width())
        .floor()
        .max(SMALLEST_VALUE);
    cx.text.fitted(text, style(smaller), width)
}

impl Keypad {
    pub fn new(engine: Rc<RefCell<Engine>>) -> Self {
        Self {
            id: WidgetId::next(),
            engine,
            frame: Rect::default(),
            keys: Vec::new(),
            placed: Vec::new(),
            display: String::new(),
            items: Vec::new(),
            pressed: None,
            inside: false,
            flash: None,
        }
    }

    fn hit(&self, point: Point) -> Option<Target> {
        self.placed
            .iter()
            .find(|placed| placed.rect.contains(point))
            .map(|placed| placed.target)
    }

    fn activate(&mut self, target: Target) {
        let input = match target {
            Target::Key(index) => self.keys.get(index).map(|spec| spec.input),
            Target::Base(base) => Some(Input::Base(base)),
            Target::Word(bits) => Some(Input::Word(bits)),
            Target::Bit(bit) => {
                self.engine.borrow_mut().toggle_bit(bit);
                None
            }
        };
        if let Some(input) = input {
            self.engine.borrow_mut().press(input);
        }
    }

    fn type_input(&mut self, cx: &Context, input: Input) {
        let index = self.keys.iter().position(|spec| spec.input == input);
        self.engine.borrow_mut().press(input);
        if let Some(index) = index {
            self.flash = Some((Target::Key(index), cx.now() + FLASH));
        }
    }

    fn lit(&self, cx: &Context, target: Target) -> bool {
        let pressed = self.pressed == Some(target) && self.inside;
        let flashing = self
            .flash
            .is_some_and(|(flashed, until)| flashed == target && cx.now() < until);
        if flashing && let Some((_, until)) = self.flash {
            cx.wake_at(until);
        }
        pressed || flashing
    }

    fn lay_out(&mut self, cx: &mut Context) {
        let engine = self.engine.borrow();
        let layout = keys::layout(&engine);
        let palette = cx.palette;
        let frame = self.frame;
        let display_height = match engine.mode {
            Mode::Programmer => PROGRAMMER_DISPLAY,
            _ => (frame.height * DISPLAY_SHARE).max(DISPLAY_MIN),
        }
        .min(frame.height);
        let display = Rect::new(frame.x, frame.y, frame.width, display_height);
        let mut items = Vec::new();
        let mut placed = Vec::new();
        fill(frame, palette.hairline, &mut items);
        fill(display, palette.chassis, &mut items);
        let inner = display.inset(PADDING);
        let (top, bottom) = match engine.mode {
            Mode::Programmer => (
                inner.y + CHIP_HEIGHT,
                self.lay_out_programmer(cx, &engine, inner, &mut items, &mut placed),
            ),
            _ => (display.y, inner.y + inner.height),
        };
        self.display = match engine.error() {
            Some(crate::decimal::Failure::Overflow) => tr!("Overflow"),
            Some(crate::decimal::Failure::Undefined) => tr!("Not a number"),
            None => engine.display(),
        };
        let value = fitted(
            cx,
            &self.display,
            VALUE_SIZE.min((bottom - top) * 0.5).max(SMALLEST_VALUE),
            palette.ink,
            inner.width,
        );
        let (width, height) = text_size(&value);
        let value_top = bottom - height;
        items.push(Item::Text {
            origin: Point::new(inner.x + inner.width - width, value_top),
            paragraph: value,
        });
        let mut lines: Vec<String> = match engine.rpn {
            true => engine.stack().into_iter().rev().take(STACK_LINES).collect(),
            false => vec![engine.expression()],
        };
        lines.retain(|line| !line.is_empty());
        let mut line_bottom = value_top;
        for line in lines {
            let paragraph = cx.text.fitted(
                &line,
                TextStyle::new(LINE_SIZE, palette.ink2).tabular(),
                inner.width,
            );
            let (width, height) = text_size(&paragraph);
            if line_bottom - height < top {
                break;
            }
            line_bottom -= height;
            items.push(Item::Text {
                origin: Point::new(inner.x + inner.width - width, line_bottom),
                paragraph,
            });
        }
        let mut badges = Vec::new();
        if engine.mode == Mode::Scientific && engine.angle == Angle::Radians {
            badges.push(tr!("Rad"));
        }
        if engine.has_memory() && engine.mode != Mode::Programmer {
            badges.push("M".to_owned());
        }
        if engine.rpn {
            badges.push(tr!("RPN"));
        }
        if !badges.is_empty() && engine.mode != Mode::Programmer {
            let paragraph = cx.text.paragraph(
                &badges.join("  "),
                TextStyle::new(CHIP_SIZE, palette.ink2).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            );
            items.push(Item::Text {
                origin: Point::new(inner.x, display.y + PADDING / 2.0),
                paragraph,
            });
        }
        let pending = engine.pending();
        let second = engine.second;
        let rows = layout.rows.len().max(1) as f32;
        let columns = layout.columns.max(1) as f32;
        let area = Rect::new(
            frame.x,
            display.y + display.height + GAP,
            frame.width,
            (frame.height - display.height - GAP).max(0.0),
        );
        let cell_width = (area.width + GAP) / columns;
        let cell_height = (area.height + GAP) / rows;
        let digit_text = (cell_height * KEY_TEXT_SHARE).min(LARGEST_KEY_TEXT);
        let mut keys = Vec::new();
        for (row_index, row) in layout.rows.iter().enumerate() {
            let mut column = 0usize;
            for spec in row {
                let x = area.x + (column as f32 * cell_width).round();
                let right = area.x + ((column + spec.span) as f32 * cell_width).round() - GAP;
                let y = area.y + (row_index as f32 * cell_height).round();
                let bottom = area.y + ((row_index + 1) as f32 * cell_height).round() - GAP;
                let rect = Rect::new(x, y, right - x, bottom - y);
                column += spec.span;
                let index = keys.len();
                keys.push(spec.clone());
                let target = Target::Key(index);
                let selected = (pending.is_some()
                    && matches!(spec.input, Input::Binary(operator) if Some(operator) == pending))
                    || (second && spec.input == Input::Second);
                let (background, ink) = match (spec.tone, selected) {
                    (Tone::Operator, true) => (palette.on_accent, palette.accent),
                    (Tone::Operator, false) => (palette.accent, palette.on_accent),
                    (_, true) => (palette.ink2, palette.sheet),
                    (Tone::Digit, false) => (palette.raised, palette.ink),
                    (Tone::Function, false) => (palette.well, palette.ink),
                };
                let background = match self.lit(cx, target) {
                    true => background.mix(palette.ink, PRESSED_MIX),
                    false => background,
                };
                fill(rect, background, &mut items);
                let size = match spec.tone == Tone::Function || spec.label.chars().count() > 2 {
                    true => digit_text * FUNCTION_TEXT_SHARE,
                    false => digit_text,
                };
                let label = cx.text.fitted(
                    &spec.label,
                    TextStyle::new(size, ink),
                    (rect.width - 4.0).max(0.0),
                );
                centered(label, rect, &mut items);
                placed.push(Placed {
                    target,
                    rect,
                    label: spec.name.clone(),
                    toggled: None,
                });
            }
        }
        drop(engine);
        self.keys = keys;
        self.placed = placed;
        self.items = items;
    }

    fn lay_out_programmer(
        &self,
        cx: &mut Context,
        engine: &Engine,
        inner: Rect,
        items: &mut Vec<Item>,
        placed: &mut Vec<Placed>,
    ) -> f32 {
        let palette = cx.palette;
        let mut chip = |cx: &mut Context, x: f32, label: &str, selected: bool, target: Target| {
            let paragraph = cx.text.paragraph(
                label,
                TextStyle::new(
                    CHIP_SIZE,
                    if selected {
                        palette.on_accent
                    } else {
                        palette.ink2
                    },
                )
                .weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            );
            let width = paragraph.width().ceil() + 2.0 * CHIP_PADDING;
            let rect = Rect::new(x, inner.y, width, CHIP_HEIGHT);
            let lit = self.lit(cx, target);
            match (selected, lit) {
                (true, _) => rounded(rect, palette.accent, items),
                (false, true) => rounded(rect, palette.well, items),
                (false, false) => {}
            }
            centered(paragraph, rect, items);
            placed.push(Placed {
                target,
                rect,
                label: label.to_owned(),
                toggled: Some(selected),
            });
            width
        };
        let mut x = inner.x;
        for base in Base::ALL {
            x += chip(
                cx,
                x,
                &keys::base_label(base),
                engine.base == base,
                Target::Base(base),
            );
        }
        let word_widths: Vec<f32> = WORD_SIZES
            .iter()
            .map(|bits| {
                cx.text
                    .paragraph(
                        &bits.to_string(),
                        TextStyle::new(CHIP_SIZE, palette.ink2).weight(TextStyle::SEMIBOLD),
                        None,
                        TextAlign::Start,
                    )
                    .width()
                    .ceil()
                    + 2.0 * CHIP_PADDING
            })
            .collect();
        let mut x = inner.x + inner.width - word_widths.iter().sum::<f32>();
        for bits in WORD_SIZES {
            x += chip(
                cx,
                x,
                &bits.to_string(),
                engine.bits == bits,
                Target::Word(bits),
            );
        }
        let word = engine.word().unwrap_or(0);
        let grid_top = inner.y + inner.height - 2.0 * BIT_ROW;
        let nibble_width = (inner.width - 7.0 * NIBBLE_GAP) / 8.0;
        let bit_width = nibble_width / 4.0;
        for row in 0..2u32 {
            let top = grid_top + row as f32 * BIT_ROW;
            for column in 0..32u32 {
                let bit = 63 - row * 32 - column;
                let x = inner.x
                    + (column / 4) as f32 * (nibble_width + NIBBLE_GAP)
                    + (column % 4) as f32 * bit_width;
                let rect = Rect::new(x, top, bit_width, BIT_SIZE + 4.0);
                let active = bit < engine.bits;
                let set = word >> bit & 1 == 1;
                let ink = match (active, set) {
                    (false, _) => palette.ink3.with_opacity(0.5),
                    (true, true) => palette.accent_text,
                    (true, false) => palette.ink2,
                };
                let paragraph = cx.text.paragraph(
                    if set { "1" } else { "0" },
                    TextStyle::new(BIT_SIZE, ink).monospace(),
                    None,
                    TextAlign::Start,
                );
                if self.lit(cx, Target::Bit(bit)) {
                    rounded(rect, palette.well, items);
                }
                centered(paragraph, rect, items);
                if active {
                    placed.push(Placed {
                        target: Target::Bit(bit),
                        rect,
                        label: tr!("Bit {bit}", bit = bit.to_string()),
                        toggled: Some(set),
                    });
                }
                if column % 4 == 0 || column == 31 {
                    let label = cx.text.paragraph(
                        &bit.to_string(),
                        TextStyle::new(INDEX_SIZE, palette.ink3),
                        None,
                        TextAlign::Start,
                    );
                    if column % 8 == 0 || column == 31 {
                        let width = label.width();
                        let left = match column {
                            31 => x + bit_width - width,
                            _ => x,
                        };
                        items.push(Item::Text {
                            origin: Point::new(left.round(), top + BIT_SIZE + 4.0),
                            paragraph: label,
                        });
                    }
                }
            }
        }
        grid_top - PADDING / 2.0
    }
}

impl Widget for Keypad {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.lay_out(cx);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.extend(self.items.iter().cloned());
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(tr!("Calculator"));
        node
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.button == PointerButton::Primary => {
                self.pressed = self.hit(pointer.position);
                self.inside = self.pressed.is_some();
                self.pressed.is_some()
            }
            Event::PointerMove(pointer) => {
                let Some(pressed) = self.pressed else {
                    return false;
                };
                let inside = self.hit(pointer.position) == Some(pressed);
                std::mem::replace(&mut self.inside, inside) != inside
            }
            Event::PointerUp(_) => {
                let Some(pressed) = self.pressed.take() else {
                    return false;
                };
                if std::mem::take(&mut self.inside) {
                    self.activate(pressed);
                }
                true
            }
            Event::PointerLeave => {
                self.inside = false;
                self.pressed.take().is_some()
            }
            Event::Key {
                key: Key::Character('c'),
                modifiers,
            } if modifiers.control && !modifiers.shift && !modifiers.alt => {
                cx.copy(&self.engine.borrow().copy_text());
                true
            }
            Event::Key { .. } => {
                let input = {
                    let engine = self.engine.borrow();
                    let layout = keys::layout(&engine);
                    keys::typed(&engine, &layout, event)
                };
                match input {
                    Some(input) => {
                        self.type_input(cx, input);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let bounds = |rect: Rect| accesskit::Rect {
            x0: f64::from(rect.x),
            y0: f64::from(rect.y),
            x1: f64::from(rect.x + rect.width),
            y1: f64::from(rect.y + rect.height),
        };
        let mut display = Node::new(Role::Label);
        display.set_label(tr!("Result"));
        display.set_value(self.display.replace('−', "-"));
        let mut nodes = vec![(self.id.part(0), display)];
        for (index, placed) in self.placed.iter().enumerate() {
            let mut node = Node::new(Role::Button);
            node.set_label(placed.label.clone());
            if let Target::Key(key) = placed.target
                && let Some(spec) = self.keys.get(key)
            {
                node.set_description(binding_label(spec.binding));
            }
            if let Some(toggled) = placed.toggled {
                node.set_toggled(match toggled {
                    true => Toggled::True,
                    false => Toggled::False,
                });
            }
            node.add_action(Action::Click);
            node.set_bounds(bounds(placed.rect));
            nodes.push((self.id.part(index as u64 + 1), node));
        }
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            focus: None,
            nodes,
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if request.action != Action::Click || part == 0 {
            return false;
        }
        let Some(target) = self
            .placed
            .get(part as usize - 1)
            .map(|placed| placed.target)
        else {
            return false;
        };
        self.activate(target);
        true
    }
}
