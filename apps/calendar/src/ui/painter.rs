use unbound_toolkit::{Color, Item, Paint, Point, Rect, Shape, TextAlign, TextStyle, TextSystem};

pub struct Painter<'a, H> {
    pub text: &'a mut TextSystem,
    pub items: Vec<Item>,
    pub hits: Vec<(Rect, H)>,
}

impl<'a, H> Painter<'a, H> {
    pub fn new(text: &'a mut TextSystem) -> Self {
        Self {
            text,
            items: Vec::new(),
            hits: Vec::new(),
        }
    }

    pub fn fill(&mut self, rect: Rect, radius: f32, color: Color) {
        self.items.push(Item::Fill {
            shape: Shape::rounded(rect, radius),
            paint: Paint::Solid(color),
        });
    }

    pub fn stroke(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.items.push(Item::Stroke {
            shape: Shape::rounded(rect, radius),
            width,
            paint: Paint::Solid(color),
        });
    }

    pub fn line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        self.items.push(Item::Line {
            from,
            to,
            width,
            paint: Paint::Solid(color),
        });
    }

    pub fn measure(&mut self, text: &str, style: TextStyle) -> f32 {
        self.text
            .paragraph(text, style, None, TextAlign::Start)
            .width()
    }

    pub fn label(&mut self, text: &str, style: TextStyle, frame: Rect, align: TextAlign) {
        let paragraph = self.text.paragraph(text, style, None, align);
        let x = match align {
            TextAlign::Center => frame.x + ((frame.width - paragraph.width()) / 2.0).round(),
            TextAlign::End => frame.x + frame.width - paragraph.width(),
            _ => frame.x,
        };
        let y = frame.y + ((frame.height - paragraph.height()) / 2.0).round();
        self.items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Text {
                origin: Point::new(x, y),
                paragraph,
            }],
        });
    }

    pub fn wrapped(&mut self, text: &str, style: TextStyle, frame: Rect) {
        let paragraph = self
            .text
            .paragraph(text, style, Some(frame.width), TextAlign::Start);
        self.items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Text {
                origin: Point::new(frame.x, frame.y),
                paragraph,
            }],
        });
    }

    pub fn hit(&mut self, rect: Rect, hit: H) {
        self.hits.push((rect, hit));
    }

    pub fn clipped(&mut self, shape: Shape, paint: impl FnOnce(&mut Self)) {
        let outer = std::mem::take(&mut self.items);
        paint(self);
        let inner = std::mem::replace(&mut self.items, outer);
        self.items.push(Item::Clip {
            shape,
            items: inner,
        });
    }
}

pub fn parse_color(text: &str) -> Option<Color> {
    let hex = text.trim().trim_start_matches('#');
    let channel = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
    match hex.len() {
        6 | 8 => Some(Color {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: 255,
        }),
        _ => None,
    }
}

pub fn color_text(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_server_colours_with_and_without_alpha() {
        assert_eq!(parse_color("#FF2968FF"), Some(Color::hex(0xFF2968)));
        assert_eq!(parse_color("#34c759"), Some(Color::hex(0x34C759)));
        assert_eq!(parse_color("red"), None);
        assert_eq!(color_text(Color::hex(0x0A84FF)), "#0A84FF");
    }
}
