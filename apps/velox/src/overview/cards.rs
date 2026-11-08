use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{Context, Event, Size, Widget, WidgetId};
use bande_toolkit::{
    Color, Image, Item, Palette, Paragraph, Path, Point, Radius, Rect, Scheme, Shape, TextAlign,
    TextRole, TextStyle,
};

const PADDING: f32 = 14.0;
const GAP: f32 = 4.0;
const ICON: f32 = 14.0;
const ICON_GAP: f32 = 6.0;
const STAT_HEIGHT: f32 = 96.0;
const ENGINE_HEIGHT: f32 = 74.0;
const DISC: f32 = 46.0;
const DISC_ICON: f32 = 22.0;
const LIVE_HEIGHT: f32 = 104.0;
const BAR_HEIGHT: f32 = 8.0;
const BAR_GAP: f32 = 1.0;
const SEGMENT_MIN: f32 = 2.0;
const LEGEND_DOT: f32 = 7.0;
const LEGEND_GAP: f32 = 14.0;
const AREA_OPACITY: f32 = 0.22;
const STROKE: f32 = 1.5;
const TINT_OPACITY: f32 = 0.18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Green,
    Blue,
    Orange,
    Purple,
    Red,
}

impl Tone {
    pub fn color(self, palette: &Palette) -> Color {
        let (lightness, chroma) = match palette.scheme {
            Scheme::Light => (0.58, 0.16),
            Scheme::Dark => (0.72, 0.15),
        };
        match self {
            Self::Green => palette.success,
            Self::Blue => Color::oklch(lightness, chroma, 255.0),
            Self::Orange => Color::oklch(lightness + 0.04, chroma, 60.0),
            Self::Purple => Color::oklch(lightness, chroma, 310.0),
            Self::Red => Color::oklch(lightness, chroma + 0.02, 25.0),
        }
    }
}

fn paragraph(cx: &mut Context, text: &str, role: TextRole, ink: Color, width: f32) -> Paragraph {
    cx.text.paragraph(
        text,
        TextStyle::role(role, ink).tabular(),
        Some(width.max(1.0)),
        TextAlign::Start,
    )
}

fn panel(cx: &Context, frame: Rect, items: &mut Vec<Item>) {
    let shape = Shape::rounded(frame, Radius::L);
    items.push(Item::Fill {
        shape,
        paint: cx.palette.well.into(),
    });
    items.push(Item::Stroke {
        shape: Shape::rounded(frame.inset(0.5), Radius::L - 0.5),
        width: 1.0,
        paint: cx.palette.hairline.into(),
    });
}

fn texts(items: &mut Vec<Item>, texts: &[(Point, Paragraph)]) {
    for (origin, paragraph) in texts {
        items.push(Item::Text {
            origin: *origin,
            paragraph: paragraph.clone(),
        });
    }
}

pub struct Card {
    id: WidgetId,
    title: String,
    icon: Option<Image>,
    value: String,
    caption: String,
    on_click: Box<dyn FnMut()>,
    pressed: bool,
    texts: Vec<(Point, Paragraph)>,
    icon_rect: Rect,
    frame: Rect,
}

