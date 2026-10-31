use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, FreeLayout, SEGMENTED_HEIGHT, SearchField, SegmentedControl,
    TEXT_FIELD_HEIGHT, WidgetId,
};
use bande_toolkit::{Image, Rect, WINDOW_HEADER_HEIGHT, window_controls_area};

use super::{L10N, Mode, Signal, Signals, sender};

pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const SIDE: f32 = 12.0;
const GAP: f32 = 8.0;
const SYMBOL_BUTTON: f32 = 28.0;
const TODAY_WIDTH: f32 = 64.0;
const SEGMENT_WIDTH: f32 = 64.0;
const SEARCH_WIDTH: f32 = 180.0;
const SYMBOL_PIXELS: u32 = 16;

const PLUS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>"##;
const LEFT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M10 3.5 5.5 8l4.5 4.5" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;
const RIGHT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="m6 3.5 4.5 4.5L6 12.5" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;

pub struct Toolbar {
    pub add: WidgetId,
    pub previous: WidgetId,
    pub today: WidgetId,
    pub next: WidgetId,
    pub modes: WidgetId,
    pub search: WidgetId,
    pub add_account: WidgetId,
}

fn symbol(svg: &str, label: &str, on_click: impl FnMut() + 'static) -> Button {
    match Image::from_svg(svg.as_bytes(), SYMBOL_PIXELS, SYMBOL_PIXELS) {
        Some(image) => Button::symbol(image, label, on_click),
        None => Button::new(label, on_click),
    }
}

impl Toolbar {
    pub fn new(host: &mut FreeLayout, signals: &Signals, mode: Mode) -> Self {
        let send = sender(signals);
        let add = host.insert_in_header(
            symbol(PLUS, &L10N.tr("add-event"), move || send(Signal::NewEvent))
                .tooltip(&L10N.tr("add-event")),
            Rect::default(),
        );
        let send = sender(signals);
        let previous = host.insert_in_header(
            symbol(LEFT, &L10N.tr("previous"), move || send(Signal::Previous)),
            Rect::default(),
        );
        let send = sender(signals);
        let today = host.insert_in_header(
            Button::new(&L10N.tr("today"), move || send(Signal::Today)),
            Rect::default(),
        );
        let send = sender(signals);
        let next = host.insert_in_header(
            symbol(RIGHT, &L10N.tr("next"), move || send(Signal::Next)),
            Rect::default(),
        );
        let send = sender(signals);
        let titles: Vec<String> = Mode::ALL.iter().map(|mode| mode.title()).collect();
        let selected = Mode::ALL.iter().position(|shown| *shown == mode);
        let modes = host.insert_in_header(
            SegmentedControl::new(&L10N.tr("view-mode"), &titles, selected, move |index| {
                send(Signal::Mode(index))
            }),
            Rect::default(),
        );
        let send = sender(signals);
        let search = host.insert_in_header(
            SearchField::new(&L10N.tr("search"), "", move |text| {
                send(Signal::Search(text.to_owned()))
            }),
            Rect::default(),
        );
        let send = sender(signals);
        let add_account = host.insert(
            Button::new(&L10N.tr("add-account"), move || send(Signal::AddAccount)).quiet(),
            Rect::default(),
        );
        Self {
            add,
            previous,
            today,
            next,
            modes,
            search,
            add_account,
        }
    }

    fn leading(&self) -> f32 {
        window_controls_area(TOOLBAR_HEIGHT).width + SIDE
    }

    pub fn title_x(&self) -> f32 {
        self.leading() + SYMBOL_BUTTON + 2.0 * GAP
    }

    pub fn frames(&self, width: f32) -> [(WidgetId, Rect); 6] {
        let middle = |height: f32| ((TOOLBAR_HEIGHT - height) / 2.0).round();
        let search = Rect::new(
            width - SIDE - SEARCH_WIDTH,
            middle(TEXT_FIELD_HEIGHT),
            SEARCH_WIDTH,
            TEXT_FIELD_HEIGHT,
        );
        let modes_width = SEGMENT_WIDTH * Mode::ALL.len() as f32;
        let modes = Rect::new(
            search.x - 2.0 * GAP - modes_width,
            middle(SEGMENTED_HEIGHT),
            modes_width,
            SEGMENTED_HEIGHT,
        );
        let next = Rect::new(
            modes.x - 2.0 * GAP - SYMBOL_BUTTON,
            middle(SYMBOL_BUTTON),
            SYMBOL_BUTTON,
            SYMBOL_BUTTON,
        );
        let today = Rect::new(
            next.x - TODAY_WIDTH - 2.0,
            middle(BUTTON_HEIGHT),
            TODAY_WIDTH,
            BUTTON_HEIGHT,
        );
        let previous = Rect::new(
            today.x - 2.0 - SYMBOL_BUTTON,
            middle(SYMBOL_BUTTON),
            SYMBOL_BUTTON,
            SYMBOL_BUTTON,
        );
        let add = Rect::new(
            self.leading(),
            middle(SYMBOL_BUTTON),
            SYMBOL_BUTTON,
            SYMBOL_BUTTON,
        );
        [
            (self.add, add),
            (self.previous, previous),
            (self.today, today),
            (self.next, next),
            (self.modes, modes),
            (self.search, search),
        ]
    }
}
