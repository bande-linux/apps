use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Context, Cursor, Event, Key, PointerButton, Scroll, ScrollState, Size, Widget, WidgetId,
};
use bande_toolkit::{Color, Image, Item, Point, Rect, Shape};

use bande_content::pdf::protocol::{Glyph, MAX_RENDER_PIXELS, Render, Use};

const MARGIN: f32 = 16.0;
const GAP: f32 = 14.0;
const PAGE_PIXELS: f32 = 8_388_608.0;
const LINE: f32 = 48.0;
const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 16.0;
const WHEEL_ZOOM: f32 = 240.0;
const ZOOM_STEP: f32 = std::f32::consts::SQRT_2;
const SAME_SCALE: f32 = 0.001;
const KEEP_AROUND: usize = 2;
const SELECTION: Color = Color::hex(0x3A7BF0).with_alpha(0x55);
const MATCH: Color = Color::hex(0xF5C518).with_alpha(0x66);
const CURRENT_MATCH: Color = Color::hex(0xF08A00).with_alpha(0xAA);

pub struct PageText {
    text: String,
    starts: Vec<usize>,
    rects: Vec<[f32; 4]>,
    lower: String,
    lower_starts: Vec<usize>,
}

impl PageText {
    pub fn new(glyphs: Vec<Glyph>) -> Self {
        let mut page = Self {
            text: String::new(),
            starts: Vec::with_capacity(glyphs.len() + 1),
            rects: Vec::with_capacity(glyphs.len()),
            lower: String::new(),
            lower_starts: Vec::with_capacity(glyphs.len()),
        };
        for glyph in glyphs {
            page.starts.push(page.text.len());
            page.lower_starts.push(page.lower.len());
            page.text.push_str(&glyph.text);
            page.lower
                .extend(glyph.text.chars().flat_map(char::to_lowercase));
            page.rects.push(glyph.rect);
        }
        page.starts.push(page.text.len());
        page
    }

    fn len(&self) -> usize {
        self.rects.len()
    }

    fn glyph(&self, index: usize) -> &str {
        match (self.starts.get(index), self.starts.get(index + 1)) {
            (Some(start), Some(end)) => self.text.get(*start..*end).unwrap_or_default(),
            _ => "",
        }
    }

    fn text(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.len());
        let start = range.start.min(end);
        match (self.starts.get(start), self.starts.get(end)) {
            (Some(from), Some(to)) => self.text.get(*from..*to).unwrap_or_default().to_owned(),
            _ => String::new(),
        }
    }

    fn glyph_at(&self, byte: usize) -> usize {
        self.lower_starts
            .partition_point(|start| *start <= byte)
            .saturating_sub(1)
    }

    fn find(&self, needle: &str) -> Vec<Range<usize>> {
        if needle.is_empty() {
            return Vec::new();
        }
        self.lower
            .match_indices(needle)
            .map(|(start, found)| {
                self.glyph_at(start)..self.glyph_at(start + found.len().saturating_sub(1)) + 1
            })
            .collect()
    }

    fn hit(&self, point: Point) -> Option<usize> {
        let distance = |rect: &[f32; 4]| {
            let [left, top, right, bottom] = *rect;
            let dx = (left - point.x).max(point.x - right).max(0.0);
            let dy = (top - point.y).max(point.y - bottom).max(0.0);
            dy * 4.0 + dx
        };
        self.rects
            .iter()
            .enumerate()
            .filter(|(index, _)| self.glyph(*index) != "\n")
            .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
            .map(|(index, _)| index)
    }
}

struct Shown {
    render: Render,
    image: Image,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    Width,
    Scale(f32),
}

type Spot = (usize, usize);

pub struct PdfView {
    id: WidgetId,
    frame: Rect,
    radius: f32,
    label: String,
    pages: Vec<(f32, f32)>,
    tops: Vec<f32>,
    fit: Fit,
    scale: f32,
    scroll: ScrollState,
    sideways: f32,
    bases: HashMap<usize, Shown>,
    details: HashMap<usize, Shown>,
    texts: Vec<Option<PageText>>,
    selection: Option<(Spot, Spot)>,
    selecting: bool,
    query: String,
    matches: Vec<(usize, Range<usize>)>,
    current: Option<usize>,
    anchor: Option<(usize, f32, f32)>,
    across: Option<(f32, f32)>,
    jump: Option<(usize, f32)>,
    visible: Cell<(usize, usize)>,
    viewport: Cell<(f32, f32)>,
    display_scale: Cell<f32>,
}

