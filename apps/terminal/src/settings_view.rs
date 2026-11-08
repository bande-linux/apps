use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::{Text, text, tr};

use crate::settings::{
    COLUMN_RANGE, CloseAsk, CursorStyle, FONT_SIZES, FONTS, NewTabFolder, ROW_RANGE,
    SCROLLBACK_CHOICES, Settings,
};
use accesskit::{Node, Role};
use bande_terminal_view::{Profile, Theme};
use bande_toolkit::ui::{
    Context, Form, FormLine, PopUpButton, ScrollView, SegmentedControl, Size, Stepper, TextField,
    Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, Item, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle, TextSystem,
};

pub const SETTINGS_SIZE: (u32, u32) = (520, 600);
const PREVIEW: f32 = 76.0;
const SWATCH: f32 = 14.0;
const SWATCH_GAP: f32 = 4.0;
const PREVIEW_PADDING: f32 = 12.0;

struct Preview {
    id: WidgetId,
    settings: SharedSettings,
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
        let (profile, font) = {
            let settings = self.settings.borrow();
            (settings.profile, settings.font.clone())
        };
        let theme = Theme::new(appearance, profile);
        cx.text
            .set_monospace_family((!font.is_empty()).then_some(font.as_str()));
        let rect = Rect::new(frame.x, frame.y + 4.0, frame.width, frame.height - 8.0);
        let sample = cx.text.paragraph(
            &tr!("~/bande $ cargo build --release"),
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
        self.items
            .extend(theme.ansi().iter().enumerate().map(|(index, color)| {
                let x = rect.x + PREVIEW_PADDING + index as f32 * (SWATCH + SWATCH_GAP);
                let y = rect.y + rect.height - PREVIEW_PADDING - SWATCH;
                Item::Fill {
                    shape: Shape::rounded(Rect::new(x, y, SWATCH, SWATCH), 2.0),
                    paint: (*color).into(),
                }
            }));
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
        node.set_label(tr!("~/bande $ cargo build --release"));
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

fn grouped(number: usize) -> String {
    let digits = number.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn pop_up<T: Copy + PartialEq + 'static>(
    settings: &SharedSettings,
    label: &str,
    choices: &[T],
    title: impl Fn(T) -> String,
    current: T,
    set: fn(&mut Settings, T),
) -> PopUpButton {
    let titles: Vec<String> = choices.iter().map(|choice| title(*choice)).collect();
    let chosen = choices
        .iter()
        .position(|choice| *choice == current)
        .unwrap_or_default();
    let shared = settings.clone();
    let choices = choices.to_vec();
    PopUpButton::new(label, &titles, chosen, move |index| {
        if let Some(choice) = choices.get(index) {
            set(&mut shared.borrow_mut(), *choice);
        }
    })
}

fn form(settings: &SharedSettings, fonts: &[String]) -> Form {
    let current = settings.borrow().clone();
    let profiles: Vec<String> = Profile::ALL
        .iter()
        .map(|profile| (profile.message()).tr())
        .collect();
    let chosen = Profile::ALL
        .iter()
        .position(|candidate| *candidate == current.profile)
        .unwrap_or_default();
    let shared = settings.clone();
    let profile_button = PopUpButton::new(&tr!("Profile"), &profiles, chosen, move |index| {
        let profile = Profile::ALL.get(index).copied().unwrap_or_default();
        shared.borrow_mut().profile = profile;
    });
    let shared = settings.clone();
    let families = fonts.to_vec();
    let font = PopUpButton::new(
        &tr!("Font"),
        fonts,
        fonts
            .iter()
            .position(|family| *family == current.font)
            .unwrap_or_default(),
        move |index| {
            shared.borrow_mut().font = match index {
                0 => String::new(),
                index => families.get(index).cloned().unwrap_or_default(),
            };
        },
    );
    let shared = settings.clone();
    let size = Stepper::new(
        &tr!("Text size"),
        f64::from(current.font_size),
        (f64::from(FONT_SIZES.0), f64::from(FONT_SIZES.1)),
        1.0,
        move |value| shared.borrow_mut().font_size = value as f32,
    )
    .format(|value| tr!("{size} pt", size = (value as i64)));
    let shared = settings.clone();
    let columns = Stepper::new(
        &tr!("Columns"),
        current.columns as f64,
        (COLUMN_RANGE.0 as f64, COLUMN_RANGE.1 as f64),
        1.0,
        move |value| shared.borrow_mut().columns = value as usize,
    );
    let shared = settings.clone();
    let rows = Stepper::new(
        &tr!("Rows"),
        current.rows as f64,
        (ROW_RANGE.0 as f64, ROW_RANGE.1 as f64),
        1.0,
        move |value| shared.borrow_mut().rows = value as usize,
    );
    let cursors: Vec<String> = CursorStyle::ALL
        .iter()
        .map(|style| (style.message()).tr())
        .collect();
    let shared = settings.clone();
    let cursor = SegmentedControl::new(
        &tr!("Cursor"),
        &cursors,
        CursorStyle::ALL
            .iter()
            .position(|style| *style == current.cursor),
        move |index| {
            shared.borrow_mut().cursor = CursorStyle::ALL.get(index).copied().unwrap_or_default();
        },
    );
    let audible = toggle(
        settings,
        &tr!("Play the alert sound"),
        current.audible_bell,
        |settings, on| settings.audible_bell = on,
    );
    let flash = toggle(
        settings,
        &tr!("Flash the window"),
        current.flash_on_bell,
        |settings, on| settings.flash_on_bell = on,
    );
    let badge = toggle(
        settings,
        &tr!("Show a badge in the bar"),
        current.badge_on_bell,
        |settings, on| settings.badge_on_bell = on,
    );
    let notify = toggle(
        settings,
        &tr!("Notify when long commands finish"),
        current.notify_finished,
        |settings, on| settings.notify_finished = on,
    );
    let blink = toggle(
        settings,
        &tr!("Blink the cursor"),
        current.blink,
        |settings, on| settings.blink = on,
    );
    let shared = settings.clone();
    let shell = TextField::new(&tr!("Login shell"), &current.shell, move |text| {
        shared.borrow_mut().shell = text.to_owned()
    })
    .without_text_checking();
    let folder = pop_up(
        settings,
        &tr!("New tabs and windows open in"),
        &NewTabFolder::ALL,
        |folder| (folder.message()).tr(),
        current.new_tab_folder,
        |settings, folder| settings.new_tab_folder = folder,
    );
    let close = pop_up(
        settings,
        &tr!("Ask before closing"),
        &CloseAsk::ALL,
        |ask| (ask.message()).tr(),
        current.close_ask,
        |settings, ask| settings.close_ask = ask,
    );
    let scrollback = pop_up(
        settings,
        &tr!("Scrollback"),
        &SCROLLBACK_CHOICES,
        |lines| tr!("{count} lines", count = grouped(lines)),
        current.scrollback,
        |settings, lines| settings.scrollback = lines,
    );
    let meta = toggle(
        settings,
        &tr!("Use Alt as Meta"),
        current.alt_as_meta,
        |settings, on| settings.alt_as_meta = on,
    );
    let paste = toggle(
        settings,
        &tr!("Warn before pasting several lines"),
        current.paste_warning,
        |settings, on| settings.paste_warning = on,
    );
    let heading = |message: Text| FormLine::Heading(message.tr());
    let row = |message: Text, control: Box<dyn Widget>| FormLine::Row(message.tr(), control);
    Form::new(
        &tr!("Terminal Settings"),
        vec![
            heading(text!("Appearance")),
            row(text!("Profile"), Box::new(profile_button)),
            FormLine::Wide(Box::new(Preview {
                id: WidgetId::next(),
                settings: settings.clone(),
                frame: Rect::default(),
                items: Vec::new(),
            })),
            row(text!("Font"), Box::new(font)),
            row(text!("Text size"), Box::new(size)),
            row(text!("Columns"), Box::new(columns)),
            row(text!("Rows"), Box::new(rows)),
            row(text!("Cursor"), Box::new(cursor)),
            row(text!("Blink the cursor"), Box::new(blink)),
            heading(text!("Shell")),
            row(text!("Run command"), Box::new(shell)),
            row(text!("New tabs and windows open in"), Box::new(folder)),
            row(text!("Ask before closing"), Box::new(close)),
            heading(text!("Keyboard and Paste")),
            row(text!("Use Alt as Meta"), Box::new(meta)),
            row(text!("Warn before pasting several lines"), Box::new(paste)),
            row(text!("Scrollback"), Box::new(scrollback)),
            heading(text!("Bell")),
            row(text!("Play the alert sound"), Box::new(audible)),
            row(text!("Flash the window"), Box::new(flash)),
            row(text!("Show a badge in the bar"), Box::new(badge)),
            heading(text!("Notifications")),
            row(text!("Notify when long commands finish"), Box::new(notify)),
        ],
    )
}

pub fn settings_view(appearance: Appearance, settings: &SharedSettings) -> Ui {
    let mut text = TextSystem::new();
    let mut fonts: Vec<String> = FONTS
        .iter()
        .enumerate()
        .filter(|(index, family)| *index == 0 || text.has_family(family))
        .map(|(_, family)| (*family).to_owned())
        .collect();
    let chosen = settings.borrow().font.clone();
    if !chosen.is_empty() && !fonts.contains(&chosen) {
        fonts.push(chosen);
    }
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form(settings, &fonts))),
        Context::new(text, appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
