use std::cell::{Cell, RefCell};
use std::rc::Rc;

use accesskit::{Node, Role};
use unbound_toolkit::ui::{
    Context, PopUpButton, ScrollView, SegmentedControl, Size, Stepper, Toggle, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Appearance, Color, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
    TextSystem,
};

use crate::L10N;
use crate::settings::{CursorStyle, FONT_SIZES, Settings};
use crate::theme::{Profile, Theme, ansi_colors};

pub const SETTINGS_SIZE: (u32, u32) = (500, 480);
const MARGIN: f32 = 24.0;
const ROW: f32 = 44.0;
const HEADING: f32 = 32.0;
const PREVIEW: f32 = 76.0;
const SWATCH: f32 = 14.0;
const SWATCH_GAP: f32 = 4.0;
const PREVIEW_PADDING: f32 = 12.0;
const ROW_PADDING: f32 = 8.0;
const LABEL_SHARE: f32 = 0.55;

enum Line {
    Heading(String),
    Row(String),
    Preview,
}

struct Form {
    id: WidgetId,
    lines: Vec<Line>,
    children: Vec<Box<dyn Widget>>,
    profile: Rc<Cell<Profile>>,
    texts: Vec<(Point, Paragraph)>,
    heights: Vec<f32>,
    preview: Option<Preview>,
    frame: Rect,
}

struct Preview {
    rect: Rect,
    background: Color,
    edge: Color,
    swatches: Vec<(Rect, Color)>,
}

fn row_label(cx: &mut Context, text: &str, width: f32) -> Paragraph {
    cx.text.paragraph(
        text,
        TextStyle::role(TextRole::Body, cx.palette.ink),
        Some(width * LABEL_SHARE),
        TextAlign::Start,
    )
}