impl PdfView {
    pub fn new(label: &str, pages: Vec<(f32, f32)>) -> Self {
        let count = pages.len();
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            radius: 0.0,
            label: label.to_owned(),
            pages,
            tops: Vec::new(),
            fit: Fit::Width,
            scale: 1.0,
            scroll: ScrollState::default(),
            sideways: 0.0,
            bases: HashMap::new(),
            details: HashMap::new(),
            texts: (0..count).map(|_| None).collect(),
            selection: None,
            selecting: false,
            query: String::new(),
            matches: Vec::new(),
            current: None,
            anchor: None,
            across: None,
            jump: None,
            visible: Cell::new((0, 0)),
            viewport: Cell::new((0.0, 0.0)),
            display_scale: Cell::new(1.0),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
    }

    fn widest(&self) -> f32 {
        self.pages
            .iter()
            .map(|(width, _)| *width)
            .fold(1.0, f32::max)
    }

    fn fitted_scale(&self) -> f32 {
        match self.fit {
            Fit::Width => (self.frame.width - 2.0 * MARGIN) / self.widest(),
            Fit::Scale(scale) => scale,
        }
        .clamp(MIN_SCALE, MAX_SCALE)
    }

    pub fn set_fit(&mut self, fit: Fit) {
        let middle = Point::new(
            self.frame.x + self.frame.width / 2.0,
            self.frame.y + self.frame.height / 2.0,
        );
        self.hold(middle);
        self.fit = fit;
    }

    fn hold(&mut self, at: Point) {
        self.anchor = self.spot_at_height(at.y);
        let inside = at.x - self.frame.x;
        self.across = Some(((self.sideways + inside) / self.content_width(), inside));
    }

    pub fn zoom_by(&mut self, steps: f32) {
        self.set_fit(Fit::Scale(self.scale * ZOOM_STEP.powf(steps)));
    }

    fn spot_at_height(&self, y: f32) -> Option<(usize, f32, f32)> {
        let offset = self.viewport.get().0;
        let content = y - self.frame.y + offset;
        let page = self
            .tops
            .partition_point(|top| *top <= content)
            .checked_sub(1)?;
        Some((page, (content - self.tops[page]) / self.scale, y))
    }

    fn content_height(&self) -> f32 {
        match (self.tops.last(), self.pages.last()) {
            (Some(top), Some((_, height))) => top + height * self.scale + MARGIN,
            _ => 0.0,
        }
    }

    fn content_width(&self) -> f32 {
        self.widest() * self.scale + 2.0 * MARGIN
    }

    fn lay_out(&mut self) {
        self.scale = self.fitted_scale();
        let mut y = MARGIN;
        self.tops = self
            .pages
            .iter()
            .map(|(_, height)| {
                let top = y;
                y += height * self.scale + GAP;
                top
            })
            .collect();
    }

    fn page_rect(&self, page: usize, offset: f32) -> Option<Rect> {
        let (width, height) = *self.pages.get(page)?;
        let top = *self.tops.get(page)?;
        let shown = width * self.scale;
        let x = match self.content_width() <= self.frame.width {
            true => self.frame.x + ((self.frame.width - shown) / 2.0).round(),
            false => {
                self.frame.x + MARGIN + ((self.widest() * self.scale - shown) / 2.0).round()
                    - self.sideways
            }
        };
        Some(Rect::new(
            x,
            self.frame.y + top - offset,
            shown,
            height * self.scale,
        ))
    }

    pub fn current_page(&self) -> usize {
        let (offset, height) = self.viewport.get();
        let middle = offset + height / 3.0;
        self.tops
            .partition_point(|top| *top <= middle)
            .saturating_sub(1)
    }

    pub fn go_to_page(&mut self, page: usize) {
        if page < self.pages.len() {
            self.jump = Some((page, 0.0));
        }
    }

    pub fn page_label(&self) -> (usize, usize) {
        (self.current_page() + 1, self.pages.len())
    }

    pub fn set_render(&mut self, render: Render, pixels: Vec<u8>) -> bool {
        let Some(image) = Image::from_premultiplied(render.width, render.height, pixels) else {
            return false;
        };
        let shown = Shown { render, image };
        match render.purpose {
            Use::Page => self.bases.insert(render.page, shown),
            Use::Detail => self.details.insert(render.page, shown),
            Use::Thumbnail => return false,
        };
        true
    }