impl Card {
    pub fn new(title: &str, icon: Option<Image>, on_click: impl FnMut() + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            icon,
            value: String::new(),
            caption: String::new(),
            on_click: Box::new(on_click),
            pressed: false,
            texts: Vec::new(),
            icon_rect: Rect::default(),
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, value: &str, caption: &str) {
        value.clone_into(&mut self.value);
        caption.clone_into(&mut self.caption);
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, STAT_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = (frame.width - 2.0 * PADDING).max(0.0);
        let left = frame.x + PADDING;
        let top = frame.y + PADDING;
        let shift = match self.icon {
            Some(_) => ICON + ICON_GAP,
            None => 0.0,
        };
        let ink2 = cx.palette.ink2;
        let title = paragraph(cx, &self.title, TextRole::Callout, ink2, width - shift);
        let line = title.height();
        self.icon_rect = Rect::new(left, top + ((line - ICON) / 2.0).round(), ICON, ICON);
        let ink = cx.palette.ink;
        let value = paragraph(cx, &self.value, TextRole::Display, ink, width);
        let ink3 = cx.palette.ink3;
        let caption = cx.text.fitted(
            &self.caption,
            TextStyle::role(TextRole::Callout, ink3).tabular(),
            width,
        );
        let value_top = top + line + GAP;
        let caption_top = frame.y + frame.height - PADDING - caption.height();
        self.texts = vec![
            (Point::new(left + shift, top), title),
            (Point::new(left, value_top), value),
            (Point::new(left, caption_top), caption),
        ];
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        panel(cx, self.frame, items);
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: self.icon_rect,
                image: icon.clone(),
            });
        }
        texts(items, &self.texts);
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerLeave => self.pressed = false,
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let clicked = self.pressed && self.frame.contains(pointer.position);
                self.pressed = false;
                if clicked {
                    (self.on_click)();
                }
            }
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            (self.on_click)();
            return true;
        }
        false
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label(self.title.clone());
        node.set_value(format!("{} {}", self.value, self.caption));
        node.add_action(Action::Click);
        node
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineText {
    pub headline: String,
    pub uptime: String,
    pub detail: String,
    pub app: String,
    pub engine: String,
    pub running: bool,
}

pub struct EngineCard {
    id: WidgetId,
    icon: Option<Image>,
    text: EngineText,
    texts: Vec<(Point, Paragraph)>,
    disc: Rect,
    frame: Rect,
}

impl EngineCard {
    pub fn new(icon: Option<Image>) -> Self {
        Self {
            id: WidgetId::next(),
            icon,
            text: EngineText::default(),
            texts: Vec::new(),
            disc: Rect::default(),
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, text: EngineText) {
        self.text = text;
    }
}

impl Widget for EngineCard {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, ENGINE_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let left = frame.x + PADDING;
        self.disc = Rect::new(
            left,
            frame.y + ((frame.height - DISC) / 2.0).round(),
            DISC,
            DISC,
        );
        let palette = cx.palette;
        let right_width = (frame.width / 3.0).max(0.0);
        let app = cx.text.paragraph(
            &self.text.app,
            TextStyle::role(TextRole::Callout, palette.ink2).tabular(),
            Some(right_width),
            TextAlign::End,
        );
        let engine = cx.text.paragraph(
            &self.text.engine,
            TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
            Some(right_width),
            TextAlign::End,
        );
        let text_left = left + DISC + PADDING;
        let text_width = (frame.x + frame.width - PADDING - right_width - text_left).max(0.0);
        let headline = paragraph(
            cx,
            &self.text.headline,
            TextRole::BodyStrong,
            palette.ink,
            text_width,
        );
        let uptime = cx.text.fitted(
            &self.text.uptime,
            TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
            (text_width - headline.width() - GAP * 2.0).max(0.0),
        );
        let detail = cx.text.fitted(
            &self.text.detail,
            TextStyle::role(TextRole::Callout, palette.ink2).tabular(),
            text_width,
        );
        let block = headline.height() + GAP + detail.height();
        let top = frame.y + ((frame.height - block) / 2.0).round();
        let right_block = app.height() + engine.height();
        let right_top = frame.y + ((frame.height - right_block) / 2.0).round();
        let right_left = frame.x + frame.width - PADDING - right_width;
        let headline_width = headline.width();
        let headline_height = headline.height();
        self.texts = vec![
            (
                Point::new(
                    text_left + headline_width + GAP * 2.0,
                    top + (headline_height - uptime.height()).max(0.0),
                ),
                uptime,
            ),
            (Point::new(text_left, top), headline),
            (Point::new(text_left, top + headline_height + GAP), detail),
            (Point::new(right_left, right_top), app.clone()),
            (Point::new(right_left, right_top + app.height()), engine),
        ];
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        panel(cx, self.frame, items);
        let tone = match self.text.running {
            true => cx.palette.success,
            false => cx.palette.ink3,
        };
        items.push(Item::Fill {
            shape: Shape::rounded(self.disc, DISC / 2.0),
            paint: tone.with_opacity(TINT_OPACITY).into(),
        });
        if let Some(icon) = &self.icon {
            let inset = (DISC - DISC_ICON) / 2.0;
            items.push(Item::Image {
                rect: Rect::new(
                    self.disc.x + inset,
                    self.disc.y + inset,
                    DISC_ICON,
                    DISC_ICON,
                ),
                image: icon.clone(),
            });
        }
        texts(items, &self.texts);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.text.headline.clone());
        node.set_value(format!(
            "{} {} {} {}",
            self.text.uptime, self.text.detail, self.text.app, self.text.engine
        ));
        node
    }
}

