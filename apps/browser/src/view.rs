use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_app::{Detached, TabDrop, TabDropTarget, TabHost, View};
use bande_toolkit::ui::{
    Alert, BUTTON_HEIGHT, Bounded, Button, Cell as TableCell, Column, Context, Event, FreeLayout,
    Key, Modifiers, Pointer, PointerButton, PopUpButton, RadioGroup, Row, TEXT_FIELD_HEIGHT,
    TabAction, TabLabel, TabStrip, TabStripLayout, Table, TabsShown, TextField, Ui, WidgetId,
    frame_time,
};
use bande_toolkit::{
    Appearance, Elevation, FrameRegions, Header, Image, Item, Point, Radius, Rect, SHEET_INSET,
    Scene, Shape, TextStyle, TextSystem, Transfer, WINDOW_HEADER_HEIGHT, Window, WindowEvent,
    window_controls_end,
};
use web_host::Url;
use web_host::{Clear, SiteData, Viewport};

use crate::page::{Content, HangChoice, PageView};
use crate::pages;
use crate::places::{Place, Places};
use crate::renderer::{Command, Link, LoadId, Outcome, Session, Update, Verdict, ViewId};
use crate::settings::{BrowserSettings, SearchEngine};
use crate::{L10N, address};

const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const STRIP_HEIGHT: f32 = 36.0;
const SIDE: f32 = 12.0;
const GAP: f32 = 4.0;
const SYMBOL_BUTTON: f32 = 28.0;
const SYMBOL_PIXELS: u32 = 16;
const ADDRESS_MIN: f32 = 240.0;
const ADDRESS_MAX: f32 = 680.0;
const CLOSED_KEPT: usize = 32;
const SUGGESTIONS: usize = 6;
const SUGGESTION_ROW: f32 = 40.0;
const SUGGESTION_INSET: f32 = 6.0;
const HISTORY_SHOWN: usize = 500;
const ZOOMS: [f32; 9] = [0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0];
const DEFAULT_ZOOM: usize = 4;
const SITES_HEIGHT: f32 = 240.0;
const SITE_COLUMN: f32 = 200.0;
const DATA_COLUMN: f32 = 140.0;
const HOUR: u64 = 3600;
const FIRST_VIEWPORT: Viewport = Viewport {
    width: 1088.0,
    height: 706.0,
};

const BACK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M10 3.5 5.5 8l4.5 4.5" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;
const FORWARD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="m6 3.5 4.5 4.5L6 12.5" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;
const STOP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="m4.5 4.5 7 7m0-7-7 7" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>"##;
const RELOAD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M12.6 6.2A5 5 0 1 0 13 9" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" fill="none"/><path d="M13 2.8v3.6H9.4" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;

enum Signal {
    Back,
    Forward,
    Reload,
    Stop,
    Typed(String),
    ClearHistory,
    RemoveSites,
    AskRemoveAll,
    RemoveAll,
    ShowSites,
    EngineChosen,
    EngineDeclined,
}

#[derive(Clone, Copy)]
enum Range {
    LastHour,
    Today,
    TodayAndYesterday,
    All,
}

impl Range {
    const ALL: [Self; 4] = [
        Self::LastHour,
        Self::Today,
        Self::TodayAndYesterday,
        Self::All,
    ];

    fn label(self) -> String {
        L10N.tr(match self {
            Self::LastHour => "clear-last-hour",
            Self::Today => "clear-today",
            Self::TodayAndYesterday => "clear-today-yesterday",
            Self::All => "clear-all-history",
        })
    }

    fn since(self) -> u64 {
        let midnight = |days: i64| {
            jiff::Zoned::now()
                .start_of_day()
                .and_then(|today| today.checked_sub(jiff::Span::new().days(days)))
                .map(|start| u64::try_from(start.timestamp().as_second()).unwrap_or(0))
                .unwrap_or(0)
        };
        match self {
            Self::LastHour => crate::places::now().saturating_sub(HOUR),
            Self::Today => midnight(0),
            Self::TodayAndYesterday => midnight(1),
            Self::All => 0,
        }
    }
}