    pub fn set_text(&mut self, page: usize, glyphs: Vec<Glyph>) {
        let Some(slot) = self.texts.get_mut(page) else {
            return;
        };
        let text = PageText::new(glyphs);
        let found = text.find(&self.query);
        *slot = Some(text);
        if found.is_empty() {
            return;
        }
        let at = self.matches.partition_point(|(other, _)| *other < page);
        let count = found.len();
        self.matches
            .splice(at..at, found.into_iter().map(|range| (page, range)));
        match self.current {
            Some(index) if index >= at => self.current = Some(index + count),
            Some(_) => {}
            None => {
                self.current = Some(at);
                self.reveal_match();
            }
        }
    }

    pub fn missing_text(&self, pages: Range<usize>) -> Vec<usize> {
        pages
            .filter(|page| self.texts.get(*page).is_some_and(Option::is_none))
            .collect()
    }

    pub fn visible_pages(&self) -> Range<usize> {
        let (first, last) = self.visible.get();
        first..last
    }

    pub fn wanted(&self) -> Vec<Render> {
        let display = self.display_scale.get();
        let device = self.scale * display;
        let visible = self.visible_pages();
        let (offset, _) = self.viewport.get();
        let around = visible.start..(visible.end + 1).min(self.pages.len());
        let mut wanted = Vec::new();
        for page in around {
            let Some((width, height)) = self.pages.get(page).copied() else {
                continue;
            };
            let full = width * height * device * device;
            let base_scale = match full <= PAGE_PIXELS {
                true => device,
                false => (PAGE_PIXELS / (width * height)).sqrt(),
            };
            let current = self
                .bases
                .get(&page)
                .is_some_and(|shown| (shown.render.scale - base_scale).abs() < SAME_SCALE);
            if !current {
                wanted.push(Render {
                    page,
                    scale: base_scale,
                    x: 0,
                    y: 0,
                    width: (width * base_scale).ceil() as u32,
                    height: (height * base_scale).ceil() as u32,
                    purpose: Use::Page,
                });
            }
            if base_scale >= device - SAME_SCALE || !visible.contains(&page) {
                continue;
            }
            let Some(rect) = self.page_rect(page, offset) else {
                continue;
            };
            let left = (self.frame.x.max(rect.x) - rect.x) * display;
            let top = (self.frame.y.max(rect.y) - rect.y) * display;
            let right =
                ((self.frame.x + self.frame.width).min(rect.x + rect.width) - rect.x) * display;
            let bottom =
                ((self.frame.y + self.frame.height).min(rect.y + rect.height) - rect.y) * display;
            if right <= left || bottom <= top {
                continue;
            }
            let detail = Render {
                page,
                scale: device,
                x: left.floor() as u32,
                y: top.floor() as u32,
                width: (right - left.floor()).ceil() as u32,
                height: (bottom - top.floor()).ceil() as u32,
                purpose: Use::Detail,
            };
            let pixels = u64::from(detail.width) * u64::from(detail.height);
            let covered = self.details.get(&page).is_some_and(|shown| {
                let had = shown.render;
                (had.scale - device).abs() < SAME_SCALE
                    && had.x <= detail.x
                    && had.y <= detail.y
                    && had.x + had.width >= detail.x + detail.width
                    && had.y + had.height >= detail.y + detail.height
            });
            if !covered && pixels <= MAX_RENDER_PIXELS {
                wanted.push(detail);
            }
        }
        wanted
    }

    pub fn forget_far(&mut self) {
        let visible = self.visible_pages();
        let keep = visible.start.saturating_sub(KEEP_AROUND)..visible.end + KEEP_AROUND;
        self.bases.retain(|page, _| keep.contains(page));
        self.details.retain(|page, _| visible.contains(page));
    }

    pub fn set_query(&mut self, query: &str) {
        let query = query.to_lowercase();
        if query == self.query {
            return;
        }
        self.query = query;
        self.current = None;
        self.find_all();
        if !self.matches.is_empty() {
            self.current = Some(0);
            self.reveal_match();
        }
    }

    fn find_all(&mut self) {
        self.matches = self
            .texts
            .iter()
            .enumerate()
            .filter_map(|(page, text)| Some((page, text.as_ref()?)))
            .flat_map(|(page, text)| {
                text.find(&self.query)
                    .into_iter()
                    .map(move |range| (page, range))
            })
            .collect();
    }

    pub fn matches(&self) -> (Option<usize>, usize) {
        (self.current, self.matches.len())
    }