type Legend = (Vec<(Point, Paragraph)>, Vec<(Rect, Tone)>, f32);

#[derive(Debug, Clone, Copy)]
struct Lane {
    tone: Tone,
    baseline: f32,
    height: f32,
    up: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub label: String,
    pub bytes: u64,
    pub tone: Tone,
}

pub struct Breakdown {
    id: WidgetId,
    title: String,
    parts: Vec<Part>,
    texts: Vec<(Point, Paragraph)>,
    dots: Vec<(Rect, Tone)>,
    bar: Rect,
    frame: Rect,
}

impl Breakdown {
    pub fn new(title: &str) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            parts: Vec::new(),
            texts: Vec::new(),
            dots: Vec::new(),
            bar: Rect::default(),
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, parts: Vec<Part>) {
        self.parts = parts.into_iter().filter(|part| part.bytes > 0).collect();
    }

    fn legend(&self, cx: &mut Context, width: f32) -> Legend {
        let mut texts = Vec::new();
        let mut dots = Vec::new();
        let (mut x, mut y, mut line) = (0.0f32, 0.0f32, 0.0f32);
        for part in &self.parts {
            let label = cx.text.paragraph(
                &part.label,
                TextStyle::role(TextRole::Callout, cx.palette.ink2).tabular(),
                None,
                TextAlign::Start,
            );
            let item = LEGEND_DOT + ICON_GAP + label.width();
            if x > 0.0 && x + item > width {
                x = 0.0;
                y += line + GAP;
            }
            line = line.max(label.height());
            let dot_y = y + ((label.height() - LEGEND_DOT) / 2.0).round();
            dots.push((Rect::new(x, dot_y, LEGEND_DOT, LEGEND_DOT), part.tone));
            texts.push((Point::new(x + LEGEND_DOT + ICON_GAP, y), label));
            x += item + LEGEND_GAP;
        }
        (texts, dots, y + line)
    }
}

