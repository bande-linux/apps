use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Context, Cursor, Event, Key, PointerButton, Scroll, Size, Widget, WidgetId,
};
use bande_toolkit::{
    Color, Image, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, Transform,
};

use bande_content::{Decoded, Level, Pixels};

const TILE: u32 = 512;
const OVERLAP: u32 = 2;
const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 64.0;
const ZOOM_STEP: f32 = std::f32::consts::SQRT_2;
const WHEEL_ZOOM: f32 = 240.0;
const MESSAGE_WIDTH: f32 = 420.0;
const LINE_GAP: f32 = 8.0;
const KEY_PAN: f32 = 60.0;
const LATE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    Auto,
    Fit,
    Scale(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Orientation {
    pub turns: u8,
    pub flipped: bool,
}

impl Orientation {
    pub fn rotated(self, clockwise: bool) -> Self {
        let step = if clockwise != self.flipped { 1 } else { 3 };
        Self {
            turns: (self.turns + step) % 4,
            ..self
        }
    }

    pub fn flipped(self, vertical: bool) -> Self {
        Self {
            turns: if vertical {
                (self.turns + 2) % 4
            } else {
                self.turns
            },
            flipped: !self.flipped,
        }
    }

    fn sideways(self) -> bool {
        self.turns % 2 == 1
    }

    fn transform(self, width: f32, height: f32) -> Transform {
        let matrix = match self.turns {
            1 => [0.0, 1.0, -1.0, 0.0, height, 0.0],
            2 => [-1.0, 0.0, 0.0, -1.0, width, height],
            3 => [0.0, -1.0, 1.0, 0.0, 0.0, width],
            _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        };
        let turned = Transform { matrix };
        match self.flipped {
            false => turned,
            true => {
                let across = if self.sideways() { height } else { width };
                turned.then(Transform {
                    matrix: [-1.0, 0.0, 0.0, 1.0, across, 0.0],
                })
            }
        }
    }
}

fn inverse(transform: Transform) -> Option<Transform> {
    let [a, b, c, d, e, f] = transform.matrix;
    let determinant = a * d - b * c;
    if determinant.abs() < f32::EPSILON {
        return None;
    }
    let (ia, ib, ic, id) = (
        d / determinant,
        -b / determinant,
        -c / determinant,
        a / determinant,
    );
    Some(Transform {
        matrix: [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)],
    })
}

enum Shown {
    Nothing,
    Message { title: String, detail: String },
    Picture(Box<Decoded>),
}

struct Animation {
    frame: usize,
    next: Option<Instant>,
    playing: bool,
}

type TileKey = (usize, u32, u32);