fn site_detail(site: &SiteData) -> String {
    let mut parts = Vec::new();
    if site.cookies > 0 {
        parts.push(L10N.tr_args("site-cookies", [("count", site.cookies.into())]));
    }
    if site.cache_bytes > 0 {
        parts.push(L10N.tr_args(
            "site-cache",
            [("size", bande_fs::format_size(site.cache_bytes).into())],
        ));
    }
    parts.join(", ")
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

fn reload_button(signals: &Signals, loading: bool) -> Button {
    match loading {
        true => symbol(STOP, &L10N.tr("stop"), signals, || Signal::Stop),
        false => symbol(RELOAD, &L10N.tr("reload"), signals, || Signal::Reload),
    }
}

fn address_text(url: &Url) -> &str {
    match url.scheme() {
        "http" | "https" => url.as_str(),
        _ => "",
    }
}

fn without_fragment(url: &Url) -> Url {
    let mut url = url.clone();
    url.set_fragment(None);
    url
}

#[derive(Clone, Copy)]
enum Target {
    New,
    Entry(usize),
}

struct Entry {
    url: Url,
    scroll: f32,
}

struct Pending {
    load: LoadId,
    target: Target,
    fallback: Option<Url>,
}

struct Tab {
    id: ViewId,
    session: Session,
    page: WidgetId,
    next_load: u64,
    shown: Option<LoadId>,
    pending: Option<Pending>,
    entries: Vec<Entry>,
    current: Option<usize>,
    sent_viewport: Option<Viewport>,
    title: Option<String>,
    deferred: Option<Url>,
    zoom: f32,
}

impl Tab {
    fn new(id: ViewId, session: Session, page: WidgetId) -> Self {
        Self {
            id,
            session,
            page,
            next_load: 0,
            shown: None,
            pending: None,
            entries: Vec::new(),
            current: None,
            sent_viewport: None,
            title: None,
            deferred: None,
            zoom: 1.0,
        }
    }

    fn current_url(&self) -> Option<&Url> {
        self.current
            .and_then(|current| self.entries.get(current))
            .map(|entry| &entry.url)
            .or(self.deferred.as_ref())
    }

    fn shown_url(&self) -> Option<&Url> {
        self.current_url()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
    }

    fn label(&self) -> String {
        self.title
            .clone()
            .or_else(|| {
                self.current_url()
                    .and_then(|url| url.host_str().map(str::to_owned))
            })
            .unwrap_or_else(|| L10N.tr("new-tab-title"))
    }
}

pub(crate) struct CarriedTab {
    tab: Tab,
    page: Box<PageView>,
}

impl CarriedTab {
    pub(crate) fn session(&self) -> Session {
        self.tab.session
    }
}

pub(crate) enum Ask {
    Detach(Box<Detached<CarriedTab>>),
    TabDrop(TabDrop),
}

pub(crate) struct SavedWindow {
    pub(crate) tabs: Vec<Url>,
    pub(crate) active: usize,
}

struct Toolbar {
    back: WidgetId,
    forward: WidgetId,
    address: WidgetId,
    reload: WidgetId,
    stop_shown: bool,
}

pub(crate) struct BrowserView {
    ui: Ui,
    signals: Signals,
    toolbar: Toolbar,
    host: WidgetId,
    session: Session,
    renderer: Link,
    ids: Rc<Cell<u64>>,
    tabs: Vec<Tab>,
    active: usize,
    strip: TabStrip,
    strip_pressed: bool,
    drop_target: TabDropTarget,
    asks: Vec<Ask>,
    closed: Vec<Url>,
    viewport: Option<Viewport>,
    width: f32,
    window_title: Option<String>,
    full_screen: bool,
    focused: bool,
    changed: bool,
    places: Rc<Places>,
    page_size: Option<(f32, f32)>,
    suggestions: Vec<Place>,
    suggested: Option<usize>,
    settings: Rc<Cell<BrowserSettings>>,
    insecure_allowed: HashSet<String>,
    site_data_wanted: bool,
    clear_range: Rc<Cell<usize>>,
    chosen_sites: Rc<RefCell<Vec<String>>>,
    engines: Vec<SearchEngine>,
    engine_pick: Rc<Cell<Option<usize>>>,
    waiting_query: Option<String>,
}

impl BrowserView {
    pub(crate) fn new(
        appearance: Appearance,
        session: Session,
        renderer: Link,
        ids: Rc<Cell<u64>>,
        places: Rc<Places>,
        settings: Rc<Cell<BrowserSettings>>,
    ) -> Self {
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
        let placeholder = match session {
            Session::Default => L10N.tr("address-placeholder"),
            Session::Private(_) => L10N.tr("private-placeholder"),
        };
        let typed = Rc::clone(&signals);
        let address = host.insert_in_header(
            TextField::new(&placeholder, "", move |text| {
                typed.borrow_mut().push(Signal::Typed(text.to_owned()));
            }),
            Rect::default(),
        );
        let reload = host.insert_in_header(reload_button(&signals, false), Rect::default());
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
                stop_shown: false,
            },
            host: host_id,
            session,
            renderer,
            ids,
            tabs: Vec::new(),
            active: 0,
            strip: TabStrip::new(),
            strip_pressed: false,
            drop_target: TabDropTarget::default(),
            asks: Vec::new(),
            closed: Vec::new(),
            viewport: None,
            width: 0.0,
            window_title: None,
            full_screen: false,
            focused: true,
            changed: false,
            places,
            page_size: None,
            suggestions: Vec::new(),
            suggested: None,
            settings,
            insecure_allowed: HashSet::new(),
            site_data_wanted: false,
            clear_range: Rc::new(Cell::new(0)),
            chosen_sites: Rc::default(),
            engines: Vec::new(),
            engine_pick: Rc::new(Cell::new(None)),
            waiting_query: None,
        };
        view.refresh_navigation();
        view
    }

    pub(crate) fn is_private(&self) -> bool {
        matches!(self.session, Session::Private(_))
    }

    pub(crate) fn has_tab(&self, id: ViewId) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub(crate) fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub(crate) fn saved(&self) -> Option<SavedWindow> {
        let tabs: Vec<Url> = self
            .tabs
            .iter()
            .filter(|tab| tab.session == Session::Default)
            .filter_map(|tab| tab.current_url().cloned())
            .collect();
        (!tabs.is_empty()).then_some(SavedWindow {
            active: self.active.min(tabs.len() - 1),
            tabs,
        })
    }

    fn next_id(&self) -> ViewId {
        let id = self.ids.get() + 1;
        self.ids.set(id);
        ViewId(id)
    }

    fn host(&mut self) -> Option<&mut FreeLayout> {
        self.ui.widget_as::<FreeLayout>(self.host)
    }

    pub(crate) fn open_tab(&mut self, url: Option<Url>, select: bool) {
        let page = PageView::new();
        let page_id = bande_toolkit::ui::Widget::id(&page);
        let Some(host) = self.host() else {
            return;
        };
        host.insert(page, Rect::default());
        let index = self.tabs.len();
        let id = self.next_id();
        self.tabs.push(Tab::new(id, self.session, page_id));
        let shown = select || index == 0;
        match url {
            Some(url) if shown => {
                self.select(index);
                self.start(index, url, Target::New);
            }
            Some(url) => self.tabs[index].deferred = Some(url),
            None if shown => {
                self.select(index);
                self.focus_address();
            }
            None => {}
        }
        self.changed = true;
    }

    pub(crate) fn new_tab(&mut self) {
        let palette = self.ui.context().palette;
        let start = pages::favourites(&self.places.bookmarks(), &palette);
        self.open_tab(start, true);
        self.set_address("");
        self.focus_address();
    }

    pub(crate) fn open_page(&mut self, url: Option<Url>) {
        if url.is_some() {
            self.open_tab(url, true);
        }
    }

    pub(crate) fn show_history(&mut self) {
        let palette = self.ui.context().palette;
        let history = self.places.history(HISTORY_SHOWN);
        let page = pages::listing(
            &L10N.tr("history-title"),
            &history,
            &palette,
            &L10N.tr("history-empty"),
        );
        self.open_page(page);
    }

    pub(crate) fn show_bookmarks(&mut self) {
        let palette = self.ui.context().palette;
        let bookmarks = self.places.bookmarks();
        let page = pages::listing(
            &L10N.tr("bookmarks-title"),
            &bookmarks,
            &palette,
            &L10N.tr("bookmarks-empty"),
        );
        self.open_page(page);
    }

    pub(crate) fn current_page(&self) -> Option<Url> {
        self.tabs.get(self.active)?.shown_url().cloned()
    }

    pub(crate) fn toggle_bookmark(&mut self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let Some(url) = tab.shown_url().cloned() else {
            return false;
        };
        let title = tab.title.clone().unwrap_or_default();
        match self.places.toggle_bookmark(&url, &title) {
            Ok(added) => added,
            Err(error) => {
                tracing::warn!("cannot change the bookmark: {error}");
                false
            }
        }
    }

    pub(crate) fn is_bookmarked(&self) -> bool {
        self.current_page()
            .is_some_and(|url| self.places.is_bookmarked(&url))
    }

    pub(crate) fn zoom(&mut self, step: Option<i32>) {
        let index = self.active;
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let current = ZOOMS
            .iter()
            .position(|zoom| (*zoom - tab.zoom).abs() < 0.01)
            .unwrap_or(DEFAULT_ZOOM);
        let next = match step {
            Some(step) => current
                .saturating_add_signed(step as isize)
                .min(ZOOMS.len() - 1),
            None => DEFAULT_ZOOM,
        };
        tab.zoom = ZOOMS[next];
        let zoom = tab.zoom;
        self.with_page(index, |page| page.set_zoom(zoom));
    }

    fn suggest(&mut self, text: &str) {
        self.suggested = None;
        self.suggestions = match self.address_focused() {
            true => self.places.suggest(text, SUGGESTIONS),
            false => Vec::new(),
        };
    }

    fn suggestion_frame(&self) -> Option<Rect> {
        if self.suggestions.is_empty() {
            return None;
        }
        let address = self.frames(self.width)[2].1;
        let height = SUGGESTION_ROW * self.suggestions.len() as f32 + 2.0 * SUGGESTION_INSET;
        Some(Rect::new(
            address.x,
            address.y + address.height + 4.0,
            address.width,
            height,
        ))
    }

    fn suggestion_at(&self, point: Point) -> Option<usize> {
        let frame = self.suggestion_frame()?;
        if !frame.contains(point) {
            return None;
        }
        let row = ((point.y - frame.y - SUGGESTION_INSET) / SUGGESTION_ROW).floor();
        (row >= 0.0)
            .then_some(row as usize)
            .filter(|row| *row < self.suggestions.len())
    }

    fn open_suggestion(&mut self, index: usize) -> bool {
        let Some(place) = self.suggestions.get(index) else {
            return false;
        };
        let url = place.url.clone();
        self.suggestions.clear();
        self.suggested = None;
        self.navigate(url);
        true
    }

    fn paint_suggestions(&mut self, items: &mut Vec<Item>) {
        let Some(frame) = self.suggestion_frame() else {
            return;
        };
        let palette = self.ui.context().palette;
        items.extend(Item::shadows(
            Shape::rounded(frame, Radius::M),
            Elevation::Overlay,
            palette.scheme,
            true,
        ));
        items.push(Item::Fill {
            shape: Shape::rounded(frame, Radius::M),
            paint: palette.raised.into(),
        });
        let width = (frame.width - 2.0 * SUGGESTION_INSET - 16.0).max(0.0);
        for (index, place) in self.suggestions.iter().enumerate() {
            let row = Rect::new(
                frame.x + SUGGESTION_INSET,
                frame.y + SUGGESTION_INSET + index as f32 * SUGGESTION_ROW,
                frame.width - 2.0 * SUGGESTION_INSET,
                SUGGESTION_ROW,
            );
            if self.suggested == Some(index) {
                items.push(Item::Fill {
                    shape: Shape::rounded(row, Radius::S),
                    paint: palette.accent_soft.into(),
                });
            }
            let title = match place.title.is_empty() {
                true => place.url.host_str().unwrap_or_default().to_owned(),
                false => place.title.clone(),
            };
            let text = &mut self.ui.context().text;
            let heading = text.fitted(&title, TextStyle::new(13.0, palette.ink), width);
            let detail = text.fitted(
                place.url.as_str(),
                TextStyle::new(11.0, palette.ink2),
                width,
            );
            items.push(Item::Text {
                origin: Point::new(row.x + 8.0, row.y + 4.0),
                paragraph: heading,
            });
            items.push(Item::Text {
                origin: Point::new(row.x + 8.0, row.y + 21.0),
                paragraph: detail,
            });
        }
    }

    fn suggestion_input(&mut self, window: &mut Window, input: &Event) -> Option<bool> {
        let frame = self.suggestion_frame()?;
        match input {
            Event::PointerMove(pointer) | Event::PointerEnter(pointer)
                if frame.contains(pointer.position) =>
            {
                let hovered = self.suggestion_at(pointer.position);
                if hovered != self.suggested {
                    self.suggested = hovered;
                    window.request_redraw();
                }
                Some(true)
            }
            Event::PointerDown(pointer) if frame.contains(pointer.position) => {
                if let Some(index) = self.suggestion_at(pointer.position) {
                    self.open_suggestion(index);
                }
                Some(true)
            }
            Event::PointerDown(_) => {
                self.suggestions.clear();
                None
            }
            _ => None,
        }
    }

    pub(crate) fn select(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.save_scroll(self.active);
        self.active = index;
        let url = self.tabs[index].shown_url().map(Url::to_string);
        self.set_address(url.as_deref().unwrap_or_default());
        self.window_title = Some(self.tabs[index].label());
        if let Some(url) = self.tabs[index].deferred.take() {
            self.start(index, url, Target::New);
        }
        self.focus_page();
        self.refresh_navigation();
        self.changed = true;
    }

    pub(crate) fn select_number(&mut self, number: usize) {
        let index = match number {
            9 => self.tabs.len().saturating_sub(1),
            number => number.saturating_sub(1),
        };
        self.select(index);
    }

    pub(crate) fn step_tab(&mut self, forward: bool) {
        let count = self.tabs.len();
        if count > 1 {
            let next = match forward {
                true => (self.active + 1) % count,
                false => (self.active + count - 1) % count,
            };
            self.select(next);
        }
    }

    pub(crate) fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() < 2 {
            return false;
        }
        self.close_tab(self.active);
        true
    }

    fn close_tab(&mut self, index: usize) {
        let Some(carried) = self.take_tab(index) else {
            return;
        };
        if let Some(url) = carried.tab.current_url().cloned() {
            self.closed.push(url);
            if self.closed.len() > CLOSED_KEPT {
                self.closed.remove(0);
            }
        }
        self.renderer.send(Command::Close {
            view: carried.tab.id,
        });
    }

    pub(crate) fn reopen_closed_tab(&mut self) {
        if let Some(url) = self.closed.pop() {
            self.open_tab(Some(url), true);
        }
    }

    pub(crate) fn can_reopen_closed_tab(&self) -> bool {
        !self.closed.is_empty()
    }

    pub(crate) fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    fn take_tab(&mut self, index: usize) -> Option<CarriedTab> {
        if index >= self.tabs.len() {
            return None;
        }
        self.save_scroll(index);
        let tab = self.tabs.remove(index);
        let widget = self.host().and_then(|host| host.remove(tab.page))?;
        let any: Box<dyn std::any::Any> = widget;
        let page = any.downcast::<PageView>().ok()?;
        if !self.tabs.is_empty() {
            let next = match index < self.active {
                true => self.active - 1,
                false => self.active.min(self.tabs.len() - 1),
            };
            self.active = self.tabs.len();
            self.select(next);
        }
        self.changed = true;
        Some(CarriedTab { tab, page })
    }

    pub(crate) fn close_all(&self) {
        for tab in &self.tabs {
            self.renderer.send(Command::Close { view: tab.id });
        }
    }

    pub(crate) fn navigate_typed(&mut self, typed: address::Typed) {
        self.navigate(typed.url);
        if let Some(pending) = self
            .tabs
            .get_mut(self.active)
            .and_then(|tab| tab.pending.as_mut())
        {
            pending.fallback = typed.fallback;
        }
    }

    pub(crate) fn navigate(&mut self, url: Url) {
        if self.tabs.is_empty() {
            return self.open_tab(Some(url), true);
        }
        self.start(self.active, url, Target::New);
    }

    fn follow(&mut self, url: Url) {
        let index = self.active;
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let same_document = tab.shown.is_some()
            && tab
                .current_url()
                .is_some_and(|current| without_fragment(current) == without_fragment(&url));
        if !same_document {
            return self.navigate(url);
        }
        self.save_scroll(index);
        let tab = &mut self.tabs[index];
        let next = tab.current.map_or(0, |current| current + 1);
        tab.entries.truncate(next);
        tab.entries.push(Entry {
            url: url.clone(),
            scroll: 0.0,
        });
        tab.current = Some(next);
        self.set_address(address_text(&url));
        self.scroll_to_fragment(index, &url);
        self.refresh_navigation();
        self.changed = true;
    }

    fn scroll_to_fragment(&mut self, index: usize, url: &Url) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        match (tab.shown, url.fragment().filter(|name| !name.is_empty())) {
            (Some(load), Some(name)) => self.renderer.send(Command::Fragment {
                view: tab.id,
                load,
                name: name.to_owned(),
            }),
            _ => {
                self.with_page(index, |page| page.jump_to(0.0));
            }
        }
    }

    fn start(&mut self, index: usize, url: Url, target: Target) {
        let (url, fallback) = self.upgraded(url);
        self.save_scroll(index);
        let zoom = self.tabs.get(index).map_or(1.0, |tab| tab.zoom);
        let viewport = self
            .with_page(index, |page| page.viewport())
            .flatten()
            .or(self.page_size.map(|(width, height)| Viewport {
                width: (width / zoom).round(),
                height: (height / zoom).round(),
            }))
            .unwrap_or(FIRST_VIEWPORT);
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.next_load += 1;
        let load = LoadId(tab.next_load);
        tab.sent_viewport = Some(viewport);
        tab.deferred = None;
        tab.pending = Some(Pending {
            load,
            target,
            fallback,
        });
        self.renderer.send(Command::Load {
            view: tab.id,
            load,
            url: url.clone(),
            viewport,
            session: tab.session,
        });
        if !self
            .with_page(index, |page| page.has_document())
            .unwrap_or(false)
        {
            self.with_page(index, |page| page.show(Content::Loading(url.clone())));
        }
        if index == self.active {
            self.set_address(address_text(&url));
            self.focus_page();
        }
        self.refresh_navigation();
    }

    fn upgraded(&self, url: Url) -> (Url, Option<Url>) {
        let allowed = url
            .host_str()
            .is_some_and(|host| self.insecure_allowed.contains(host));
        if url.scheme() != "http" || !self.settings.get().https_only || allowed {
            return (url, None);
        }
        let mut secure = url.clone();
        match secure.set_scheme("https") {
            Ok(()) => (secure, Some(url)),
            Err(()) => (url, None),
        }
    }

    fn search(&mut self, query: String) {
        match self.settings.get().search {
            Some(engine) => {
                if let Some(url) = engine.search(&query) {
                    self.navigate(url);
                }
            }
            None => self.ask_search_engine(Some(query)),
        }
    }

    pub(crate) fn ask_search_engine(&mut self, query: Option<String>) {
        self.waiting_query = query;
        self.engines = SearchEngine::shuffled();
        self.engine_pick.set(None);
        let names: Vec<&str> = self.engines.iter().map(|engine| engine.name()).collect();
        let pick = Rc::clone(&self.engine_pick);
        let choices = RadioGroup::new(&L10N.tr("engine-choices"), &names, None, move |index| {
            pick.set(Some(index))
        });
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(
                &L10N.tr("engine-title"),
                &L10N.tr("engine-message"),
                move |action| {
                    signals.borrow_mut().push(match action {
                        "choose" => Signal::EngineChosen,
                        _ => Signal::EngineDeclined,
                    });
                },
            )
            .accessory(choices)
            .primary(&L10N.tr("engine-choose"), "choose")
            .cancel(&L10N.tr("engine-later"), "later"),
        );
    }

    fn engine_chosen(&mut self) {
        let picked = self
            .engine_pick
            .get()
            .and_then(|index| self.engines.get(index).copied());
        let Some(engine) = picked else {
            let query = self.waiting_query.take();
            return self.ask_search_engine(query);
        };
        self.save_settings(BrowserSettings {
            search: Some(engine),
            search_asked: true,
            ..self.settings.get()
        });
        if let Some(query) = self.waiting_query.take() {
            self.search(query);
        }
    }

    fn engine_declined(&mut self) {
        self.waiting_query = None;
        self.save_settings(BrowserSettings {
            search_asked: true,
            ..self.settings.get()
        });
    }

    fn save_settings(&self, settings: BrowserSettings) {
        self.settings.set(settings);
        if let Some(folder) = bande_app_settings::settings_folder() {
            let saved = std::fs::create_dir_all(&folder)
                .and_then(|()| bande_app_settings::Stored::save(&settings, &folder));
            if let Err(error) = saved {
                tracing::warn!("cannot save the browser settings: {error}");
            }
        }
    }

    pub(crate) fn ask_clear_history(&mut self) {
        let ranges: Vec<String> = Range::ALL.iter().map(|range| range.label()).collect();
        let chosen = Rc::clone(&self.clear_range);
        let range = PopUpButton::new(
            &L10N.tr("clear-range"),
            &ranges,
            chosen.get().min(ranges.len() - 1),
            move |index| chosen.set(index),
        );
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(
                &L10N.tr("clear-history-title"),
                &L10N.tr("clear-history-message"),
                move |action| {
                    if action == "clear" {
                        signals.borrow_mut().push(Signal::ClearHistory);
                    }
                },
            )
            .accessory(range)
            .destructive(&L10N.tr("clear-history-button"), "clear")
            .cancel(&L10N.tr("cancel"), "cancel"),
        );
    }

    fn clear_history(&mut self) {
        let range = Range::ALL
            .get(self.clear_range.get())
            .copied()
            .unwrap_or(Range::LastHour);
        let since = range.since();
        if let Err(error) = self.places.clear_history(since) {
            tracing::warn!("cannot clear the history: {error}");
        }
        self.renderer.send(Command::ClearData {
            session: Session::Default,
            clear: Clear {
                since,
                cookies: true,
                cache: true,
                sites: None,
            },
        });
        self.closed.clear();
    }

    pub(crate) fn ask_website_data(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        self.site_data_wanted = true;
        self.renderer.send(Command::SiteData {
            view: tab.id,
            session: self.session,
        });
    }

    fn show_website_data(&mut self, sites: Vec<SiteData>) {
        if !std::mem::take(&mut self.site_data_wanted) {
            return;
        }
        self.chosen_sites.borrow_mut().clear();
        let mut table = Table::new(vec![
            Column::new(&L10N.tr("site-column"), SITE_COLUMN)
                .flexible()
                .unsortable(),
            Column::new(&L10N.tr("data-column"), DATA_COLUMN).unsortable(),
        ]);
        let empty = sites.is_empty();
        table.set_rows(
            sites
                .iter()
                .map(|site| {
                    Row::new(
                        site.site.clone(),
                        vec![
                            TableCell::new(site.site.as_str()),
                            TableCell::new(site_detail(site)).muted(),
                        ],
                    )
                })
                .collect(),
        );
        let chosen = Rc::clone(&self.chosen_sites);
        let table = table.on_selection(move |keys| *chosen.borrow_mut() = keys.to_vec());
        let signals = Rc::clone(&self.signals);
        let message = match empty {
            true => L10N.tr("website-data-empty"),
            false => L10N.tr("website-data-message"),
        };
        let mut alert = Alert::new(&L10N.tr("website-data-title"), &message, move |action| {
            let signal = match action {
                "remove" => Signal::RemoveSites,
                "remove-all" => Signal::AskRemoveAll,
                _ => return,
            };
            signals.borrow_mut().push(signal);
        });
        if !empty {
            alert = alert
                .accessory(Bounded::new(table, SITES_HEIGHT))
                .other(&L10N.tr("remove"), "remove")
                .other(&L10N.tr("remove-all"), "remove-all");
        }
        self.ui
            .present_sheet(alert.primary(&L10N.tr("done"), "done"));
    }

    pub(crate) fn remove_sites(&mut self, sites: Option<Vec<String>>) {
        if sites.as_ref().is_some_and(Vec::is_empty) {
            return self.ask_website_data();
        }
        self.renderer.send(Command::ClearData {
            session: self.session,
            clear: Clear {
                since: 0,
                cookies: true,
                cache: true,
                sites,
            },
        });
        self.ask_website_data();
    }

    fn ask_remove_all(&mut self) {
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(
                &L10N.tr("remove-all-title"),
                &L10N.tr("remove-all-message"),
                move |action| {
                    let signal = match action {
                        "remove" => Signal::RemoveAll,
                        _ => Signal::ShowSites,
                    };
                    signals.borrow_mut().push(signal);
                },
            )
            .destructive(&L10N.tr("remove-now"), "remove")
            .cancel(&L10N.tr("cancel"), "cancel"),
        );
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|tab| tab.pending.is_some())
    }

    pub(crate) fn update(&mut self, update: Update) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == update.view) else {
            return;
        };
        let tab = &self.tabs[index];
        let pending = tab.pending.as_ref().map(|pending| pending.load);
        let shown = tab.shown;
        match update.outcome {
            Outcome::Relaid(list, links, damage) if Some(update.load) == shown => {
                self.with_page(index, |page| page.relaid(list, links, damage));
            }
            Outcome::Scroll(y) if Some(update.load) == shown => {
                self.with_page(index, |page| page.jump_to(y));
            }
            Outcome::Crashed { url } if Some(update.load) == shown => {
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.shown = None;
                }
                self.with_page(index, |page| page.show(Content::Crashed(url)));
                self.refresh_navigation();
            }
            Outcome::Unresponsive if Some(update.load) == shown || Some(update.load) == pending => {
                self.with_page(index, |page| page.set_unresponsive(true));
            }
            Outcome::Responsive => {
                self.with_page(index, |page| page.set_unresponsive(false));
            }
            Outcome::SiteData(sites) => self.show_website_data(sites),
            Outcome::Paused(paused) => self.renderer.send(Command::Resume {
                request: paused.request,
                verdict: Verdict::Continue,
            }),
            outcome if Some(update.load) == pending => self.commit(index, update.load, outcome),
            _ => {}
        }
    }

    fn commit(&mut self, index: usize, load: LoadId, outcome: Outcome) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let Some(pending) = tab.pending.take() else {
            return;
        };
        let (url, title, content, scroll) = match outcome {
            Outcome::Frame(frame) => {
                tab.shown = Some(load);
                let content = Content::Document {
                    list: frame.list,
                    links: frame.links,
                };
                (frame.url, frame.title, content, frame.fragment)
            }
            Outcome::Failed { url, message } => {
                tab.shown = None;
                let content = match pending.fallback.clone() {
                    Some(http) => Content::Insecure {
                        https: url.clone(),
                        http,
                        message,
                    },
                    None => Content::Failed {
                        url: url.clone(),
                        message,
                    },
                };
                (url, None, content, None)
            }
            Outcome::Unsupported { url, kind } => {
                tab.shown = None;
                (url.clone(), None, Content::Unsupported { url, kind }, None)
            }
            Outcome::Crashed { url } => {
                tab.shown = None;
                (url.clone(), None, Content::Crashed(url), None)
            }
            Outcome::Relaid(..)
            | Outcome::Scroll(_)
            | Outcome::Paused(_)
            | Outcome::Unresponsive
            | Outcome::Responsive
            | Outcome::SiteData(_) => return,
        };
        let restored = match pending.target {
            Target::New => {
                let next = tab.current.map_or(0, |current| current + 1);
                tab.entries.truncate(next);
                tab.entries.push(Entry {
                    url: url.clone(),
                    scroll: 0.0,
                });
                tab.current = Some(next);
                None
            }
            Target::Entry(entry) => tab.entries.get_mut(entry).and_then(|saved| {
                saved.url = url.clone();
                tab.current = Some(entry);
                Some(saved.scroll).filter(|scroll| *scroll > 0.0)
            }),
        };
        tab.title = title;
        let label = tab.label();
        let record = tab.session == Session::Default && matches!(content, Content::Document { .. });
        if record
            && let Err(error) = self
                .places
                .visit(&url, tab.title.as_deref().unwrap_or_default())
        {
            tracing::warn!("cannot record the visit: {error}");
        }
        self.with_page(index, |page| {
            page.show(content);
            match (restored, scroll) {
                (Some(offset), _) => page.restore_offset(offset),
                (None, Some(y)) => page.jump_to(y),
                (None, None) => {}
            }
        });
        if index == self.active {
            if !self.address_focused() {
                self.set_address(address_text(&url));
            }
            self.window_title = Some(label);
        }
        self.refresh_navigation();
        self.changed = true;
    }

    pub(crate) fn stop(&mut self) {
        let index = self.active;
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if tab.pending.take().is_none() {
            return;
        }
        self.renderer.send(Command::Stop { view: tab.id });
        let shown = tab.shown.is_some();
        let url = tab.shown_url().map(Url::to_string);
        self.set_address(url.as_deref().unwrap_or_default());
        if !shown {
            self.with_page(index, |page| page.show(Content::Blank));
        }
        self.refresh_navigation();
    }

    pub(crate) fn reload(&mut self) {
        let index = self.active;
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if let Some((entry, url)) = tab
            .current
            .and_then(|entry| Some((entry, tab.entries.get(entry)?.url.clone())))
        {
            self.start(index, url, Target::Entry(entry));
        }
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.tabs
            .get(self.active)
            .and_then(|tab| tab.current)
            .is_some_and(|current| current > 0)
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|tab| {
            tab.current
                .is_some_and(|current| current + 1 < tab.entries.len())
        })
    }

    pub(crate) fn go_back(&mut self) {
        let current = self.tabs.get(self.active).and_then(|tab| tab.current);
        if let Some(entry) = current.and_then(|current| current.checked_sub(1)) {
            self.go_to(entry);
        }
    }

    pub(crate) fn go_forward(&mut self) {
        let current = self.tabs.get(self.active).and_then(|tab| tab.current);
        if self.can_go_forward()
            && let Some(current) = current
        {
            self.go_to(current + 1);
        }
    }

    fn go_to(&mut self, entry: usize) {
        let index = self.active;
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let Some(saved) = tab.entries.get(entry) else {
            return;
        };
        let (url, scroll) = (saved.url.clone(), saved.scroll);
        let same_document = tab.shown.is_some()
            && tab.pending.is_none()
            && tab
                .current_url()
                .is_some_and(|current| without_fragment(current) == without_fragment(&url));
        if !same_document {
            return self.start(index, url, Target::Entry(entry));
        }
        self.save_scroll(index);
        self.tabs[index].current = Some(entry);
        self.set_address(address_text(&url));
        self.with_page(index, |page| page.restore_offset(scroll));
        self.refresh_navigation();
        self.changed = true;
    }

    fn save_scroll(&mut self, index: usize) {
        if self.tabs.get(index).is_none_or(|tab| tab.shown.is_none()) {
            return;
        }
        let offset = self.with_page(index, |page| page.offset()).unwrap_or(0.0);
        if let Some(tab) = self.tabs.get_mut(index)
            && let Some(entry) = tab.current.and_then(|current| tab.entries.get_mut(current))
        {
            entry.scroll = offset;
        }
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
        if let Some(tab) = self.tabs.get(self.active) {
            self.ui.set_focus(tab.page);
        }
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

    fn with_page<T>(&mut self, index: usize, update: impl FnOnce(&mut PageView) -> T) -> Option<T> {
        let page = self.tabs.get(index)?.page;
        self.ui.widget_as::<PageView>(page).map(update)
    }

    fn refresh_navigation(&mut self) {
        let loading = self.is_loading();
        if loading != self.toolbar.stop_shown {
            let button = reload_button(&self.signals, loading);
            let old = self.toolbar.reload;
            if let Some(id) = self.host().and_then(|host| host.replace(old, button)) {
                self.toolbar.reload = id;
                self.toolbar.stop_shown = loading;
            }
        }
        let has_page = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.current.is_some());
        let enabled = [
            (self.toolbar.back, self.can_go_back()),
            (self.toolbar.forward, self.can_go_forward()),
            (self.toolbar.reload, loading || has_page),
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
        match address::typed(&field.text()) {
            Some(address::Input::Address(typed)) => {
                self.navigate_typed(typed);
                true
            }
            Some(address::Input::Search(query)) => {
                self.search(query);
                true
            }
            None => false,
        }
    }

    fn settle(&mut self) {
        self.drain();
        let index = self.active;
        if let Some(url) = self.with_page(index, PageView::take_followed).flatten() {
            self.follow(url);
        }
        if let Some(url) = self.with_page(index, PageView::take_continued).flatten() {
            if let Some(host) = url.host_str() {
                self.insecure_allowed.insert(host.to_owned());
            }
            self.navigate(url);
        }
        if self
            .with_page(index, PageView::take_reload)
            .unwrap_or(false)
        {
            self.reload();
        }
        let choice = self.with_page(index, PageView::take_hang_choice).flatten();
        if let (Some(choice), Some(tab)) = (choice, self.tabs.get(index)) {
            let view = tab.id;
            self.renderer.send(match choice {
                HangChoice::Wait => Command::Wait { view },
                HangChoice::Stop => Command::Kill { view },
            });
            self.with_page(index, |page| page.set_unresponsive(false));
        }
        let hovered = self.with_page(index, PageView::take_hovered).flatten();
        if let (Some((x, y)), Some(tab)) = (hovered, self.tabs.get(index))
            && let Some(load) = tab.shown
        {
            self.renderer.send(Command::Hover {
                view: tab.id,
                load,
                x,
                y,
            });
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
                    Signal::Stop => self.stop(),
                    Signal::Typed(text) => self.suggest(&text),
                    Signal::ClearHistory => self.clear_history(),
                    Signal::RemoveSites => {
                        let sites = self.chosen_sites.borrow().clone();
                        self.remove_sites(Some(sites));
                    }
                    Signal::AskRemoveAll => self.ask_remove_all(),
                    Signal::RemoveAll => self.remove_sites(None),
                    Signal::ShowSites => self.ask_website_data(),
                    Signal::EngineChosen => self.engine_chosen(),
                    Signal::EngineDeclined => self.engine_declined(),
                }
            }
        }
    }

    fn strip_shown(&self) -> bool {
        self.tabs.len() > 1
    }

    fn strip_layout(&self) -> TabStripLayout {
        TabStripLayout::new(self.tabs.len(), self.width, SIDE, STRIP_HEIGHT)
    }

    fn synced_strip(&mut self) -> TabStripLayout {
        let ids: Vec<u64> = self.tabs.iter().map(|tab| tab.id.0).collect();
        self.strip.sync(&ids);
        self.strip_layout()
    }

    fn content_top(&self) -> f32 {
        match self.strip_shown() {
            true => TOOLBAR_HEIGHT + STRIP_HEIGHT,
            false => TOOLBAR_HEIGHT,
        }
    }

    fn sheet(&self, width: f32, height: f32) -> Rect {
        let inset = if self.full_screen {
            0.0
        } else {
            SHEET_INSET as f32
        };
        let top = self.content_top();
        Rect::new(
            inset,
            top,
            (width - 2.0 * inset).max(0.0),
            (height - top - inset).max(0.0),
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
        self.width = w;
        let sheet = self.sheet(w, h);
        let radius = if self.full_screen {
            0.0
        } else {
            Radius::concentric(Radius::XL, SHEET_INSET as f32)
        };
        let frames = self.frames(w);
        let pages: Vec<(WidgetId, Rect)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| match index == self.active {
                true => (tab.page, sheet),
                false => (tab.page, Rect::default()),
            })
            .collect();
        if let Some(host) = self.host() {
            for (id, frame) in frames.into_iter().chain(pages) {
                host.set_frame(id, frame);
            }
        }
        let active = self.active;
        self.with_page(active, |page| page.set_radius(radius));
        let palette = self.ui.context().palette;
        let (controls, dialog) = self.ui.paint_layers(width, height);
        self.viewport = self.with_page(active, |page| page.viewport()).flatten();
        if let Some(size) = self.with_page(active, |page| page.frame_size()).flatten() {
            self.page_size = Some(size);
        }
        self.follow_viewport();
        let mut items = vec![Item::Fill {
            shape: Shape::rounded(sheet, radius),
            paint: palette.sheet.into(),
        }];
        if self.strip_shown() {
            let layout = self.synced_strip();
            let titles: Vec<String> = self.tabs.iter().map(Tab::label).collect();
            let labels: Vec<TabLabel<'_>> = titles
                .iter()
                .map(|title| TabLabel {
                    title,
                    badge: false,
                })
                .collect();
            let shown = TabsShown {
                labels: &labels,
                active: self.active,
                focused: self.focused,
                palette: &palette,
            };
            let strip = self.strip.paint(
                &layout,
                &shown,
                &mut self.ui.context().text,
                frame_time().presentation,
            );
            items.push(Item::Translate {
                offset: Point::new(0.0, TOOLBAR_HEIGHT),
                items: strip,
            });
        }
        items.extend(controls);
        self.paint_suggestions(&mut items);
        items.extend(dialog);
        Scene {
            width,
            height,
            background: palette.chassis,
            items,
        }
    }

    fn follow_viewport(&mut self) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.sent_viewport != Some(viewport) && tab.deferred.is_none() {
            tab.sent_viewport = Some(viewport);
            self.renderer.send(Command::Resize {
                view: tab.id,
                viewport,
            });
        }
    }

    fn header(&self, width: f32) -> Header {
        Header {
            title_x: width,
            items: self.frames(width).iter().map(|(_, frame)| *frame).collect(),
            tabs: Vec::new(),
        }
    }

    fn strip_input(&mut self, window: &mut Window, input: &Event) -> Option<bool> {
        if !self.strip_shown() {
            return None;
        }
        let local = |pointer: &Pointer| Pointer {
            position: Point::new(pointer.position.x, pointer.position.y - TOOLBAR_HEIGHT),
            ..*pointer
        };
        let layout = self.synced_strip();
        let now = frame_time().presentation;
        let reduce_motion = window.appearance().reduce_motion;
        match input {
            Event::PointerDown(pointer) => {
                let pointer = local(pointer);
                if !(0.0..STRIP_HEIGHT).contains(&pointer.position.y) {
                    return None;
                }
                if let Some(action) = self.strip.press(&layout, &pointer) {
                    self.strip_action(window, action);
                }
                self.strip_pressed = self.strip.is_engaged();
                Some(true)
            }
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                let pointer = local(pointer);
                let over = (0.0..STRIP_HEIGHT).contains(&pointer.position.y);
                if self.strip.hover(&layout, over.then_some(pointer.position)) {
                    window.request_redraw();
                }
                if !self.strip_pressed {
                    return None;
                }
                if let Some(action) =
                    self.strip
                        .moved(&layout, pointer.position, now, reduce_motion)
                {
                    self.strip_action(window, action);
                }
                self.strip_pressed = self.strip.is_engaged();
                Some(true)
            }
            Event::PointerUp(pointer) if self.strip_pressed => {
                let pointer = local(pointer);
                self.strip_pressed = false;
                if let Some(action) =
                    self.strip
                        .released(&layout, pointer.position, now, reduce_motion)
                {
                    self.strip_action(window, action);
                }
                Some(true)
            }
            Event::PointerLeave => {
                if self.strip.hover(&layout, None) {
                    window.request_redraw();
                }
                None
            }
            _ => None,
        }
    }

    fn strip_action(&mut self, window: &mut Window, action: TabAction) {
        match action {
            TabAction::Select(index) => self.select(index),
            TabAction::Close(index) => self.close_tab(index),
            TabAction::NewTab => self.new_tab(),
            TabAction::Menu { .. } => {}
            TabAction::Moved { from, to } => self.move_tab(from, to),
            TabAction::Detach { index, grab } => self.detach(window, index, grab),
        }
        window.request_redraw();
    }

    fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let active = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self
            .tabs
            .iter()
            .position(|tab| tab.id == active)
            .unwrap_or(to);
        self.changed = true;
    }

    fn detach(&mut self, window: &mut Window, index: usize, grab: Point) {
        self.strip_pressed = false;
        let Some(label) = self.tabs.get(index).map(Tab::label) else {
            return;
        };
        let drag = match window.start_window_drag(Transfer::text(&label)) {
            Ok(drag) => drag,
            Err(error) => {
                tracing::warn!("cannot drag the tab out: {error}");
                return;
            }
        };
        let size = window.size();
        let Some(tab) = self.take_tab(index) else {
            return;
        };
        let id = tab.tab.id.0;
        self.asks.push(Ask::Detach(Box::new(Detached {
            tab,
            id,
            index,
            drag,
            grab: Point::new(grab.x, grab.y + TOOLBAR_HEIGHT),
            size,
        })));
    }

    fn handle_unhandled(&mut self, event: &WindowEvent) -> bool {
        let WindowEvent::Input(input) = event else {
            return false;
        };
        match input {
            Event::Key {
                key: Key::Function(5),
                modifiers,
            } if *modifiers == Modifiers::default() => {
                self.reload();
                true
            }
            Event::Key { key: Key::Down, .. }
                if self.address_focused() && !self.suggestions.is_empty() =>
            {
                let last = self.suggestions.len() - 1;
                self.suggested = Some(self.suggested.map_or(0, |index| (index + 1).min(last)));
                true
            }
            Event::Key { key: Key::Up, .. }
                if self.address_focused() && !self.suggestions.is_empty() =>
            {
                self.suggested = self.suggested.and_then(|index| index.checked_sub(1));
                true
            }
            Event::Key {
                key: Key::Enter, ..
            } if self.address_focused() => match self.suggested {
                Some(index) => self.open_suggestion(index),
                None => self.submit_address(),
            },
            Event::Key {
                key: Key::Escape, ..
            } if self.address_focused() => {
                if !self.suggestions.is_empty() {
                    self.suggestions.clear();
                    self.suggested = None;
                    return true;
                }
                let url = self
                    .tabs
                    .get(self.active)
                    .and_then(Tab::shown_url)
                    .map(Url::to_string);
                self.set_address(url.as_deref().unwrap_or_default());
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
                self.content_top()
            } else {
                0.0
            },
        });
        if let Some(title) = self.window_title.take() {
            let title = match self.session {
                Session::Default => title,
                Session::Private(_) => format!("{title} — {}", L10N.tr("private-browsing")),
            };
            window.set_title(&title);
        }
        let (width, height) = window.size();
        window.set_header(self.header(width as f32));
        self.paint(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::Drag(drag) = event {
            let band = Some(TOOLBAR_HEIGHT + STRIP_HEIGHT);
            if let Some(drop) = self.drop_target.event(window, band, drag) {
                self.asks.push(Ask::TabDrop(drop));
            }
            return true;
        }
        if let WindowEvent::Input(input) = &event
            && let Some(handled) = self
                .suggestion_input(window, input)
                .or_else(|| self.strip_input(window, input))
        {
            self.settle();
            return handled;
        }
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
        history || handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.focused = active;
        self.ui.set_window_active(active);
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: std::time::Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating() || self.strip.animating(frame_time().presentation)
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}