impl Widget for Breakdown {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        if self.parts.is_empty() {
            return Size::new(available.width, 0.0);
        }
        let width = (available.width - 2.0 * PADDING).max(0.0);
        let title = paragraph(cx, &self.title, TextRole::BodyStrong, cx.palette.ink, width);
        let (_, _, legend) = self.legend(cx, width);
        let height = PADDING * 2.0 + title.height() + GAP * 2.0 + BAR_HEIGHT + GAP * 2.0 + legend;
        Size::new(available.width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        if self.parts.is_empty() {
            self.texts.clear();
            self.dots.clear();
            return;
        }
        let left = frame.x + PADDING;
        let width = (frame.width - 2.0 * PADDING).max(0.0);
        let title = paragraph(cx, &self.title, TextRole::BodyStrong, cx.palette.ink, width);
        let mut y = frame.y + PADDING;
        let mut placed = vec![(Point::new(left, y), title.clone())];
        y += title.height() + GAP * 2.0;
        self.bar = Rect::new(left, y, width, BAR_HEIGHT);
        y += BAR_HEIGHT + GAP * 2.0;
        let (legend, dots, _) = self.legend(cx, width);
        placed.extend(
            legend
                .into_iter()
                .map(|(origin, text)| (Point::new(origin.x + left, origin.y + y), text)),
        );
        self.dots = dots
            .into_iter()
            .map(|(rect, tone)| {
                (
                    Rect::new(rect.x + left, rect.y + y, rect.width, rect.height),
                    tone,
                )
            })
            .collect();
        self.texts = placed;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if self.parts.is_empty() {
            return;
        }
        panel(cx, self.frame, items);
        let total: u64 = self.parts.iter().map(|part| part.bytes).sum();
        let gaps = BAR_GAP * self.parts.len().saturating_sub(1) as f32;
        let room = (self.bar.width - gaps).max(0.0);
        let mut x = self.bar.x;
        let mut segments = Vec::new();
        for part in &self.parts {
            let width = (room * part.bytes as f32 / total.max(1) as f32).max(SEGMENT_MIN);
            segments.push(Item::Fill {
                shape: Shape::rect(Rect::new(x, self.bar.y, width, self.bar.height)),
                paint: part.tone.color(&cx.palette).into(),
            });
            x += width + BAR_GAP;
        }
        items.push(Item::Clip {
            shape: Shape::rounded(self.bar, Radius::S.min(BAR_HEIGHT / 2.0)),
            items: segments,
        });
        for (rect, tone) in &self.dots {
            items.push(Item::Fill {
                shape: Shape::rounded(*rect, LEGEND_DOT / 2.0),
                paint: tone.color(&cx.palette).into(),
            });
        }
        texts(items, &self.texts);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.title.clone());
        node.set_value(
            self.parts
                .iter()
                .map(|part| part.label.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        node
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plot {
    Single(Tone, Vec<f32>),
    Mirrored {
        up: (Tone, Vec<f32>),
        down: (Tone, Vec<f32>),
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    pub label: Option<(String, Tone)>,
    pub value: String,
}

pub struct Live {
    id: WidgetId,
    title: String,
    span: usize,
    readouts: Vec<Readout>,
    plot: Plot,
    note: Option<String>,
    texts: Vec<(Point, Paragraph)>,
    area: Rect,
    frame: Rect,
}

impl Live {
    pub fn new(title: &str, span: usize) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            span: span.max(2),
            readouts: Vec::new(),
            plot: Plot::Single(Tone::Green, Vec::new()),
            note: None,
            texts: Vec::new(),
            area: Rect::default(),
            frame: Rect::default(),
        }
    }

    pub fn set(&mut self, readouts: Vec<Readout>, plot: Plot, note: Option<&str>) {
        self.readouts = readouts;
        self.plot = plot;
        self.note = note.map(str::to_owned);
    }

    fn line(&self, values: &[f32], baseline: f32, height: f32, up: bool) -> Option<(Path, Path)> {
        let step = self.area.width / (self.span - 1) as f32;
        let start = self.span.saturating_sub(values.len());
        let points: Vec<Point> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let offset = value.clamp(0.0, 1.0) * height;
                Point::new(
                    self.area.x + (start + index) as f32 * step,
                    match up {
                        true => baseline - offset,
                        false => baseline + offset,
                    },
                )
            })
            .collect();
        let points = match points.as_slice() {
            [only] => vec![Point::new(only.x - 1.0, only.y), *only],
            _ => points,
        };
        let (first, last) = (points.first().copied()?, points.last().copied()?);
        let line = Path::through(&points);
        let area = line
            .clone()
            .line_to(Point::new(last.x, baseline))
            .line_to(Point::new(first.x, baseline))
            .close();
        Some((line, area))
    }

    fn paint_series(&self, palette: &Palette, values: &[f32], lane: Lane, items: &mut Vec<Item>) {
        let Lane {
            tone,
            baseline,
            height,
            up,
        } = lane;
        let Some((line, area)) = self.line(values, baseline, height, up) else {
            return;
        };
        let color = tone.color(palette);
        items.push(Item::FillPath {
            path: area,
            paint: color.with_opacity(AREA_OPACITY).into(),
        });
        items.push(Item::StrokePath {
            path: line,
            width: STROKE,
            paint: color.into(),
        });
    }
}