impl Widget for Form {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        for child in &mut self.children {
            child.measure(cx, available);
        }
        let grown = cx.grown_line();
        let width = available.width - 2.0 * MARGIN;
        self.heights = self
            .lines
            .iter()
            .map(|line| match line {
                Line::Heading(_) => HEADING + grown,
                Line::Row(text) => {
                    let label = row_label(cx, text, width);
                    (ROW + grown).max((label.height() + 2.0 * ROW_PADDING).ceil())
                }
                Line::Preview => PREVIEW + grown,
            })
            .collect();
        Size::new(available.width, self.heights.iter().sum::<f32>() + MARGIN)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.texts.clear();
        self.preview = None;
        let width = frame.width - 2.0 * MARGIN;
        let mut y = frame.y + MARGIN / 2.0;
        let mut children = self.children.iter_mut();
        for (line, &height) in self.lines.iter().zip(&self.heights) {
            match line {
                Line::Heading(text) => {
                    let paragraph = cx.text.paragraph(
                        text,
                        TextStyle::role(TextRole::Label, cx.palette.ink3),
                        None,
                        TextAlign::Start,
                    );
                    let origin = Point::new(
                        frame.x + MARGIN,
                        (y + height - paragraph.height() - 6.0).round(),
                    );
                    self.texts.push((origin, paragraph));
                    y += height;
                }
                Line::Row(text) => {
                    let paragraph = row_label(cx, text, width);
                    let origin = Point::new(
                        frame.x + MARGIN,
                        (y + (height - paragraph.height()) / 2.0).round(),
                    );
                    self.texts.push((origin, paragraph));
                    if let Some(child) = children.next() {
                        let size =
                            child.measure(cx, Size::new(width * (1.0 - LABEL_SHARE), height));
                        child.place(
                            cx,
                            Rect::new(
                                frame.x + frame.width - MARGIN - size.width,
                                (y + (height - size.height) / 2.0).round(),
                                size.width,
                                size.height,
                            ),
                        );
                    }
                    y += height;
                }
                Line::Preview => {
                    let appearance = Appearance {
                        scheme: cx.palette.scheme,
                        ..Appearance::default()
                    };
                    let theme = Theme::new(appearance, self.profile.get());
                    let rect = Rect::new(frame.x + MARGIN, y + 4.0, width, height - 8.0);
                    let sample = cx.text.paragraph(
                        &L10N.tr("settings-preview-sample"),
                        TextStyle::role(TextRole::Mono, theme.foreground),
                        None,
                        TextAlign::Start,
                    );
                    self.texts.push((
                        Point::new(rect.x + PREVIEW_PADDING, rect.y + PREVIEW_PADDING),
                        sample,
                    ));
                    let colors = ansi_colors(&theme.palette);
                    let swatches = colors
                        .iter()
                        .enumerate()
                        .map(|(index, color)| {
                            let x = rect.x + PREVIEW_PADDING + index as f32 * (SWATCH + SWATCH_GAP);
                            let y = rect.y + rect.height - PREVIEW_PADDING - SWATCH;
                            (Rect::new(x, y, SWATCH, SWATCH), *color)
                        })
                        .collect();
                    self.preview = Some(Preview {
                        rect,
                        background: theme.background,
                        edge: cx.palette.hairline,
                        swatches,
                    });
                    y += height;
                }
            }
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if let Some(preview) = &self.preview {
            items.push(Item::Fill {
                shape: Shape::rounded(preview.rect, Radius::S),
                paint: preview.background.into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(preview.rect.inset(0.5), Radius::S - 0.5),
                width: 1.0,
                paint: preview.edge.into(),
            });
            for (rect, color) in &preview.swatches {
                items.push(Item::Fill {
                    shape: Shape::rounded(*rect, 2.0),
                    paint: (*color).into(),
                });
            }
        }
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(L10N.tr("settings-title"));
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub type SharedSettings = Rc<RefCell<Settings>>;

fn toggle(
    settings: &SharedSettings,
    label: &str,
    on: bool,
    set: fn(&mut Settings, bool),
) -> Toggle {
    let settings = settings.clone();
    Toggle::new(label, on, move |on| set(&mut settings.borrow_mut(), on))
}

fn form(settings: &SharedSettings, profile: Rc<Cell<Profile>>) -> Form {
    let current = *settings.borrow();
    let profiles: Vec<String> = Profile::ALL
        .iter()
        .map(|profile| L10N.tr(profile.message()))
        .collect();
    let chosen = Profile::ALL
        .iter()
        .position(|candidate| *candidate == current.profile)
        .unwrap_or_default();
    let shared = settings.clone();
    let shown = profile.clone();
    let profile_button = PopUpButton::new(
        &L10N.tr("settings-profile"),
        &profiles,
        chosen,
        move |index| {
            let profile = Profile::ALL.get(index).copied().unwrap_or_default();
            shown.set(profile);
            shared.borrow_mut().profile = profile;
        },
    );
    let shared = settings.clone();
    let size = Stepper::new(
        &L10N.tr("settings-font-size"),
        f64::from(current.font_size),
        (f64::from(FONT_SIZES.0), f64::from(FONT_SIZES.1)),
        1.0,
        move |value| shared.borrow_mut().font_size = value as f32,
    )
    .format(|value| L10N.tr_args("settings-points", [("size", (value as i64).into())]));
    let cursors: Vec<String> = CursorStyle::ALL
        .iter()
        .map(|style| L10N.tr(style.message()))
        .collect();
    let shared = settings.clone();
    let cursor = SegmentedControl::new(
        &L10N.tr("settings-cursor"),
        &cursors,
        CursorStyle::ALL
            .iter()
            .position(|style| *style == current.cursor),
        move |index| {
            shared.borrow_mut().cursor = CursorStyle::ALL.get(index).copied().unwrap_or_default();
        },
    );
    let flash = toggle(
        settings,
        &L10N.tr("settings-flash"),
        current.flash_on_bell,
        |settings, on| settings.flash_on_bell = on,
    );
    let badge = toggle(
        settings,
        &L10N.tr("settings-badge"),
        current.badge_on_bell,
        |settings, on| settings.badge_on_bell = on,
    );
    let notify = toggle(
        settings,
        &L10N.tr("settings-notify"),
        current.notify_finished,
        |settings, on| settings.notify_finished = on,
    );
    Form {
        id: WidgetId::next(),
        lines: vec![
            Line::Heading(L10N.tr("settings-appearance")),
            Line::Row(L10N.tr("settings-profile")),
            Line::Preview,
            Line::Row(L10N.tr("settings-font-size")),
            Line::Row(L10N.tr("settings-cursor")),
            Line::Heading(L10N.tr("settings-bell")),
            Line::Row(L10N.tr("settings-flash")),
            Line::Row(L10N.tr("settings-badge")),
            Line::Heading(L10N.tr("settings-notifications")),
            Line::Row(L10N.tr("settings-notify")),
        ],
        children: vec![
            Box::new(profile_button),
            Box::new(size),
            Box::new(cursor),
            Box::new(flash),
            Box::new(badge),
            Box::new(notify),
        ],
        profile,
        texts: Vec::new(),
        heights: Vec::new(),
        preview: None,
        frame: Rect::default(),
    }
}

pub fn settings_view(appearance: Appearance, settings: &SharedSettings) -> Ui {
    let profile = Rc::new(Cell::new(settings.borrow().profile));
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form(settings, profile))),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
