use std::cell::RefCell;
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, Context, Event, FreeLayout, Key, PointerButton, TEXT_FIELD_HEIGHT,
    TextField, Ui, WidgetId,
};
use bande_toolkit::{
    Appearance, FrameRegions, Header, Image, Item, Radius, Rect, SHEET_INSET, Scene, Shape,
    TextSystem, WINDOW_HEADER_HEIGHT, Window, WindowEvent, window_controls_end,
};
use web_content::Fonts;
use web_net::{FetchError, FetchId, Response, Url};

use crate::page::{Content, PageView};
use crate::{L10N, address};

const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const SIDE: f32 = 12.0;
const GAP: f32 = 4.0;
const SYMBOL_BUTTON: f32 = 28.0;
const SYMBOL_PIXELS: u32 = 16;
const ADDRESS_MIN: f32 = 240.0;
const ADDRESS_MAX: f32 = 680.0;

const BACK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M10 3.5 5.5 8l4.5 4.5" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;
const FORWARD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="m6 3.5 4.5 4.5L6 12.5" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;
const RELOAD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M12.6 6.2A5 5 0 1 0 13 9" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" fill="none"/><path d="M13 2.8v3.6H9.4" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;

enum Signal {
    Back,
    Forward,
    Reload,
}

type Signals = Rc<RefCell<Vec<Signal>>>;

fn symbol(svg: &str, label: &str, signals: &Signals, signal: fn() -> Signal) -> Button {
    let signals = Rc::clone(signals);
    let click = move || signals.borrow_mut().push(signal());
    match Image::from_svg(svg.as_bytes(), SYMBOL_PIXELS, SYMBOL_PIXELS) {
        Some(image) => Button::symbol(image, label, click),
        None => Button::new(label, click),
    }
    .tooltip(label)
}

pub(crate) struct Request {
    pub(crate) cancel: Option<FetchId>,
    pub(crate) load: Option<Url>,
}

#[derive(Clone, Copy)]
enum Target {
    New,
    Entry(usize),
}

struct Toolbar {
    back: WidgetId,
    forward: WidgetId,
    address: WidgetId,
    reload: WidgetId,
}

pub(crate) struct BrowserView {
    ui: Ui,
    signals: Signals,
    toolbar: Toolbar,
    page: WidgetId,
    host: WidgetId,
    entries: Vec<Url>,
    current: Option<usize>,
    pending: Option<(Url, Target)>,
    fetch: Option<FetchId>,
    request: Option<Request>,
    title: Option<String>,
    full_screen: bool,
}