pub struct Canvas {
    id: WidgetId,
    frame: Rect,
    shown: Shown,
    label: String,
    zoom: Zoom,
    center: Point,
    orientation: Orientation,
    dragging: Option<Point>,
    animation: Animation,
    dark: bool,
    radius: f32,
    lines: Vec<(Point, Paragraph)>,
    tiles: RefCell<HashMap<TileKey, Image>>,
    frame_image: RefCell<Option<(usize, Image)>>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            shown: Shown::Nothing,
            label: String::new(),
            zoom: Zoom::Auto,
            center: Point::new(0.0, 0.0),
            orientation: Orientation::default(),
            dragging: None,
            animation: Animation {
                frame: 0,
                next: None,
                playing: false,
            },
            dark: false,
            radius: 0.0,
            lines: Vec::new(),
            tiles: RefCell::default(),
            frame_image: RefCell::default(),
        }
    }

    pub fn clear(&mut self, label: &str) {
        self.shown = Shown::Nothing;
        self.label = label.to_owned();
        self.forget();
    }

    pub fn show_message(&mut self, title: &str, detail: &str) {
        self.shown = Shown::Message {
            title: title.to_owned(),
            detail: detail.to_owned(),
        };
        self.label = title.to_owned();
        self.forget();
    }

    pub fn show(&mut self, decoded: Box<Decoded>, label: &str, now: Instant) {
        self.animation = Animation {
            frame: 0,
            next: match &decoded.pixels {
                Pixels::Animation(frames) => frames.first().map(|frame| now + frame.delay),
                Pixels::Still(_) => None,
            },
            playing: matches!(decoded.pixels, Pixels::Animation(_)),
        };
        self.shown = Shown::Picture(decoded);
        self.label = label.to_owned();
        self.zoom = Zoom::Auto;
        self.orientation = Orientation::default();
        self.forget();
        self.recenter();
    }

    fn forget(&mut self) {
        self.tiles.borrow_mut().clear();
        self.frame_image.replace(None);
        self.lines.clear();
        self.dragging = None;
    }

    pub fn picture(&self) -> Option<&Decoded> {
        match &self.shown {
            Shown::Picture(decoded) => Some(decoded),
            _ => None,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
    }

    pub fn is_animated(&self) -> bool {
        matches!(
            self.picture().map(|picture| &picture.pixels),
            Some(Pixels::Animation(_))
        )
    }

    pub fn is_playing(&self) -> bool {
        self.is_animated() && self.animation.playing
    }

    pub fn current_frame(&self) -> usize {
        self.animation.frame
    }

    pub fn toggle_playing(&mut self, now: Instant) {
        if !self.is_animated() {
            return;
        }
        self.animation.playing = !self.animation.playing;
        self.animation.next = None;
        if self.animation.playing {
            self.resume(now);
        }
    }

    pub fn resume(&mut self, now: Instant) {
        if let (Some(delay), true) = (self.delay(self.animation.frame), self.is_playing()) {
            self.animation.next = Some(now + delay);
        }
    }

    fn delay(&self, frame: usize) -> Option<Duration> {
        match &self.picture()?.pixels {
            Pixels::Animation(frames) => frames.get(frame).map(|frame| frame.delay),
            Pixels::Still(_) => None,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.is_playing().then_some(self.animation.next).flatten()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(mut next) = self.deadline() else {
            return false;
        };
        let count = match self.picture().map(|picture| &picture.pixels) {
            Some(Pixels::Animation(frames)) => frames.len(),
            _ => return false,
        };
        if now.saturating_duration_since(next) > LATE {
            next = now;
        }
        let mut frame = self.animation.frame;
        while next <= now {
            frame = (frame + 1) % count;
            next += self.delay(frame).unwrap_or(LATE);
        }
        self.animation.frame = frame;
        self.animation.next = Some(next);
        true
    }

    fn content_size(&self) -> Option<(f32, f32)> {
        let picture = self.picture()?;
        let (width, height) = (picture.width as f32, picture.height as f32);
        Some(match self.orientation.sideways() {
            true => (height, width),
            false => (width, height),
        })
    }

    fn fit_scale(&self) -> f32 {
        match self.content_size() {
            Some((width, height)) if width > 0.0 && height > 0.0 => {
                (self.frame.width / width).min(self.frame.height / height)
            }
            _ => 1.0,
        }
    }

    pub fn scale(&self) -> f32 {
        match self.zoom {
            Zoom::Auto => self.fit_scale().min(1.0),
            Zoom::Fit => self.fit_scale(),
            Zoom::Scale(scale) => scale,
        }
        .clamp(MIN_SCALE, MAX_SCALE)
    }

    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: Zoom) {
        self.zoom_around(zoom, self.middle());
    }

    pub fn zoom_by(&mut self, steps: f32) {
        let scale = self.scale() * ZOOM_STEP.powf(steps);
        self.zoom_around(Zoom::Scale(scale), self.middle());
    }

    fn middle(&self) -> Point {
        Point::new(
            self.frame.x + self.frame.width / 2.0,
            self.frame.y + self.frame.height / 2.0,
        )
    }

    fn zoom_around(&mut self, zoom: Zoom, at: Point) {
        let before = self.scale();
        let middle = self.middle();
        let anchor = Point::new(
            self.center.x + (at.x - middle.x) / before,
            self.center.y + (at.y - middle.y) / before,
        );
        self.zoom = match zoom {
            Zoom::Scale(scale) => Zoom::Scale(scale.clamp(MIN_SCALE, MAX_SCALE)),
            other => other,
        };
        let after = self.scale();
        self.center = Point::new(
            anchor.x - (at.x - middle.x) / after,
            anchor.y - (at.y - middle.y) / after,
        );
        self.clamp_center();
    }

    pub fn orient(&mut self, orientation: Orientation) {
        self.orientation = orientation;
        self.recenter();
    }

    fn recenter(&mut self) {
        if let Some((width, height)) = self.content_size() {
            self.center = Point::new(width / 2.0, height / 2.0);
        }
    }

    fn clamp_center(&mut self) {
        let Some((width, height)) = self.content_size() else {
            return;
        };
        let scale = self.scale();
        let axis = |center: f32, content: f32, view: f32| {
            let half = view / (2.0 * scale);
            match content * scale <= view {
                true => content / 2.0,
                false => center.clamp(half, content - half),
            }
        };
        self.center = Point::new(
            axis(self.center.x, width, self.frame.width),
            axis(self.center.y, height, self.frame.height),
        );
    }

    pub fn overflows(&self) -> bool {
        self.content_size().is_some_and(|(width, height)| {
            let scale = self.scale();
            width * scale > self.frame.width + 0.5 || height * scale > self.frame.height + 0.5
        })
    }

    pub fn pan(&mut self, dx: f32, dy: f32) -> bool {
        if !self.overflows() {
            return false;
        }
        let scale = self.scale();
        self.center = Point::new(self.center.x + dx / scale, self.center.y + dy / scale);
        self.clamp_center();
        true
    }

    fn transform(&self, width: f32, height: f32) -> Transform {
        let scale = self.scale();
        let middle = self.middle();
        self.orientation
            .transform(width, height)
            .then(Transform::scale(scale, scale))
            .then(Transform::translate(
                (middle.x - self.center.x * scale).round(),
                (middle.y - self.center.y * scale).round(),
            ))
    }

    fn visible_source(&self, transform: Transform, width: f32, height: f32) -> Option<Rect> {
        let back = inverse(transform)?;
        let corners = [
            Point::new(self.frame.x, self.frame.y),
            Point::new(self.frame.x + self.frame.width, self.frame.y),
            Point::new(self.frame.x, self.frame.y + self.frame.height),
            Point::new(
                self.frame.x + self.frame.width,
                self.frame.y + self.frame.height,
            ),
        ]
        .map(|corner| back.apply(corner));
        let left = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::MAX, f32::min)
            .max(0.0);
        let top = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::MAX, f32::min)
            .max(0.0);
        let right = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::MIN, f32::max)
            .min(width);
        let bottom = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::MIN, f32::max)
            .min(height);
        (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
    }

    fn tile_items(&self, levels: &[Level], cx: &Context, transform: Transform) -> Vec<Item> {
        let Some(full) = levels.first() else {
            return Vec::new();
        };
        let (width, height) = (full.width as f32, full.height as f32);
        let Some(visible) = self.visible_source(transform, width, height) else {
            return Vec::new();
        };
        let device = self.scale() * cx.display_scale as f32;
        let wanted = match device >= 1.0 {
            true => 0,
            false => (1.0 / device).log2().floor().max(0.0) as usize,
        };
        let index = wanted.min(levels.len() - 1);
        let Some(level) = levels.get(index) else {
            return Vec::new();
        };
        let step = (1u32 << index) as f32;
        let reach = TILE as f32 * step;
        let first_x = (visible.x / reach).floor() as u32;
        let first_y = (visible.y / reach).floor() as u32;
        let last_x = ((visible.x + visible.width) / reach).ceil() as u32;
        let last_y = ((visible.y + visible.height) / reach).ceil() as u32;
        let mut previous = self.tiles.borrow_mut();
        let mut kept = HashMap::new();
        let mut items = Vec::new();
        for tile_y in first_y..last_y {
            for tile_x in first_x..last_x {
                let key = (index, tile_x, tile_y);
                let image = match previous.remove(&key) {
                    Some(image) => Some(image),
                    None => tile(level, tile_x, tile_y),
                };
                let Some(image) = image else {
                    continue;
                };
                let x = tile_x as f32 * reach;
                let y = tile_y as f32 * reach;
                let rect = Rect::new(
                    x,
                    y,
                    (image.width() as f32 * step).min(width - x),
                    (image.height() as f32 * step).min(height - y),
                );
                items.push(Item::Image {
                    rect,
                    image: image.clone(),
                });
                kept.insert(key, image);
            }
        }
        *previous = kept;
        items
    }

    fn frame_items(&self, frames: &[bande_content::Frame], width: u32, height: u32) -> Vec<Item> {
        let index = self.animation.frame.min(frames.len().saturating_sub(1));
        let mut cached = self.frame_image.borrow_mut();
        let image = match cached.as_ref() {
            Some((shown, image)) if *shown == index => Some(image.clone()),
            _ => frames.get(index).and_then(|frame| {
                let image = Image::from_rgba(width, height, frame.rgba.to_vec())?;
                *cached = Some((index, image.clone()));
                Some(image)
            }),
        };
        image
            .map(|image| Item::Image {
                rect: Rect::new(0.0, 0.0, width as f32, height as f32),
                image,
            })
            .into_iter()
            .collect()
    }

    fn lay_out_message(&mut self, cx: &mut Context) {
        self.lines.clear();
        let Shown::Message { title, detail } = &self.shown else {
            return;
        };
        let width = (self.frame.width - 48.0).clamp(0.0, MESSAGE_WIDTH);
        let palette = cx.palette;
        let ink = if self.dark { Color::WHITE } else { palette.ink };
        let quiet = if self.dark {
            Color::WHITE
        } else {
            palette.ink2
        };
        let texts = [
            (
                title.as_str(),
                TextStyle::new(17.0, ink).weight(TextStyle::SEMIBOLD),
            ),
            (detail.as_str(), TextStyle::new(13.0, quiet)),
        ];
        let paragraphs: Vec<Paragraph> = texts
            .into_iter()
            .filter(|(text, _)| !text.is_empty())
            .map(|(text, style)| {
                cx.text
                    .paragraph(text, style, Some(width), TextAlign::Center)
            })
            .collect();
        let total: f32 = paragraphs
            .iter()
            .map(|p| p.height() + LINE_GAP)
            .sum::<f32>()
            - LINE_GAP;
        let mut y = self.frame.y + ((self.frame.height - total) / 2.0).max(0.0);
        let x = self.frame.x + ((self.frame.width - width) / 2.0).max(0.0);
        for paragraph in paragraphs {
            let height = paragraph.height();
            self.lines.push((Point::new(x, y.round()), paragraph));
            y += height + LINE_GAP;
        }
    }
}