impl Widget for Live {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, LIVE_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let palette = cx.palette;
        let width = (frame.width - 2.0 * PADDING).max(0.0);
        let top = frame.y + PADDING;
        let title = paragraph(
            cx,
            &self.title,
            TextRole::Callout,
            palette.ink2,
            width / 2.0,
        );
        let single = self.readouts.len() == 1 && self.readouts[0].label.is_none();
        let mut pieces = Vec::new();
        for readout in &self.readouts {
            if let Some((label, tone)) = &readout.label {
                pieces.push(cx.text.paragraph(
                    label,
                    TextStyle::role(TextRole::Callout, tone.color(&palette)).tabular(),
                    None,
                    TextAlign::Start,
                ));
            }
            let role = match single {
                true => TextRole::Heading,
                false => TextRole::Callout,
            };
            pieces.push(cx.text.paragraph(
                &readout.value,
                TextStyle::role(role, palette.ink).tabular(),
                None,
                TextAlign::Start,
            ));
        }
        let header = pieces
            .iter()
            .map(Paragraph::height)
            .fold(title.height(), f32::max)
            .ceil();
        let mut x = frame.x + frame.width - PADDING;
        let mut placed = Vec::new();
        for piece in pieces.into_iter().rev() {
            x -= piece.width();
            placed.push((Point::new(x, top + header - piece.height()), piece));
            x -= GAP;
        }
        placed.push((
            Point::new(frame.x + PADDING, top + header - title.height()),
            title,
        ));
        if let Some(note) = &self.note {
            let note = paragraph(cx, note, TextRole::Callout, palette.ink3, width);
            let y = frame.y + frame.height - PADDING - note.height();
            placed.push((Point::new(frame.x + PADDING, y), note));
        }
        self.texts = placed;
        let area_top = top + header + GAP * 2.0;
        self.area = Rect::new(
            frame.x + PADDING,
            area_top,
            width,
            (frame.y + frame.height - PADDING - area_top).max(1.0),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        panel(cx, self.frame, items);
        texts(items, &self.texts);
        if self.note.is_some() {
            return;
        }
        let palette = &cx.palette;
        let mut plot = Vec::new();
        match &self.plot {
            Plot::Single(tone, values) => {
                let bottom = self.area.y + self.area.height;
                let lane = Lane {
                    tone: *tone,
                    baseline: bottom,
                    height: self.area.height,
                    up: true,
                };
                self.paint_series(palette, values, lane, &mut plot);
            }
            Plot::Mirrored { up, down } => {
                let middle = (self.area.y + self.area.height / 2.0).round();
                let half = self.area.height / 2.0;
                plot.push(Item::Fill {
                    shape: Shape::rect(Rect::new(self.area.x, middle, self.area.width, 1.0)),
                    paint: palette.hairline.into(),
                });
                let lane = |tone: Tone, up: bool| Lane {
                    tone,
                    baseline: middle,
                    height: half,
                    up,
                };
                self.paint_series(palette, &up.1, lane(up.0, true), &mut plot);
                self.paint_series(palette, &down.1, lane(down.0, false), &mut plot);
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.area.inset(-STROKE)),
            items: plot,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.title.clone());
        node.set_value(
            self.readouts
                .iter()
                .map(|readout| match &readout.label {
                    Some((label, _)) => format!("{label} {}", readout.value),
                    None => readout.value.clone(),
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        node
    }
}

pub struct Grid {
    id: WidgetId,
    columns: usize,
    gap: f32,
    children: Vec<Box<dyn Widget>>,
    heights: Vec<f32>,
    frame: Rect,
}

impl Grid {
    pub fn new(columns: usize, gap: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            columns: columns.max(1),
            gap,
            children,
            heights: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn cell_width(&self, width: f32) -> f32 {
        ((width - self.gap * (self.columns - 1) as f32) / self.columns as f32).max(0.0)
    }
}

impl Widget for Grid {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = self.cell_width(available.width);
        let sizes: Vec<f32> = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(width, available.height)).height)
            .collect();
        self.heights = sizes
            .chunks(self.columns)
            .map(|row| row.iter().copied().fold(0.0, f32::max))
            .collect();
        let height = self.heights.iter().sum::<f32>()
            + self.gap * self.heights.len().saturating_sub(1) as f32;
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let width = self.cell_width(frame.width);
        let mut y = frame.y;
        for (row, height) in self.children.chunks_mut(self.columns).zip(&self.heights) {
            for (index, child) in row.iter_mut().enumerate() {
                let x = frame.x + index as f32 * (width + self.gap);
                child.place(cx, Rect::new(x, y, width, *height));
            }
            y += height + self.gap;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
