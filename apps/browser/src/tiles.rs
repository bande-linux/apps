use std::sync::Arc;

use bande_toolkit::{Color, GlyphRun, Image, Item, Picture, PositionedGlyph, Rect, Shape};
use web_paint::DisplayList;

const TILE_HEIGHT: f32 = 1024.0;
const GLYPH_REACH: f32 = 0.5;

enum Node {
    Leaf {
        top: f32,
        bottom: f32,
        item: Item,
    },
    Image {
        top: f32,
        bottom: f32,
        rect: Rect,
        clip: Rect,
        image: usize,
    },
    Group {
        top: f32,
        bottom: f32,
        group: Group,
        children: Vec<Node>,
    },
}

#[derive(Clone, Copy)]
enum Group {
    Clip(Rect),
    Opacity(f32),
}

impl Node {
    fn span(&self) -> (f32, f32) {
        match self {
            Self::Leaf { top, bottom, .. }
            | Self::Image { top, bottom, .. }
            | Self::Group { top, bottom, .. } => (*top, *bottom),
        }
    }
}

#[derive(Default)]
pub(crate) struct Images {
    uploaded: Vec<(Arc<web_paint::Image>, Image)>,
}

impl Images {
    fn get(&mut self, image: &Arc<web_paint::Image>) -> Option<Image> {
        if let Some((_, uploaded)) = self
            .uploaded
            .iter()
            .find(|(source, _)| Arc::ptr_eq(source, image))
        {
            return Some(uploaded.clone());
        }
        let uploaded = Image::from_rgba(image.width, image.height, image.rgba.to_vec())?;
        self.uploaded.push((Arc::clone(image), uploaded.clone()));
        Some(uploaded)
    }

    fn keep_only(&mut self, list: &DisplayList) {
        self.uploaded
            .retain(|(source, _)| list.images.iter().any(|kept| Arc::ptr_eq(kept, source)));
    }
}

pub(crate) struct Tiles {
    list: Arc<DisplayList>,
    nodes: Vec<Node>,
    built: Vec<Option<Picture>>,
}

impl Tiles {
    pub(crate) fn new(list: Arc<DisplayList>, images: &mut Images) -> Self {
        images.keep_only(&list);
        let nodes = tree(&list);
        let count = ((list.height / TILE_HEIGHT).ceil() as usize).max(1);
        Self {
            list,
            nodes,
            built: vec![None; count],
        }
    }

    pub(crate) fn height(&self) -> f32 {
        self.list.height
    }

    pub(crate) fn background(&self) -> Color {
        color(self.list.background)
    }

    fn range(&self, top: f32, bottom: f32) -> std::ops::RangeInclusive<usize> {
        band(top, self.built.len())..=band(bottom, self.built.len())
    }

    pub(crate) fn prepare(&mut self, top: f32, bottom: f32, images: &mut Images) {
        for index in self.range(top, bottom) {
            if self.built.get(index).is_some_and(Option::is_some) {
                continue;
            }
            let band_top = index as f32 * TILE_HEIGHT;
            let band_bottom = band_top + TILE_HEIGHT;
            let mut items = Vec::new();
            emit(
                &self.nodes,
                &self.list,
                band_top,
                band_bottom,
                images,
                &mut items,
            );
            let picture = Picture::new(vec![Item::Clip {
                shape: Shape::rect(Rect::new(0.0, band_top, self.list.width, TILE_HEIGHT)),
                items,
            }]);
            if let Some(slot) = self.built.get_mut(index) {
                *slot = Some(picture);
            }
        }
    }

    pub(crate) fn visible(&self, top: f32, bottom: f32) -> Vec<Item> {
        self.range(top, bottom)
            .filter_map(|index| self.built.get(index).cloned().flatten())
            .map(Item::Picture)
            .collect()
    }
}

fn band(y: f32, count: usize) -> usize {
    ((y / TILE_HEIGHT).floor().max(0.0) as usize).min(count.saturating_sub(1))
}

fn color(color: web_paint::Color) -> Color {
    Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    }
}