impl BrowserView {
    pub(crate) fn new(appearance: Appearance, fonts: Rc<RefCell<Fonts>>) -> Self {
        let signals: Signals = Rc::default();
        let mut host = FreeLayout::new();
        let back = host.insert_in_header(
            symbol(BACK, &L10N.tr("back"), &signals, || Signal::Back),
            Rect::default(),
        );
        let forward = host.insert_in_header(
            symbol(FORWARD, &L10N.tr("forward"), &signals, || Signal::Forward),
            Rect::default(),
        );
        let address = host.insert_in_header(
            TextField::new(&L10N.tr("address-placeholder"), "", |_| {}),
            Rect::default(),
        );
        let reload = host.insert_in_header(
            symbol(RELOAD, &L10N.tr("reload"), &signals, || Signal::Reload),
            Rect::default(),
        );
        let page = host.insert(PageView::new(fonts), Rect::default());
        let host_id = bande_toolkit::ui::Widget::id(&host);
        let mut ui = Ui::new(
            Box::new(host),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        let mut view = Self {
            ui,
            signals,
            toolbar: Toolbar {
                back,
                forward,
                address,
                reload,
            },
            page,
            host: host_id,
            entries: Vec::new(),
            current: None,
            pending: None,
            fetch: None,
            request: None,
            title: None,
            full_screen: false,
        };
        view.refresh_navigation();
        view
    }

    pub(crate) fn navigate(&mut self, url: Url) {
        self.load(url, Target::New);
    }

    fn follow(&mut self, url: Url) {
        let mut without_fragment = url.clone();
        without_fragment.set_fragment(None);
        let same_document = self.current_url().is_some_and(|current| {
            let mut current = current.clone();
            current.set_fragment(None);
            current == without_fragment
        });
        if !same_document {
            self.navigate(url);
        }
    }

    fn load(&mut self, url: Url, target: Target) {
        self.request = Some(Request {
            cancel: self.fetch.take(),
            load: Some(url.clone()),
        });
        self.set_address(url.as_str());
        self.with_page(|page| page.show(Content::Loading(url.clone())));
        self.pending = Some((url, target));
        self.focus_page();
    }

    pub(crate) fn loading(&mut self, fetch: FetchId) {
        self.fetch = Some(fetch);
    }

    pub(crate) fn fetch(&self) -> Option<FetchId> {
        self.fetch
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.fetch.is_some()
    }

    pub(crate) fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    pub(crate) fn loaded(&mut self, result: Result<Response, FetchError>) {
        self.fetch = None;
        let Some((requested, target)) = self.pending.take() else {
            return;
        };
        let url = match &result {
            Ok(response) => response.url.clone(),
            Err(_) => requested.clone(),
        };
        match target {
            Target::New => {
                let next = self.current.map_or(0, |current| current + 1);
                self.entries.truncate(next);
                self.entries.push(url.clone());
                self.current = Some(next);
            }
            Target::Entry(index) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    *entry = url.clone();
                    self.current = Some(index);
                }
            }
        }
        if !self.address_focused() {
            self.set_address(url.as_str());
        }
        let title = self.with_page(|page| {
            match result {
                Ok(response) => page.show_response(response),
                Err(error) => page.show(Content::failed(requested, &error)),
            }
            page.title()
        });
        self.title = title
            .flatten()
            .or_else(|| url.host_str().map(str::to_owned));
        self.refresh_navigation();
    }

    pub(crate) fn stop(&mut self) {
        if let Some(fetch) = self.fetch.take() {
            self.request = Some(Request {
                cancel: Some(fetch),
                load: None,
            });
        }
        self.pending = None;
        let shown = self.current_url().map(|url| url.to_string());
        self.set_address(shown.as_deref().unwrap_or_default());
        self.with_page(|page| page.show(Content::Blank));
    }

    pub(crate) fn reload(&mut self) {
        if let Some((index, url)) = self
            .current
            .and_then(|index| Some((index, self.entries.get(index)?.clone())))
        {
            self.load(url, Target::Entry(index));
        }
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.current.is_some_and(|current| current > 0)
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.current
            .is_some_and(|current| current + 1 < self.entries.len())
    }

    pub(crate) fn go_back(&mut self) {
        if let Some(index) = self.current.and_then(|current| current.checked_sub(1)) {
            self.go_to(index);
        }
    }

    pub(crate) fn go_forward(&mut self) {
        if self.can_go_forward()
            && let Some(current) = self.current
        {
            self.go_to(current + 1);
        }
    }

    fn go_to(&mut self, index: usize) {
        if let Some(url) = self.entries.get(index).cloned() {
            self.load(url, Target::Entry(index));
        }
    }

    fn current_url(&self) -> Option<&Url> {
        self.entries.get(self.current?)
    }

    pub(crate) fn focus_address(&mut self) {
        self.ui.set_focus(self.toolbar.address);
        self.ui
            .update_widget(self.toolbar.address, |field: &mut TextField, _| {
                let length = field.text().len();
                field.select(0..length);
            });
    }

    fn focus_page(&mut self) {
        self.ui.set_focus(self.page);
    }

    fn address_focused(&self) -> bool {
        self.ui.focus() == Some(self.toolbar.address)
    }

    fn set_address(&mut self, text: &str) {
        self.ui
            .update_widget(self.toolbar.address, |field: &mut TextField, _| {
                field.replace_text(text);
            });
    }

    fn with_page<T>(&mut self, update: impl FnOnce(&mut PageView) -> T) -> Option<T> {
        self.ui.widget_as::<PageView>(self.page).map(update)
    }

    fn refresh_navigation(&mut self) {
        let enabled = [
            (self.toolbar.back, self.can_go_back()),
            (self.toolbar.forward, self.can_go_forward()),
            (self.toolbar.reload, self.current.is_some()),
        ];
        for (id, on) in enabled {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(on);
            }
        }
    }

    fn submit_address(&mut self) -> bool {
        let Some(field) = self.ui.widget_as::<TextField>(self.toolbar.address) else {
            return false;
        };
        match address::parse(&field.text()) {
            Some(url) => {
                self.navigate(url);
                true
            }
            None => false,
        }
    }

    fn settle(&mut self) {
        self.drain();
        if let Some(url) = self.with_page(PageView::take_followed).flatten() {
            self.follow(url);
        }
    }

    fn drain(&mut self) {
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() {
                return;
            }
            for signal in signals {
                match signal {
                    Signal::Back => self.go_back(),
                    Signal::Forward => self.go_forward(),
                    Signal::Reload => self.reload(),
                }
            }
        }
    }

    fn sheet(&self, width: f32, height: f32) -> Rect {
        let inset = if self.full_screen {
            0.0
        } else {
            SHEET_INSET as f32
        };
        Rect::new(
            inset,
            TOOLBAR_HEIGHT,
            (width - 2.0 * inset).max(0.0),
            (height - TOOLBAR_HEIGHT - inset).max(0.0),
        )
    }

    fn frames(&self, width: f32) -> [(WidgetId, Rect); 4] {
        let middle = |height: f32| ((TOOLBAR_HEIGHT - height) / 2.0).round();
        let back = Rect::new(
            window_controls_end(true) + SIDE,
            middle(SYMBOL_BUTTON),
            SYMBOL_BUTTON,
            SYMBOL_BUTTON,
        );
        let forward = Rect::new(
            back.x + SYMBOL_BUTTON + GAP,
            back.y,
            SYMBOL_BUTTON,
            SYMBOL_BUTTON,
        );
        let leading = forward.x + SYMBOL_BUTTON + SIDE;
        let trailing = width - SIDE - SYMBOL_BUTTON - SIDE;
        let room = (trailing - leading).max(0.0);
        let address_width = (width * 0.5).clamp(ADDRESS_MIN, ADDRESS_MAX).min(room);
        let centred = (width - address_width) / 2.0;
        let address_x = centred.clamp(leading, (trailing - address_width).max(leading));
        let address = Rect::new(
            address_x.round(),
            middle(TEXT_FIELD_HEIGHT),
            address_width.round(),
            TEXT_FIELD_HEIGHT,
        );
        let reload = Rect::new(
            address.x + address.width + GAP,
            middle(BUTTON_HEIGHT.max(SYMBOL_BUTTON)),
            SYMBOL_BUTTON,
            SYMBOL_BUTTON,
        );
        [
            (self.toolbar.back, back),
            (self.toolbar.forward, forward),
            (self.toolbar.address, address),
            (self.toolbar.reload, reload),
        ]
    }

    fn paint(&mut self, width: u32, height: u32) -> Scene {
        let (w, h) = (width as f32, height as f32);
        let sheet = self.sheet(w, h);
        let radius = if self.full_screen {
            0.0
        } else {
            Radius::concentric(Radius::XL, SHEET_INSET as f32)
        };
        let frames = self.frames(w);
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
            host.set_frame(self.page, sheet);
        }
        self.with_page(|page| page.set_radius(radius));
        let palette = self.ui.context().palette;
        let (controls, dialog) = self.ui.paint_layers(width, height);
        let mut items = vec![Item::Fill {
            shape: Shape::rounded(sheet, radius),
            paint: palette.sheet.into(),
        }];
        items.extend(controls);
        items.extend(dialog);
        Scene {
            width,
            height,
            background: palette.chassis,
            items,
        }
    }

    fn header(&self, width: f32) -> Header {
        Header {
            title_x: width,
            items: self.frames(width).iter().map(|(_, frame)| *frame).collect(),
            tabs: Vec::new(),
        }
    }

    fn handle_unhandled(&mut self, event: &WindowEvent) -> bool {
        let WindowEvent::Input(input) = event else {
            return false;
        };
        match input {
            Event::Key {
                key: Key::Enter, ..
            } if self.address_focused() => self.submit_address(),
            Event::Key {
                key: Key::Escape, ..
            } if self.address_focused() => {
                let shown = self.current_url().map(|url| url.to_string());
                self.set_address(shown.as_deref().unwrap_or_default());
                self.focus_page();
                true
            }
            _ => false,
        }
    }

    fn pointer_history(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::Input(Event::PointerUp(pointer)) => match pointer.button {
                PointerButton::Back => {
                    self.go_back();
                    true
                }
                PointerButton::Forward => {
                    self.go_forward();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }
}

impl View for BrowserView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.full_screen = window.header_height() == 0;
        window.set_frame_regions(FrameRegions {
            sidebar: 0.0,
            toolbar: if self.full_screen {
                TOOLBAR_HEIGHT
            } else {
                0.0
            },
        });
        if let Some(title) = self.title.take() {
            window.set_title(&title);
        }
        let (width, height) = window.size();
        window.set_header(self.header(width as f32));
        self.paint(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let history = self.pointer_history(&event);
        let handled = match window.drive(&mut self.ui, event) {
            Ok(Some(unhandled)) => self.handle_unhandled(&unhandled),
            Ok(None) => true,
            Err(error) => {
                tracing::warn!("{error}");
                false
            }
        };
        self.settle();
        history || handled || self.request.is_some()
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: std::time::Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