    pub fn next_match(&mut self, forward: bool) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.current = Some(match (self.current, forward) {
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
            (None, _) => 0,
        });
        self.reveal_match();
    }

    fn reveal_match(&mut self) {
        let Some((page, range)) = self.current.and_then(|index| self.matches.get(index)) else {
            return;
        };
        let top = self
            .texts
            .get(*page)
            .and_then(Option::as_ref)
            .and_then(|text| text.rects.get(range.start))
            .map_or(0.0, |rect| rect[1]);
        self.jump = Some((*page, top));
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.ordered_selection()?;
        let mut parts = Vec::new();
        for page in start.0..=end.0 {
            let Some(text) = self.texts.get(page).and_then(Option::as_ref) else {
                continue;
            };
            let first = if page == start.0 { start.1 } else { 0 };
            let last = if page == end.0 { end.1 + 1 } else { text.len() };
            parts.push(text.text(first..last));
        }
        let joined = parts.join("\n");
        (!joined.trim().is_empty()).then_some(joined)
    }

    fn ordered_selection(&self) -> Option<(Spot, Spot)> {
        let (anchor, focus) = self.selection?;
        Some(match anchor <= focus {
            true => (anchor, focus),
            false => (focus, anchor),
        })
    }

    fn spot(&self, point: Point) -> Option<Spot> {
        let offset = self.viewport.get().0;
        let page = (0..self.pages.len()).min_by(|a, b| {
            let distance = |page: usize| {
                self.page_rect(page, offset).map_or(f32::MAX, |rect| {
                    (rect.y - point.y)
                        .max(point.y - rect.y - rect.height)
                        .max(0.0)
                })
            };
            distance(*a).total_cmp(&distance(*b))
        })?;
        let rect = self.page_rect(page, offset)?;
        let inside = Point::new(
            (point.x - rect.x) / self.scale,
            (point.y - rect.y) / self.scale,
        );
        let glyph = self.texts.get(page)?.as_ref()?.hit(inside)?;
        Some((page, glyph))
    }

    fn highlight(
        &self,
        items: &mut Vec<Item>,
        page: usize,
        rect: Rect,
        range: Range<usize>,
        color: Color,
    ) {
        let Some(text) = self.texts.get(page).and_then(Option::as_ref) else {
            return;
        };
        let end = range.end.min(text.len());
        for glyph in text
            .rects
            .get(range.start.min(end)..end)
            .unwrap_or_default()
        {
            let [left, top, right, bottom] = *glyph;
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    rect.x + left * self.scale,
                    rect.y + top * self.scale,
                    (right - left) * self.scale,
                    (bottom - top) * self.scale,
                )),
                paint: color.into(),
            });
        }
    }

    fn paint_page(&self, cx: &Context, page: usize, rect: Rect, items: &mut Vec<Item>) {
        items.extend(Item::shadows(
            Shape::rect(rect),
            bande_toolkit::Elevation::Frame,
            cx.palette.scheme,
            true,
        ));
        let mut inner = vec![Item::Fill {
            shape: Shape::rect(rect),
            paint: Color::WHITE.into(),
        }];
        for shown in [self.bases.get(&page), self.details.get(&page)]
            .into_iter()
            .flatten()
        {
            let unit = self.scale / shown.render.scale;
            inner.push(Item::Image {
                rect: Rect::new(
                    rect.x + shown.render.x as f32 * unit,
                    rect.y + shown.render.y as f32 * unit,
                    shown.render.width as f32 * unit,
                    shown.render.height as f32 * unit,
                ),
                image: shown.image.clone(),
            });
        }
        let first = self.matches.partition_point(|(other, _)| *other < page);
        let on_page = self.matches[first..]
            .iter()
            .take_while(|(other, _)| *other == page);
        for (index, (_, range)) in (first..).zip(on_page) {
            let color = match self.current == Some(index) {
                true => CURRENT_MATCH,
                false => MATCH,
            };
            self.highlight(&mut inner, page, rect, range.clone(), color);
        }
        if let Some((start, end)) = self.ordered_selection()
            && (start.0..=end.0).contains(&page)
        {
            let first = if page == start.0 { start.1 } else { 0 };
            let last = if page == end.0 { end.1 + 1 } else { usize::MAX };
            self.highlight(&mut inner, page, rect, first..last, SELECTION);
        }
        items.push(Item::Clip {
            shape: Shape::rect(rect),
            items: inner,
        });
    }

    fn wheel_zoom(&mut self, scroll: &Scroll) -> bool {
        let factor = (-scroll.delta.y / WHEEL_ZOOM).exp();
        self.hold(scroll.position);
        self.fit = Fit::Scale((self.scale * factor).clamp(MIN_SCALE, MAX_SCALE));
        true
    }

    fn pan_sideways(&mut self, delta: f32) -> bool {
        let limit = (self.content_width() - self.frame.width).max(0.0);
        let before = self.sideways;
        self.sideways = (self.sideways + delta).clamp(0.0, limit);
        self.sideways != before
    }

    fn scroll_by(&mut self, cx: &Context, delta: f32) -> bool {
        let offset = self.scroll.offset(cx);
        self.scroll.scroll_to(cx, offset + delta)
    }
}