fn rect(rect: &web_paint::Rect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

fn leaf(list: &DisplayList, item: &web_paint::Item) -> Option<Node> {
    match item {
        web_paint::Item::Rect {
            rect: area,
            color: fill,
        } => Some(Node::Leaf {
            top: area.y,
            bottom: area.y + area.height,
            item: Item::Fill {
                shape: Shape::rect(rect(area)),
                paint: color(*fill).into(),
            },
        }),
        web_paint::Item::Glyphs {
            rect: area,
            baseline,
            font,
            size,
            color: ink,
            glyphs,
            ..
        } => {
            let font = list.fonts.get(*font as usize)?.clone();
            let glyphs = list
                .glyphs(glyphs)
                .iter()
                .map(|glyph| {
                    let (id, x, y) = glyph.translated(0.0, *baseline);
                    PositionedGlyph { id, x, y }
                })
                .collect();
            let reach = size * GLYPH_REACH;
            Some(Node::Leaf {
                top: area.y - reach,
                bottom: area.y + area.height + reach,
                item: Item::Glyphs(GlyphRun::new(font, *size, color(*ink), glyphs)),
            })
        }
        web_paint::Item::Image {
            rect: area,
            clip,
            image,
        } => {
            let top = area.y.max(clip.y);
            let bottom = (area.y + area.height).min(clip.y + clip.height);
            (bottom > top).then(|| Node::Image {
                top,
                bottom,
                rect: rect(area),
                clip: rect(clip),
                image: *image as usize,
            })
        }
        web_paint::Item::PushClip { .. }
        | web_paint::Item::PushOpacity { .. }
        | web_paint::Item::Pop => None,
    }
}

fn group(group: Group, children: Vec<Node>) -> Option<Node> {
    let (top, bottom) =
        children
            .iter()
            .map(Node::span)
            .reduce(|(top, bottom), (next_top, next_bottom)| {
                (top.min(next_top), bottom.max(next_bottom))
            })?;
    let (top, bottom) = match group {
        Group::Clip(clip) => (top.max(clip.y), bottom.min(clip.y + clip.height)),
        Group::Opacity(_) => (top, bottom),
    };
    (bottom > top).then_some(Node::Group {
        top,
        bottom,
        group,
        children,
    })
}

fn tree(list: &DisplayList) -> Vec<Node> {
    let mut open: Vec<(Group, Vec<Node>)> = Vec::new();
    let mut root = Vec::new();
    for item in &list.items {
        let node = match item {
            web_paint::Item::PushClip { rect: clip } => {
                open.push((Group::Clip(rect(clip)), Vec::new()));
                continue;
            }
            web_paint::Item::PushOpacity { alpha } => {
                open.push((Group::Opacity(*alpha), Vec::new()));
                continue;
            }
            web_paint::Item::Pop => match open.pop() {
                Some((kind, children)) => group(kind, children),
                None => continue,
            },
            item => leaf(list, item),
        };
        if let Some(node) = node {
            match open.last_mut() {
                Some((_, children)) => children.push(node),
                None => root.push(node),
            }
        }
    }
    while let Some((kind, children)) = open.pop() {
        if let Some(node) = group(kind, children) {
            match open.last_mut() {
                Some((_, parent)) => parent.push(node),
                None => root.push(node),
            }
        }
    }
    root
}

fn emit(
    nodes: &[Node],
    list: &DisplayList,
    band_top: f32,
    band_bottom: f32,
    images: &mut Images,
    out: &mut Vec<Item>,
) {
    for node in nodes {
        let (top, bottom) = node.span();
        if bottom < band_top || top > band_bottom {
            continue;
        }
        match node {
            Node::Leaf { item, .. } => out.push(item.clone()),
            Node::Image {
                rect, clip, image, ..
            } => {
                if let Some(image) = list.images.get(*image).and_then(|image| images.get(image)) {
                    out.push(Item::Clip {
                        shape: Shape::rect(*clip),
                        items: vec![Item::Image { rect: *rect, image }],
                    });
                }
            }
            Node::Group {
                group, children, ..
            } => {
                let mut items = Vec::new();
                emit(children, list, band_top, band_bottom, images, &mut items);
                if items.is_empty() {
                    continue;
                }
                out.push(match group {
                    Group::Clip(clip) => Item::Clip {
                        shape: Shape::rect(*clip),
                        items,
                    },
                    Group::Opacity(alpha) => Item::Opacity {
                        alpha: *alpha,
                        items,
                    },
                });
            }
        }
    }
}
