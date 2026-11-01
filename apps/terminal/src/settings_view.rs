use std::cell::{Cell, RefCell};
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Context, Form, FormLine, PopUpButton, ScrollView, SegmentedControl, Size, Stepper, Toggle, Ui,
    Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, Item, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle, TextSystem,
};

use crate::L10N;
use crate::settings::{CursorStyle, FONT_SIZES, Settings};
use crate::theme::{Profile, Theme, ansi_colors};

pub const SETTINGS_SIZE: (u32, u32) = (500, 480);
const PREVIEW: f32 = 76.0;
const SWATCH: f32 = 14.0;
const SWATCH_GAP: f32 = 4.0;
const PREVIEW_PADDING: f32 = 12.0;

struct Preview {
    id: WidgetId,
    profile: Rc<Cell<Profile>>,
    frame: Rect,
    items: Vec<Item>,
}

impl Widget for Preview {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, PREVIEW + cx.grown_line())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let appearance = Appearance {
            scheme: cx.palette.scheme,
            ..Appearance::default()
        };
        let theme = Theme::new(appearance, self.profile.get());
        let rect = Rect::new(frame.x, frame.y + 4.0, frame.width, frame.height - 8.0);
        let sample = cx.text.paragraph(
            &L10N.tr("settings-preview-sample"),
            TextStyle::role(TextRole::Mono, theme.foreground),
            None,
            TextAlign::Start,
        );
        self.items = vec![
            Item::Fill {
                shape: Shape::rounded(rect, Radius::S),
                paint: theme.background.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(rect.inset(0.5), Radius::S - 0.5),
                width: 1.0,
                paint: cx.palette.hairline.into(),
            },
        ];
        self.items.extend(
            ansi_colors(&theme.palette)
                .iter()
                .enumerate()
                .map(|(index, color)| {
                    let x = rect.x + PREVIEW_PADDING + index as f32 * (SWATCH + SWATCH_GAP);
                    let y = rect.y + rect.height - PREVIEW_PADDING - SWATCH;
                    Item::Fill {
                        shape: Shape::rounded(Rect::new(x, y, SWATCH, SWATCH), 2.0),
                        paint: (*color).into(),
                    }
                }),
        );
        self.items.push(Item::Text {
            origin: Point::new(rect.x + PREVIEW_PADDING, rect.y + PREVIEW_PADDING),
            paragraph: sample,
        });
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.extend(self.items.iter().cloned());
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Image);
        node.set_label(L10N.tr("settings-preview-sample"));
        node
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
    let heading = |message: &str| FormLine::Heading(L10N.tr(message));
    let row = |message: &str, control: Box<dyn Widget>| FormLine::Row(L10N.tr(message), control);
    Form::new(
        &L10N.tr("settings-title"),
        vec![
            heading("settings-appearance"),
            row("settings-profile", Box::new(profile_button)),
            FormLine::Wide(Box::new(Preview {
                id: WidgetId::next(),
                profile,
                frame: Rect::default(),
                items: Vec::new(),
            })),
            row("settings-font-size", Box::new(size)),
            row("settings-cursor", Box::new(cursor)),
            heading("settings-bell"),
            row("settings-flash", Box::new(flash)),
            row("settings-badge", Box::new(badge)),
            heading("settings-notifications"),
            row("settings-notify", Box::new(notify)),
        ],
    )
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
