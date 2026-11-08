use accesskit::{Action, ActionRequest, Node, Role};
use bande_l10n::tr;
use bande_toolkit::ui::{
    Context, Cursor, Event, Key, Modifiers, Parts, SearchField, Size, Widget, WidgetId, access_rect,
};
use bande_toolkit::{
    Color, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
};

const WIDTH: f32 = 520.0;
const PADDING: f32 = 12.0;
const ROW: f32 = 30.0;
const HEADING: f32 = 26.0;
const DOT: f32 = 7.0;
const DOT_GAP: f32 = 10.0;
const ACTION_GAP: f32 = 14.0;
const SELECTION_INSET: f32 = 6.0;
const CONTAINER_LIMIT: usize = 6;
const OTHER_LIMIT: usize = 4;
const ACTION_PARTS: u64 = 1 << 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Reveal(String),
    Run(String, String),
    Copy(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Containers,
    Images,
    Volumes,
}

impl Group {
    fn title(self) -> String {
        match self {
            Self::Containers => tr!("Containers"),
            Self::Images => tr!(context = "containers", "Images"),
            Self::Volumes => tr!("Volumes"),
        }
    }

    fn limit(self) -> usize {
        match self {
            Self::Containers => CONTAINER_LIMIT,
            Self::Images | Self::Volumes => OTHER_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub group: Group,
    pub title: String,
    pub running: Option<bool>,
    pub primary: Choice,
    pub actions: Vec<(String, Choice)>,
}

enum Line {
    Heading(Paragraph),
    Hit {
        hit: usize,
        title: Paragraph,
        actions: Vec<(Paragraph, Rect)>,
    },
}

pub struct CommandPalette {
    field: SearchField,
    query: std::rc::Rc<std::cell::RefCell<String>>,
    shown: Option<(String, Color, u32)>,
    hits: Vec<Hit>,
    lines: Vec<Line>,
    cursor: Option<usize>,
    pressed: Option<(usize, Option<usize>)>,
    on_choice: Box<dyn FnMut(Choice)>,
    done: bool,
    frame: Rect,
}

impl CommandPalette {
    pub fn new(hits: Vec<Hit>, on_choice: impl FnMut(Choice) + 'static) -> Self {
        let query = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let typed = std::rc::Rc::clone(&query);
        Self {
            field: SearchField::new(
                &tr!("Search containers, images, volumes…"),
                "",
                move |text| text.clone_into(&mut typed.borrow_mut()),
            ),
            query,
            shown: None,
            hits,
            lines: Vec::new(),
            cursor: None,
            pressed: None,
            on_choice: Box::new(on_choice),
            done: false,
            frame: Rect::default(),
        }
    }

    fn matches(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        let mut counts = [0usize; 3];
        self.hits
            .iter()
            .enumerate()
            .filter(|(_, hit)| !query.is_empty() || hit.group == Group::Containers)
            .filter(|(_, hit)| query.is_empty() || hit.title.to_lowercase().contains(&query))
            .filter(|(_, hit)| {
                let count = &mut counts[hit.group as usize];
                *count += 1;
                *count <= hit.group.limit()
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn rebuild(&mut self, cx: &mut Context) {
        let query = self.query.borrow().clone();
        let palette = cx.palette;
        let key = Some((query.clone(), palette.ink, cx.text.scale().to_bits()));
        if self.shown == key {
            return;
        }
        self.shown = key;
        let heading = TextStyle::role(TextRole::Label, palette.ink3);
        let title = TextStyle::role(TextRole::Body, palette.ink);
        let action = TextStyle::role(TextRole::Callout, palette.accent_text);
        let mut lines = Vec::new();
        let mut group = None;
        for index in self.matches(&query) {
            let hit = &self.hits[index];
            if group != Some(hit.group) {
                group = Some(hit.group);
                lines.push(Line::Heading(cx.text.paragraph(
                    &hit.group.title(),
                    heading,
                    None,
                    TextAlign::Start,
                )));
            }
            lines.push(Line::Hit {
                hit: index,
                title: cx.text.paragraph(&hit.title, title, None, TextAlign::Start),
                actions: hit
                    .actions
                    .iter()
                    .map(|(label, _)| {
                        (
                            cx.text.paragraph(label, action, None, TextAlign::Start),
                            Rect::default(),
                        )
                    })
                    .collect(),
            });
        }
        self.lines = lines;
        self.cursor = self
            .lines
            .iter()
            .position(|line| matches!(line, Line::Hit { .. }));
    }

    fn list_top(&self) -> f32 {
        let field = self.field.frame();
        field.y + field.height + PADDING
    }

    fn line_frames(&self) -> Vec<Rect> {
        let mut y = self.list_top();
        self.lines
            .iter()
            .map(|line| {
                let height = match line {
                    Line::Heading(_) => HEADING,
                    Line::Hit { .. } => ROW,
                };
                let frame = Rect::new(self.frame.x, y, self.frame.width, height);
                y += height;
                frame
            })
            .collect()
    }

    fn list_height(&self) -> f32 {
        self.lines
            .iter()
            .map(|line| match line {
                Line::Heading(_) => HEADING,
                Line::Hit { .. } => ROW,
            })
            .sum()
    }

    fn at(&self, point: Point) -> Option<(usize, Option<usize>)> {
        let frames = self.line_frames();
        let index = frames.iter().position(|frame| frame.contains(point))?;
        let Line::Hit { actions, .. } = &self.lines[index] else {
            return None;
        };
        let action = actions.iter().position(|(_, rect)| rect.contains(point));
        Some((index, action))
    }

    fn choice(&self, line: usize, action: Option<usize>) -> Option<Choice> {
        let Some(Line::Hit { hit, .. }) = self.lines.get(line) else {
            return None;
        };
        let hit = self.hits.get(*hit)?;
        match action {
            Some(action) => hit.actions.get(action).map(|(_, choice)| choice.clone()),
            None => Some(hit.primary.clone()),
        }
    }

    fn choose(&mut self, line: usize, action: Option<usize>) -> bool {
        let Some(choice) = self.choice(line, action) else {
            return false;
        };
        self.done = true;
        (self.on_choice)(choice);
        true
    }

    fn step(&mut self, forward: bool) -> bool {
        let hits: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line, Line::Hit { .. }))
            .map(|(index, _)| index)
            .collect();
        let at = self
            .cursor
            .and_then(|cursor| hits.iter().position(|index| *index == cursor));
        let next = match (at, forward) {
            (None, _) => hits.first(),
            (Some(at), true) => hits.get(at + 1).or(hits.last()),
            (Some(at), false) => hits.get(at.saturating_sub(1)),
        };
        let moved = next.copied() != self.cursor;
        self.cursor = next.copied().or(self.cursor);
        moved
    }
}

impl Widget for CommandPalette {
    fn id(&self) -> WidgetId {
        self.field.id()
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.rebuild(cx);
        let field = self
            .field
            .measure(cx, Size::new(WIDTH - 2.0 * PADDING, 0.0));
        let list = self.list_height();
        let gap = match list > 0.0 {
            true => PADDING,
            false => 0.0,
        };
        Size::new(WIDTH, 2.0 * PADDING + field.height + gap + list)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let field = self
            .field
            .measure(cx, Size::new(WIDTH - 2.0 * PADDING, 0.0));
        self.field.place(
            cx,
            Rect::new(
                frame.x + PADDING,
                frame.y + PADDING,
                frame.width - 2.0 * PADDING,
                field.height,
            ),
        );
        let frames = self.line_frames();
        for (line, frame) in self.lines.iter_mut().zip(frames) {
            let Line::Hit { actions, .. } = line else {
                continue;
            };
            let mut right = frame.x + frame.width - PADDING - SELECTION_INSET;
            for (paragraph, rect) in actions.iter_mut().rev() {
                let width = paragraph.width();
                *rect = Rect::new(right - width, frame.y, width, frame.height);
                right -= width + ACTION_GAP;
            }
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.field.paint(cx, items);
        let palette = &cx.palette;
        for (index, (line, frame)) in self.lines.iter().zip(self.line_frames()).enumerate() {
            match line {
                Line::Heading(paragraph) => items.push(Item::Text {
                    origin: Point::new(
                        frame.x + PADDING + SELECTION_INSET,
                        frame.y + frame.height - paragraph.height() - 4.0,
                    ),
                    paragraph: paragraph.clone(),
                }),
                Line::Hit {
                    hit,
                    title,
                    actions,
                } => {
                    let lit = self.cursor == Some(index);
                    if lit {
                        items.push(Item::Fill {
                            shape: Shape::rounded(
                                Rect::new(
                                    frame.x + PADDING,
                                    frame.y + 1.0,
                                    frame.width - 2.0 * PADDING,
                                    frame.height - 2.0,
                                ),
                                Radius::S,
                            ),
                            paint: palette.well.into(),
                        });
                    }
                    let mut x = frame.x + PADDING + SELECTION_INSET;
                    if let Some(running) = self.hits.get(*hit).and_then(|hit| hit.running) {
                        let color = match running {
                            true => palette.success,
                            false => palette.ink3,
                        };
                        items.push(Item::Fill {
                            shape: Shape::rounded(
                                Rect::new(
                                    x,
                                    frame.y + ((frame.height - DOT) / 2.0).round(),
                                    DOT,
                                    DOT,
                                ),
                                DOT / 2.0,
                            ),
                            paint: color.into(),
                        });
                        x += DOT + DOT_GAP;
                    }
                    let room = actions
                        .first()
                        .map_or(frame.x + frame.width - PADDING, |(_, rect)| rect.x)
                        - ACTION_GAP
                        - x;
                    items.push(Item::Clip {
                        shape: Shape::rect(Rect::new(x, frame.y, room.max(0.0), frame.height)),
                        items: vec![Item::Text {
                            origin: Point::new(
                                x,
                                frame.y + ((frame.height - title.height()) / 2.0).round(),
                            ),
                            paragraph: title.clone(),
                        }],
                    });
                    for (paragraph, rect) in actions {
                        items.push(Item::Text {
                            origin: Point::new(
                                rect.x,
                                rect.y + ((rect.height - paragraph.height()) / 2.0).round(),
                            ),
                            paragraph: paragraph.clone(),
                        });
                    }
                }
            }
        }
    }

    fn access(&self, cx: &Context) -> Node {
        self.field.access(cx)
    }

    fn parts(&self, cx: &Context) -> Parts {
        let mut parts = self.field.parts(cx);
        for (index, (line, frame)) in self.lines.iter().zip(self.line_frames()).enumerate() {
            let Line::Hit { hit, actions, .. } = line else {
                continue;
            };
            let Some(hit) = self.hits.get(*hit) else {
                continue;
            };
            let id = self.id().part(index as u64 + 1);
            let mut node = Node::new(Role::ListBoxOption);
            node.set_label(hit.title.as_str());
            node.set_bounds(access_rect(frame));
            node.add_action(Action::Click);
            if self.cursor == Some(index) {
                node.set_selected(true);
            }
            let mut children = Vec::new();
            for (action, ((label, _), (_, rect))) in hit.actions.iter().zip(actions).enumerate() {
                let action_id = self
                    .id()
                    .part(ACTION_PARTS * (index as u64 + 1) + action as u64);
                let mut button = Node::new(Role::Button);
                button.set_label(label.as_str());
                button.set_bounds(access_rect(*rect));
                button.add_action(Action::Click);
                children.push(action_id);
                parts.nodes.push((action_id, button));
            }
            node.set_children(children);
            parts.children.push(id);
            parts.nodes.push((id, node));
        }
        parts
    }

    fn perform_part(&mut self, cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return self.field.perform_part(cx, part, request);
        }
        match part {
            part if part >= ACTION_PARTS => {
                let line = (part / ACTION_PARTS - 1) as usize;
                self.choose(line, Some((part % ACTION_PARTS) as usize))
            }
            part if part >= 1 && ((part - 1) as usize) < self.lines.len() => {
                self.choose((part - 1) as usize, None)
            }
            _ => self.field.perform_part(cx, part, request),
        }
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let list = |point: Point| point.y >= self.list_top();
        match event {
            Event::Key { key, modifiers } if *modifiers == Modifiers::default() => match key {
                Key::Escape => {
                    self.done = true;
                    true
                }
                Key::Up => self.step(false),
                Key::Down => self.step(true),
                Key::Enter => self.cursor.is_some_and(|cursor| self.choose(cursor, None)),
                _ => self.field.handle(cx, event),
            },
            Event::PointerMove(pointer) if list(pointer.position) => {
                let line = self.at(pointer.position).map(|(line, _)| line);
                let moved = line.is_some() && line != self.cursor;
                if moved {
                    self.cursor = line;
                }
                moved
            }
            Event::PointerDown(pointer) if list(pointer.position) => {
                self.pressed = self.at(pointer.position).filter(|_| pointer.is_primary());
                true
            }
            Event::PointerUp(pointer) if self.pressed.is_some() => {
                let pressed = self.pressed.take();
                match pressed.filter(|pressed| self.at(pointer.position) == Some(*pressed)) {
                    Some((line, action)) => self.choose(line, action),
                    None => true,
                }
            }
            _ => self.field.handle(cx, event),
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        self.field.perform(cx, request)
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        self.field.accepts_focus(cx)
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        match self.at(point) {
            Some((_, Some(_))) => Some(Cursor::Pointer),
            _ if point.y < self.list_top() => self.field.cursor(point),
            _ => None,
        }
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.field.text_input_area()
    }

    fn dismissed(&self) -> bool {
        self.done
    }
}