fn tile(level: &Level, tile_x: u32, tile_y: u32) -> Option<Image> {
    let x = tile_x.checked_mul(TILE)?;
    let y = tile_y.checked_mul(TILE)?;
    if x >= level.width || y >= level.height {
        return None;
    }
    let width = (TILE + OVERLAP).min(level.width - x);
    let height = (TILE + OVERLAP).min(level.height - y);
    let source = Arc::clone(&level.rgba);
    let stride = level.width as usize * 4;
    let crop = move || {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in y..y + height {
            let start = row as usize * stride + x as usize * 4;
            rgba.extend_from_slice(source.get(start..start + width as usize * 4)?);
        }
        Some(rgba)
    };
    Image::redrawable(width, height, crop()?, crop)
}

impl Widget for Canvas {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        let resized = frame != self.frame;
        self.frame = frame;
        if resized {
            self.clamp_center();
        }
        if resized || self.lines.is_empty() {
            self.lay_out_message(cx);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let background = match self.dark {
            true => Color::BLACK,
            false => cx.palette.sheet,
        };
        let mut content = vec![Item::Fill {
            shape: Shape::rect(self.frame),
            paint: background.into(),
        }];
        if let Some(picture) = self.picture() {
            let (width, height) = (picture.width as f32, picture.height as f32);
            let transform = self.transform(width, height);
            let drawn = match &picture.pixels {
                Pixels::Still(levels) => self.tile_items(levels, cx, transform),
                Pixels::Animation(frames) => {
                    self.frame_items(frames, picture.width, picture.height)
                }
            };
            content.push(Item::Transform {
                transform,
                items: drawn,
            });
        }
        content.extend(self.lines.iter().map(|(origin, paragraph)| Item::Text {
            origin: *origin,
            paragraph: paragraph.clone(),
        }));
        items.push(Item::Clip {
            shape: Shape::rounded(self.frame, self.radius),
            items: content,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Image);
        node.set_label(self.label.clone());
        if let Shown::Message { detail, .. } = &self.shown {
            node.set_description(detail.clone());
        }
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::Scroll(scroll) if scroll.modifiers.control => self.wheel_zoom(scroll),
            Event::Scroll(scroll) => self.pan(-scroll.delta.x, -scroll.delta.y),
            Event::PointerDown(pointer)
                if pointer.button == PointerButton::Primary && self.overflows() =>
            {
                self.dragging = Some(pointer.position);
                true
            }
            Event::PointerMove(pointer) => match self.dragging {
                Some(from) => {
                    self.dragging = Some(pointer.position);
                    self.pan(from.x - pointer.position.x, from.y - pointer.position.y)
                }
                None => false,
            },
            Event::PointerUp(_) => self.dragging.take().is_some(),
            Event::Key { key, modifiers } if !modifiers.control && !modifiers.alt => match key {
                Key::Up => self.pan(0.0, -KEY_PAN),
                Key::Down => self.pan(0.0, KEY_PAN),
                _ => false,
            },
            _ => false,
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn cursor(&self, _point: Point) -> Option<Cursor> {
        Some(match (self.dragging.is_some(), self.overflows()) {
            (true, _) => Cursor::Grabbing,
            (false, true) => Cursor::Grab,
            (false, false) => Cursor::Default,
        })
    }
}

impl Canvas {
    fn wheel_zoom(&mut self, scroll: &Scroll) -> bool {
        if self.picture().is_none() {
            return false;
        }
        let factor = (-scroll.delta.y / WHEEL_ZOOM).exp();
        let scale = self.scale() * factor;
        self.zoom_around(Zoom::Scale(scale), scroll.position);
        true
    }
}
