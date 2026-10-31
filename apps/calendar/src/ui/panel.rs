use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_toolkit::ui::{Context, Size, Widget, WidgetId};
use bande_toolkit::{
    Elevation, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
};

const PADDING: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
const LABEL_GAP: f32 = 10.0;
const SEPARATOR_GAP: f32 = 12.0;
const COLUMN_GAP: f32 = 6.0;
const ARROW: f32 = 9.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    Fixed(f32),
    Fill,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub label: Option<String>,
    pub cells: Vec<(usize, Width)>,
    pub height: f32,
    pub separated: bool,
    pub trailing: bool,
}

impl Line {
    pub fn new(label: Option<&str>, cells: Vec<(usize, Width)>, height: f32) -> Self {
        Self {
            label: label.map(str::to_owned),
            cells,
            height,
            separated: false,
            trailing: false,
        }
    }

    pub fn separated(self) -> Self {
        Self {
            separated: true,
            ..self
        }
    }

    pub fn trailing(self) -> Self {
        Self {
            trailing: true,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrow {
    Left(f32),
    Right(f32),
    None,
}

pub struct Panel {
    id: WidgetId,
    title: Option<String>,
    lines: Vec<Line>,
    children: Vec<Box<dyn Widget>>,
    width: f32,
    label_width: f32,
    arrow: Arrow,
    floating: bool,
    dismissed: Rc<Cell<bool>>,
    title_text: Option<Paragraph>,
    labels: Vec<Option<Paragraph>>,
    frame: Rect,
}

impl Panel {
    pub fn new(title: Option<&str>, width: f32, label_width: f32) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.map(str::to_owned),
            lines: Vec::new(),
            children: Vec::new(),
            width,
            label_width,
            arrow: Arrow::None,
            floating: true,
            dismissed: Rc::new(Cell::new(false)),
            title_text: None,
            labels: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn sheet(self) -> Self {
        Self {
            floating: false,
            ..self
        }
    }

    pub fn add(&mut self, widget: impl Widget + 'static) -> usize {
        self.children.push(Box::new(widget));
        self.children.len() - 1
    }

    pub fn id_of(&self, index: usize) -> Option<WidgetId> {
        self.children.get(index).map(|child| child.id())
    }

    pub fn replace(&mut self, old: WidgetId, widget: impl Widget + 'static) -> bool {
        match self.children.iter().position(|child| child.id() == old) {
            Some(index) => {
                self.children[index] = Box::new(widget);
                true
            }
            None => false,
        }
    }

    pub fn line(&mut self, line: Line) {
        self.lines.push(line);
    }

    pub fn set_arrow(&mut self, arrow: Arrow) {
        self.arrow = arrow;
    }

    pub fn dismisser(&self) -> Rc<Cell<bool>> {
        self.dismissed.clone()
    }

    pub fn height(&self) -> f32 {
        let title = match self.title {
            Some(_) => 22.0 + ROW_GAP + 4.0,
            None => 0.0,
        };
        let lines: f32 = self
            .lines
            .iter()
            .map(|line| line.height + ROW_GAP + if line.separated { SEPARATOR_GAP } else { 0.0 })
            .sum();
        (2.0 * PADDING + title + lines - ROW_GAP).ceil()
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    fn lines_top(&self) -> f32 {
        self.frame.y
            + PADDING
            + match self.title_text {
                Some(_) => 22.0 + ROW_GAP + 4.0,
                None => 0.0,
            }
    }
}

impl Widget for Panel {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.title_text = self.title.as_ref().map(|title| {
            cx.text.paragraph(
                title,
                TextStyle::role(TextRole::Heading, cx.palette.ink),
                Some(self.width - 2.0 * PADDING),
                TextAlign::Start,
            )
        });
        let style = TextStyle::role(TextRole::Callout, cx.palette.ink2);
        let width = self.label_width;
        self.labels = self
            .lines
            .iter()
            .map(|line| {
                line.label
                    .as_ref()
                    .map(|label| cx.text.paragraph(label, style, Some(width), TextAlign::End))
            })
            .collect();
        Size::new(self.width, self.height())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut y = self.lines_top();
        let labelled = self.lines.iter().any(|line| line.label.is_some());
        let start = match labelled {
            true => frame.x + PADDING + self.label_width + LABEL_GAP,
            false => frame.x + PADDING,
        };
        let end = frame.x + frame.width - PADDING;
        for line in &self.lines {
            if line.separated {
                y += SEPARATOR_GAP;
            }
            let fixed: f32 = line
                .cells
                .iter()
                .map(|(_, width)| match width {
                    Width::Fixed(width) => *width,
                    Width::Fill => 0.0,
                })
                .sum();
            let fills = line
                .cells
                .iter()
                .filter(|(_, width)| *width == Width::Fill)
                .count();
            let gaps = COLUMN_GAP * line.cells.len().saturating_sub(1) as f32;
            let row_start = match line.label.is_none() && !labelled {
                true => frame.x + PADDING,
                false if line.label.is_none() && line.cells.len() == 1 && fills == 1 => {
                    frame.x + PADDING
                }
                false => start,
            };
            let fill = match fills {
                0 => 0.0,
                count => ((end - row_start - fixed - gaps) / count as f32).max(0.0),
            };
            let total = fixed + fill * fills as f32 + gaps;
            let mut x = match line.trailing {
                true => end - total,
                false => row_start,
            };
            for (index, width) in &line.cells {
                let width = match width {
                    Width::Fixed(width) => *width,
                    Width::Fill => fill,
                };
                if let Some(child) = self.children.get_mut(*index) {
                    child.measure(cx, Size::new(width, line.height));
                    child.place(cx, Rect::new(x, y, width, line.height));
                }
                x += width + COLUMN_GAP;
            }
            y += line.height + ROW_GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if self.floating {
            let shape = Shape::rounded(self.frame, Radius::L);
            items.extend(Item::shadows(
                shape,
                Elevation::Frame,
                cx.palette.scheme,
                true,
            ));
            items.push(Item::Fill {
                shape,
                paint: cx.palette.overlay.into(),
            });
            let arrow = match self.arrow {
                Arrow::Left(y) => Some((self.frame.x, -ARROW, y)),
                Arrow::Right(y) => Some((self.frame.x + self.frame.width, ARROW, y)),
                Arrow::None => None,
            };
            if let Some((x, reach, y)) = arrow {
                let y = y.clamp(
                    self.frame.y + Radius::L + ARROW,
                    self.frame.y + self.frame.height - Radius::L - ARROW,
                );
                items.push(Item::FillPath {
                    path: bande_toolkit::Path::through(&[
                        Point::new(x - reach.signum(), y - ARROW),
                        Point::new(x + reach, y),
                        Point::new(x - reach.signum(), y + ARROW),
                    ])
                    .close(),
                    paint: cx.palette.overlay.into(),
                });
            }
        }
        if let Some(title) = &self.title_text {
            items.push(Item::Text {
                origin: Point::new(self.frame.x + PADDING, self.frame.y + PADDING),
                paragraph: title.clone(),
            });
        }
        let mut y = self.lines_top();
        for (line, label) in self.lines.iter().zip(&self.labels) {
            if line.separated {
                items.push(Item::Line {
                    from: Point::new(
                        self.frame.x + PADDING,
                        y + SEPARATOR_GAP / 2.0 - ROW_GAP / 2.0,
                    ),
                    to: Point::new(
                        self.frame.x + self.frame.width - PADDING,
                        y + SEPARATOR_GAP / 2.0 - ROW_GAP / 2.0,
                    ),
                    width: 1.0,
                    paint: cx.palette.hairline.into(),
                });
                y += SEPARATOR_GAP;
            }
            if let Some(label) = label {
                items.push(Item::Text {
                    origin: Point::new(
                        self.frame.x + PADDING,
                        y + ((line.height - label.height()) / 2.0).round(),
                    ),
                    paragraph: label.clone(),
                });
            }
            y += line.height + ROW_GAP;
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(match self.floating {
            true => Role::Dialog,
            false => Role::AlertDialog,
        });
        if let Some(title) = &self.title {
            node.set_label(title.clone());
        }
        node
    }

    fn dismissed(&self) -> bool {
        self.dismissed.get()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