impl Widget for PdfView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.display_scale.set(cx.display_scale as f32);
        self.lay_out();
        self.scroll.set_extent(self.content_height(), frame.height);
        if let Some((fraction, inside)) = self.across.take() {
            self.sideways = fraction * self.content_width() - inside;
        }
        self.pan_sideways(0.0);
        if let Some((page, within, at)) = self.anchor.take()
            && let Some(top) = self.tops.get(page)
        {
            let target = top + within * self.scale - (at - frame.y);
            self.scroll.follow(cx, target, target + frame.height);
        }
        if let Some((page, within)) = self.jump.take()
            && let Some(top) = self.tops.get(page)
        {
            let target = (top + within * self.scale - GAP).max(0.0);
            self.scroll.follow(cx, target, target + frame.height);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let offset = self.scroll.offset(cx);
        self.viewport.set((offset, self.frame.height));
        let first = self
            .tops
            .partition_point(|top| *top <= offset)
            .saturating_sub(1);
        let last = self
            .tops
            .partition_point(|top| *top < offset + self.frame.height);
        self.visible.set((first, last.max(first)));
        let mut content = vec![Item::Fill {
            shape: Shape::rect(self.frame),
            paint: cx.palette.well.into(),
        }];
        for page in first..last {
            if let Some(rect) = self.page_rect(page, offset) {
                self.paint_page(cx, page, rect, &mut content);
            }
        }
        items.push(Item::Clip {
            shape: Shape::rounded(self.frame, self.radius),
            items: content,
        });
        self.scroll.paint_scrollbar(cx, self.frame, items);
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::Document);
        let (page, count) = self.page_label();
        if count > 0 {
            node.set_label(format!("{}, {page} / {count}", self.label));
        }
        if !self.query.is_empty() {
            let (current, total) = self.matches();
            node.set_description(format!(
                "{} / {total}",
                current.map_or(0, |index| index + 1)
            ));
        }
        self.scroll.describe(cx, &mut node);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        if self.scroll.pointer(cx, self.frame, event) {
            return true;
        }
        match event {
            Event::Scroll(scroll) if scroll.modifiers.control => self.wheel_zoom(scroll),
            Event::Scroll(scroll) => {
                let sideways = self.pan_sideways(-scroll.delta.x);
                let reversed = Scroll {
                    delta: Point::new(0.0, -scroll.delta.y),
                    ..*scroll
                };
                self.scroll.scroll(cx, &reversed) || sideways
            }
            Event::PointerDown(pointer) if pointer.button == PointerButton::Primary => {
                let spot = self.spot(pointer.position);
                self.selection = spot.map(|spot| (spot, spot));
                self.selecting = spot.is_some();
                true
            }
            Event::PointerMove(pointer) if self.selecting => {
                if let (Some(spot), Some((anchor, _))) =
                    (self.spot(pointer.position), self.selection)
                {
                    self.selection = Some((anchor, spot));
                }
                true
            }
            Event::PointerUp(_) if self.selecting => {
                self.selecting = false;
                if self
                    .selection
                    .is_some_and(|(anchor, focus)| anchor == focus)
                {
                    self.selection = None;
                }
                true
            }
            Event::Key { key, modifiers } if !modifiers.control && !modifiers.alt => match key {
                Key::Space if modifiers.shift => self.scroll_by(cx, -self.frame.height * 0.9),
                Key::Space | Key::PageDown => self.scroll_by(cx, self.frame.height * 0.9),
                Key::PageUp => self.scroll_by(cx, -self.frame.height * 0.9),
                Key::Down => self.scroll_by(cx, LINE),
                Key::Up => self.scroll_by(cx, -LINE),
                Key::Home => self.scroll.scroll_to(cx, 0.0),
                Key::End => self.scroll.scroll_to(cx, self.content_height()),
                _ => false,
            },
            _ => false,
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        let offset = self.viewport.get().0;
        let on_page = self.visible_pages().any(|page| {
            self.page_rect(page, offset)
                .is_some_and(|rect| rect.contains(point))
        });
        Some(match on_page {
            true => Cursor::Text,
            false => Cursor::Default,
        })
    }
}
