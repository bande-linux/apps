use bande_toolkit::{Point, Rect};

pub const DIVIDER: f32 = 1.0;
const DIVIDER_REACH: f32 = 3.0;
const MIN_RATIO: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    Pane(u32),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub path: Vec<Side>,
    pub axis: Axis,
    pub line: Rect,
    pub span: Rect,
}

impl Divider {
    pub fn reach(&self) -> Rect {
        match self.axis {
            Axis::SideBySide => Rect::new(
                self.line.x - DIVIDER_REACH,
                self.line.y,
                self.line.width + 2.0 * DIVIDER_REACH,
                self.line.height,
            ),
            Axis::Stacked => Rect::new(
                self.line.x,
                self.line.y - DIVIDER_REACH,
                self.line.width,
                self.line.height + 2.0 * DIVIDER_REACH,
            ),
        }
    }

    pub fn ratio_at(&self, point: Point) -> f32 {
        let (start, length, at) = match self.axis {
            Axis::SideBySide => (self.span.x, self.span.width, point.x),
            Axis::Stacked => (self.span.y, self.span.height, point.y),
        };
        if length <= DIVIDER {
            return 0.5;
        }
        ((at - start) / (length - DIVIDER)).clamp(MIN_RATIO, 1.0 - MIN_RATIO)
    }
}

fn split_rect(area: Rect, axis: Axis, ratio: f32) -> (Rect, Rect, Rect) {
    match axis {
        Axis::SideBySide => {
            let first = ((area.width - DIVIDER) * ratio).round();
            let second = area.x + first + DIVIDER;
            (
                Rect::new(area.x, area.y, first, area.height),
                Rect::new(area.x + first, area.y, DIVIDER, area.height),
                Rect::new(second, area.y, area.x + area.width - second, area.height),
            )
        }
        Axis::Stacked => {
            let first = ((area.height - DIVIDER) * ratio).round();
            let second = area.y + first + DIVIDER;
            (
                Rect::new(area.x, area.y, area.width, first),
                Rect::new(area.x, area.y + first, area.width, DIVIDER),
                Rect::new(area.x, second, area.width, area.y + area.height - second),
            )
        }
    }
}

impl Layout {
    pub fn panes(&self) -> Vec<u32> {
        match self {
            Self::Pane(id) => vec![*id],
            Self::Split { first, second, .. } => {
                let mut panes = first.panes();
                panes.extend(second.panes());
                panes
            }
        }
    }

    pub fn split(&mut self, pane: u32, axis: Axis, new: u32) -> bool {
        match self {
            Self::Pane(id) if *id == pane => {
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Pane(pane)),
                    second: Box::new(Self::Pane(new)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split(pane, axis, new) || second.split(pane, axis, new)
            }
        }
    }

    pub fn remove(&mut self, pane: u32) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };
        let kept = match (&**first, &**second) {
            (Self::Pane(id), _) if *id == pane => Some((**second).clone()),
            (_, Self::Pane(id)) if *id == pane => Some((**first).clone()),
            _ => None,
        };
        match kept {
            Some(kept) => {
                *self = kept;
                true
            }
            None => first.remove(pane) || second.remove(pane),
        }
    }

    pub fn frames(&self, area: Rect) -> Vec<(u32, Rect)> {
        match self {
            Self::Pane(id) => vec![(*id, area)],
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (one, _, two) = split_rect(area, *axis, *ratio);
                let mut frames = first.frames(one);
                frames.extend(second.frames(two));
                frames
            }
        }
    }

    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut dividers = Vec::new();
        self.collect_dividers(area, &mut Vec::new(), &mut dividers);
        dividers
    }

    fn collect_dividers(&self, area: Rect, path: &mut Vec<Side>, dividers: &mut Vec<Divider>) {
        let Self::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return;
        };
        let (one, line, two) = split_rect(area, *axis, *ratio);
        dividers.push(Divider {
            path: path.clone(),
            axis: *axis,
            line,
            span: area,
        });
        path.push(Side::First);
        first.collect_dividers(one, path, dividers);
        path.pop();
        path.push(Side::Second);
        second.collect_dividers(two, path, dividers);
        path.pop();
    }

    pub fn set_ratio(&mut self, path: &[Side], new: f32) {
        let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        else {
            return;
        };
        match path.split_first() {
            None => *ratio = new.clamp(MIN_RATIO, 1.0 - MIN_RATIO),
            Some((Side::First, rest)) => first.set_ratio(rest, new),
            Some((Side::Second, rest)) => second.set_ratio(rest, new),
        }
    }

    pub fn neighbour(&self, pane: u32, forward: bool) -> u32 {
        let panes = self.panes();
        let count = panes.len();
        panes
            .iter()
            .position(|id| *id == pane)
            .map_or(pane, |index| {
                let next = if forward {
                    (index + 1) % count
                } else {
                    (index + count - 1) % count
                };
                panes[next]
            })
    }
}