impl TabHost for BrowserView {
    type Tab = CarriedTab;

    fn release_tab(&mut self, _window: &mut Window, id: u64) -> Option<CarriedTab> {
        let index = self.tabs.iter().position(|tab| tab.id.0 == id)?;
        self.take_tab(index)
    }

    fn adopt_tab(&mut self, window: &mut Window, carried: CarriedTab, index: usize) {
        let CarriedTab { tab, page } = carried;
        let Some(host) = self.host() else {
            return;
        };
        host.insert(*page, Rect::default());
        let index = index.min(self.tabs.len());
        self.tabs.insert(index, tab);
        self.active = self.tabs.len();
        self.select(index);
        window.request_redraw();
    }

    fn slide_tab_to(&mut self, window: &mut Window, id: u64, x: f32) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id.0 == id) else {
            return;
        };
        let layout = self.synced_strip();
        let reduce_motion = window.appearance().reduce_motion;
        if let Some(action) =
            self.strip
                .slide_to(&layout, index, x, frame_time().presentation, reduce_motion)
        {
            self.strip_action(window, action);
        }
    }

    fn insertion_index(&self, x: f32) -> usize {
        match self.strip_shown() {
            true => self.strip_layout().insertion_index(x),
            false => self.tabs.len(),
        }
    }

    fn grab_point(&self, grab: Point) -> Point {
        self.strip_layout().grab_point(grab)
    }
}
