pub mod geometry;
mod input;
mod inspector;
mod paint;
pub mod painter;
mod panel;
mod setup;
mod sidebar;
mod toolbar;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use accesskit::{ActionRequest, TreeUpdate};
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};
use unbound_datetime::DateFormats;
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{
    Alert, ClipboardRequest, Context, Cursor, FreeLayout, Menu, Ui, WidgetId,
};
use unbound_toolkit::{
    Appearance, Header, Item, Palette, Point, Radius, Rect, SHEET_INSET, Scene, Shape, TextSystem,
};

use crate::edit::{Details, Scope};
use crate::ical::{CalendarObject, InstanceKey};
use crate::store::{Found, LOCAL_ACCOUNT, Library};

pub use toolbar::TOOLBAR_HEIGHT;

const ENGLISH: &str = include_str!("../../locales/en-US/bande-calendar.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const SIDEBAR_WIDTH: f32 = 220.0;
const DAY_START_HOUR: f32 = 7.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Day,
    Week,
    Month,
    Year,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Year];

    fn title(self) -> String {
        L10N.tr(match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Menu(Menu, Point),
    Clipboard(ClipboardRequest),
    AddAccount {
        server: String,
        user: String,
        password: String,
    },
    Subscribe(String),
    RemoveAccount(String),
    Changed,
    Refresh,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Signal {
    Today,
    Previous,
    Next,
    Mode(usize),
    Search(String),
    NewEvent,
    AddAccount,
    Title(String),
    Location(String),
    AllDay(bool),
    StartDate(String),
    StartTime(String),
    EndDate(String),
    EndTime(String),
    Repeat(usize),
    Alert(usize),
    Calendar(usize),
    Notes(String),
    DeleteEvent,
    Choice(String),
    SetupKind(usize),
    SetupUser(String),
    SetupPassword(String),
    SetupServer(String),
    SetupSubmit,
    SetupCancel,
}

pub(crate) type Signals = Rc<RefCell<Vec<Signal>>>;

pub(crate) fn sender(signals: &Signals) -> impl Fn(Signal) + 'static {
    let signals = signals.clone();
    move |signal| signals.borrow_mut().push(signal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Body,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Hit {
    Calendar(usize),
    Account(String),
    MiniDay(Date),
    MiniShift(i32),
    Event(usize, Part),
    AllDay(Date),
    MonthDay(Date),
    YearDay(Date),
    YearMonth(Date),
    Result(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Drag {
    Create {
        anchor: DateTime,
        current: DateTime,
        all_day: bool,
    },
    Move {
        found: Found,
        grab: DateTime,
        current: DateTime,
        all_day: bool,
        moved: bool,
    },
    Resize {
        found: Found,
        end: DateTime,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Confirm {
    Reschedule {
        found: Found,
        start: DateTime,
        end: DateTime,
        all_day: bool,
    },
    Details {
        found: Found,
        details: Details,
        calendar: Option<usize>,
    },
    Delete(Found),
    RemoveAccount(String),
    Import(Vec<CalendarObject>, usize),
    Subscribe(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    calendar: String,
    uid: String,
    key: InstanceKey,
}

pub struct CalendarView {
    library: Library,
    ui: Ui,
    host: WidgetId,
    signals: Signals,
    toolbar: toolbar::Toolbar,
    palette: Palette,
    formats: DateFormats,
    zone: TimeZone,
    now: Zoned,
    mode: Mode,
    focus: Date,
    mini_month: Date,
    scroll: f32,
    size: (f32, f32),
    full_screen: bool,
    shown: Vec<Found>,
    hits: Vec<(Rect, Hit)>,
    drag: Option<Drag>,
    press: Option<Point>,
    hover: Option<Hit>,
    selected: Option<Selection>,
    inspector: Option<inspector::Inspector>,
    setup: Option<setup::Setup>,
    confirm: Option<Confirm>,
    search: String,
    results: Vec<Found>,
    status: Option<String>,
    requests: Vec<Request>,
    scrolled_once: bool,
    grid: Option<geometry::TimeGrid>,
    pending_open: Option<Selection>,
}

impl CalendarView {
    pub fn new(
        library: Library,
        palette: Palette,
        text: TextSystem,
        formats: DateFormats,
        zone: TimeZone,
        now: Zoned,
    ) -> Self {
        let signals: Signals = Rc::new(RefCell::new(Vec::new()));
        let mut host = FreeLayout::new();
        let toolbar = toolbar::Toolbar::new(&mut host, &signals, Mode::Week);
        let host_id = unbound_toolkit::ui::Widget::id(&host);
        let today = now.date();
        let mut cx = Context::new(text, palette);
        cx.keyboard_navigation = true;
        Self {
            library,
            ui: Ui::new(Box::new(host), cx),
            host: host_id,
            signals,
            toolbar,
            palette,
            formats,
            zone,
            now,
            mode: Mode::Week,
            focus: today,
            mini_month: today.first_of_month(),
            scroll: DAY_START_HOUR * geometry::HOUR_HEIGHT,
            size: (0.0, 0.0),
            full_screen: false,
            shown: Vec::new(),
            hits: Vec::new(),
            drag: None,
            press: None,
            hover: None,
            selected: None,
            inspector: None,
            setup: None,
            confirm: None,
            search: String::new(),
            results: Vec::new(),
            status: None,
            requests: Vec::new(),
            scrolled_once: false,
            grid: None,
            pending_open: None,
        }
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn library_mut(&mut self) -> &mut Library {
        &mut self.library
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.close_inspector(true);
        let index = Mode::ALL
            .iter()
            .position(|shown| *shown == mode)
            .unwrap_or(1);
        let id = self.toolbar.modes;
        self.ui
            .update_widget::<unbound_toolkit::ui::SegmentedControl>(id, |control, cx| {
                control.set_selected(cx, index)
            });
    }

    pub fn go_to(&mut self, day: Date) {
        self.focus = day;
        self.mini_month = day.first_of_month();
    }

    pub fn focus_date(&self) -> Date {
        self.focus
    }

    pub fn set_now(&mut self, now: Zoned) {
        self.now = now;
    }

    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        self.full_screen = full_screen;
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
    }

    pub fn set_focused(&mut self, focused: bool) {
        if !focused {
            self.press = None;
        }
    }

    pub fn title(&self) -> String {
        let at = self
            .focus
            .to_zoned(self.zone.clone())
            .unwrap_or_else(|_| self.now.clone());
        match self.mode {
            Mode::Day => self.formats.long_date(&at),
            Mode::Week | Mode::Month => self.formats.month_title(self.focus.first_of_month()),
            Mode::Year => self.focus.year().to_string(),
        }
    }

    pub fn header(&self) -> Header {
        Header {
            title_x: self.toolbar.title_x(),
            items: self
                .toolbar
                .frames(self.size.0)
                .iter()
                .map(|(_, frame)| *frame)
                .collect(),
            tabs: Vec::new(),
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        let mut requests = std::mem::take(&mut self.requests);
        if let Some((menu, at)) = self.ui.take_menu_request() {
            requests.push(Request::Menu(menu, at));
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            requests.push(Request::Clipboard(request));
        }
        requests
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        if self.ui.menu_action(action) {
            self.drain();
            return true;
        }
        input::menu_command(self, action)
    }

    pub fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        let pasted = self.ui.pasted(text);
        self.drain();
        pasted
    }

    pub fn handle(&mut self, event: &unbound_toolkit::ui::Event) -> bool {
        input::handle(self, event)
    }

    pub fn cursor(&self) -> Cursor {
        match (&self.drag, &self.hover) {
            (Some(Drag::Resize { .. }), _) | (_, Some(Hit::Event(_, Part::End))) => {
                Cursor::ResizeBottom
            }
            (Some(Drag::Move { moved: true, .. }), _) => Cursor::Grabbing,
            _ => self.ui.cursor(),
        }
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let performed = self.ui.perform(request);
        self.drain();
        performed
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    pub fn has_sheet(&self) -> bool {
        self.ui.has_sheet()
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn open_account_setup(&mut self) {
        self.close_inspector(true);
        let (panel, setup) = setup::Setup::build(&self.signals);
        self.setup = Some(setup);
        self.ui.present_sheet(panel);
    }

    pub fn offer_import(&mut self, objects: Vec<CalendarObject>, file: &str) {
        let Some(calendar) = self.default_calendar() else {
            return;
        };
        let name = self.library.calendars[calendar].info.name.clone();
        let title = L10N.tr_args(
            "import-title",
            [("count", objects.len().into()), ("calendar", name.into())],
        );
        let message = L10N.tr_args("import-message", [("file", file.to_owned().into())]);
        self.confirm = Some(Confirm::Import(objects, calendar));
        let alert = Alert::new(&title, &message, self.choice())
            .cancel(&L10N.tr("cancel"), "cancel")
            .primary(&L10N.tr("import"), "accept");
        self.ui.present_dialog(alert);
    }

    pub fn offer_subscription(&mut self, url: &str) {
        let title = L10N.tr_args("subscribe-title", [("url", url.to_owned().into())]);
        self.confirm = Some(Confirm::Subscribe(url.to_owned()));
        let alert = Alert::new(&title, &L10N.tr("subscribe-message"), self.choice())
            .cancel(&L10N.tr("cancel"), "cancel")
            .primary(&L10N.tr("subscribe"), "accept");
        self.ui.present_dialog(alert);
    }

    pub fn account_failed(&mut self, message: &str) {
        match &mut self.setup {
            Some(setup) => setup.show_error(&mut self.ui, message),
            None => self.status = Some(message.to_owned()),
        }
    }

    pub fn account_added(&mut self) {
        if let Some(setup) = self.setup.take() {
            setup.close(&mut self.ui);
        }
        self.drain();
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.layout();
        let sheet = self.sheet();
        let model = ViewModel {
            library: &self.library,
            palette: self.palette,
            formats: &self.formats,
            zone: &self.zone,
            now: &self.now,
            mode: self.mode,
            focus: self.focus,
            scroll: self.scroll,
            drag: self.drag.as_ref(),
            selected: self.selected.as_ref(),
            hover: self.hover.as_ref(),
            results: &self.results,
            search: &self.search,
            sheet,
        };
        let mut painter = painter::Painter::new(&mut self.ui.context().text);
        let (shown, grid) = paint::view(&model, &mut painter);
        let view_items = std::mem::take(&mut painter.items);
        sidebar::paint(
            &model,
            self.mini_month,
            self.status.as_deref(),
            Rect::new(
                0.0,
                TOOLBAR_HEIGHT,
                SIDEBAR_WIDTH,
                self.size.1 - TOOLBAR_HEIGHT - 48.0,
            ),
            &mut painter,
        );
        let radius = match self.full_screen {
            true => 0.0,
            false => Radius::concentric(Radius::XL, SHEET_INSET as f32),
        };
        let mut items = std::mem::take(&mut painter.items);
        items.push(Item::Fill {
            shape: Shape::rounded(sheet, radius),
            paint: self.palette.sheet.into(),
        });
        items.push(Item::Clip {
            shape: Shape::rounded(sheet, radius),
            items: view_items,
        });
        self.hits = std::mem::take(&mut painter.hits);
        self.shown = shown;
        self.grid = grid;
        if let Some(selection) = self.pending_open.take() {
            let found = self
                .shown
                .iter()
                .chain(&self.results)
                .find(|found| self.selection_of(found) == selection)
                .cloned();
            if let Some(found) = found {
                self.open_inspector(found);
                self.layout();
            }
        }
        let (controls, dialog) = self.ui.paint_layers(width, height);
        items.extend(controls);
        items.extend(dialog);
        Scene {
            width,
            height,
            background: self.palette.chassis,
            items,
        }
    }

    pub(crate) fn sheet(&self) -> Rect {
        let inset = match self.full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
        Rect::new(
            SIDEBAR_WIDTH,
            TOOLBAR_HEIGHT,
            (self.size.0 - SIDEBAR_WIDTH - inset).max(0.0),
            (self.size.1 - TOOLBAR_HEIGHT - inset).max(0.0),
        )
    }

    fn layout(&mut self) {
        let frames = self.toolbar.frames(self.size.0);
        let add = self.toolbar.add_account;
        let add_frame = Rect::new(12.0, self.size.1 - 40.0, SIDEBAR_WIDTH - 24.0, 28.0);
        let sheet = self.sheet();
        let inspector_frame = self.inspector.as_ref().map(|inspector| {
            (
                inspector.panel,
                inspector.frame(sheet),
                inspector.arrow(sheet),
            )
        });
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
            host.set_frame(add, add_frame);
            if let Some((id, frame, _)) = inspector_frame {
                host.set_frame(id, frame);
            }
        }
        if let Some((id, _, arrow)) = inspector_frame {
            self.ui
                .update_widget::<panel::Panel>(id, |panel, _| panel.set_arrow(arrow));
        }
        self.ui.set_sheet_top(TOOLBAR_HEIGHT);
        if let Some(grid) =
            paint::time_grid(self.mode, self.focus, &self.formats, sheet, self.scroll, 1)
        {
            if !self.scrolled_once {
                self.scrolled_once = true;
                let now =
                    geometry::minutes_of(self.now.datetime()) as f32 * geometry::HOUR_HEIGHT / 60.0;
                let visible = grid.hours_area().height;
                if now > self.scroll + visible - geometry::HOUR_HEIGHT {
                    self.scroll = now - visible / 2.0;
                }
            }
            self.scroll = self.scroll.clamp(0.0, grid.max_scroll());
        }
    }

    pub(crate) fn shift(&mut self, steps: i32) {
        let steps = i64::from(steps);
        let day = match self.mode {
            Mode::Day => self.focus.saturating_add(steps.days()),
            Mode::Week => self.focus.saturating_add((7 * steps).days()),
            Mode::Month => self.focus.first_of_month().saturating_add(steps.months()),
            Mode::Year => self.focus.saturating_add(steps.years()),
        };
        self.close_inspector(true);
        self.go_to(day);
    }

    pub(crate) fn default_calendar(&self) -> Option<usize> {
        let usable = |index: &usize| {
            let info = &self.library.calendars[*index].info;
            !info.read_only && info.visible
        };
        self.selected
            .as_ref()
            .and_then(|selected| self.library.calendar_index(&selected.calendar))
            .filter(usable)
            .or_else(|| {
                self.library
                    .writable()
                    .find(|(index, calendar)| usable(index) && calendar.info.url.is_some())
                    .map(|(index, _)| index)
            })
            .or_else(|| self.library.writable().map(|(index, _)| index).find(usable))
            .or_else(|| self.library.writable().map(|(index, _)| index).next())
    }

    pub(crate) fn selection_of(&self, found: &Found) -> Selection {
        Selection {
            calendar: self
                .library
                .calendars
                .get(found.calendar)
                .map(|calendar| calendar.info.id.clone())
                .unwrap_or_default(),
            uid: found.occurrence.uid.clone(),
            key: found.occurrence.key,
        }
    }

    pub(crate) fn selected_found(&self) -> Option<Found> {
        let selected = self.selected.as_ref()?;
        self.shown
            .iter()
            .chain(&self.results)
            .find(|found| self.selection_of(found) == *selected)
            .cloned()
    }

    fn choice(&self) -> impl FnMut(&str) + 'static {
        let send = sender(&self.signals);
        move |choice| send(Signal::Choice(choice.to_owned()))
    }

    pub(crate) fn ask_scope(&mut self, confirm: Confirm, title: &str) {
        self.confirm = Some(confirm);
        let alert = Alert::new(title, "", self.choice())
            .cancel(&L10N.tr("cancel"), "cancel")
            .other(&L10N.tr("only-this"), "occurrence")
            .primary(&L10N.tr("all-events"), "series");
        self.ui.present_dialog(alert);
    }

    pub(crate) fn ask_remove_account(&mut self, id: &str) {
        let Some(account) = self.library.account(id) else {
            return;
        };
        let title = L10N.tr_args(
            "remove-account-title",
            [("name", account.name.clone().into())],
        );
        self.confirm = Some(Confirm::RemoveAccount(id.to_owned()));
        let alert = Alert::new(&title, &L10N.tr("remove-account-message"), self.choice())
            .cancel(&L10N.tr("cancel"), "cancel")
            .destructive(&L10N.tr("remove-account"), "accept");
        self.ui.present_dialog(alert);
    }

    pub(crate) fn ask_delete(&mut self, found: Found) {
        self.close_inspector(false);
        match found.occurrence.recurring {
            true => self.ask_scope(Confirm::Delete(found), &L10N.tr("delete-recurring-title")),
            false => self.apply_delete(&found, Scope::Series),
        }
    }

    pub(crate) fn apply_delete(&mut self, found: &Found, scope: Scope) {
        let Some(object) = self.library.object(found.calendar, found.object).cloned() else {
            return;
        };
        match crate::edit::remove_occurrence(&object, found.occurrence.key, scope) {
            Some(kept) => {
                self.library.put_object(found.calendar, kept);
            }
            None => self.library.remove_object(found.calendar, found.object),
        }
        self.selected = None;
        self.requests.push(Request::Changed);
    }

    pub(crate) fn apply_reschedule(
        &mut self,
        found: &Found,
        start: DateTime,
        end: DateTime,
        all_day: bool,
        scope: Scope,
    ) {
        let Some(object) = self.library.object(found.calendar, found.object) else {
            return;
        };
        let moved = crate::edit::reschedule(
            object,
            found.occurrence.event,
            found.occurrence.key,
            start,
            end,
            all_day,
            scope,
            &self.zone,
        );
        self.library.put_object(found.calendar, moved);
        self.requests.push(Request::Changed);
    }

    pub(crate) fn apply_details(
        &mut self,
        found: &Found,
        details: &Details,
        calendar: Option<usize>,
        scope: Scope,
    ) {
        let Some(object) = self.library.object(found.calendar, found.object) else {
            return;
        };
        let edited = crate::edit::apply_details(
            object,
            found.occurrence.event,
            found.occurrence.key,
            details,
            scope,
            &self.zone,
        );
        let uid = edited.uid().to_owned();
        let target = calendar.filter(|target| *target != found.calendar);
        let Some(index) = self.library.put_object(found.calendar, edited) else {
            return;
        };
        if let Some(target) = target {
            self.library.move_object(found.calendar, index, target);
        }
        self.selected = Some(Selection {
            calendar: self.library.calendars[target.unwrap_or(found.calendar)]
                .info
                .id
                .clone(),
            uid,
            key: found.occurrence.key,
        });
        self.requests.push(Request::Changed);
    }

    pub(crate) fn open_inspector(&mut self, found: Found) {
        self.close_inspector(true);
        let Some(event) = self.library.event(&found).cloned() else {
            return;
        };
        let read_only = self
            .library
            .calendars
            .get(found.calendar)
            .is_none_or(|calendar| calendar.info.read_only);
        let calendars: Vec<(usize, String)> = match read_only {
            true => Vec::new(),
            false => self
                .library
                .writable()
                .map(|(index, calendar)| {
                    let account = self.library.account(&calendar.info.account);
                    let name = match account.filter(|account| account.id != LOCAL_ACCOUNT) {
                        Some(account) => format!("{} ({})", calendar.info.name, account.name),
                        None => calendar.info.name.clone(),
                    };
                    (index, name)
                })
                .collect(),
        };
        let anchor = self
            .hits
            .iter()
            .find(|(_, hit)| matches!(hit, Hit::Event(index, Part::Body) if self.shown.get(*index) == Some(&found)))
            .map(|(rect, _)| *rect);
        let (panel, inspector) =
            inspector::Inspector::build(&self.signals, found, &event, read_only, calendars, anchor);
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.insert(panel, Rect::default());
        }
        let title = inspector.title_field;
        self.inspector = Some(inspector);
        if let Some(title) = title {
            self.ui.set_focus(title);
        }
    }

    pub(crate) fn close_inspector(&mut self, commit: bool) {
        let Some(inspector) = self.inspector.take() else {
            return;
        };
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.remove(inspector.panel);
        }
        self.ui.clear_focus();
        if !commit || inspector.read_only {
            return;
        }
        let Some(details) = inspector.edited_details() else {
            return;
        };
        let calendar = inspector.chosen_calendar();
        let found = inspector.found.clone();
        let times_changed = details.start != inspector.original.start
            || details.end != inspector.original.end
            || details.all_day != inspector.original.all_day;
        match found.occurrence.recurring && times_changed {
            true => self.ask_scope(
                Confirm::Details {
                    found,
                    details,
                    calendar,
                },
                &L10N.tr("change-recurring-title"),
            ),
            false => self.apply_details(&found, &details, calendar, Scope::Series),
        }
    }

    pub(crate) fn drain(&mut self) {
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() {
                return;
            }
            for signal in signals {
                input::signal(self, signal);
            }
        }
    }

    pub fn refresh_search(&mut self) {
        self.results = match self.search.trim().is_empty() {
            true => Vec::new(),
            false => self
                .library
                .search(&self.search, self.now.timestamp(), &self.zone),
        };
    }
}

pub(crate) struct ViewModel<'a> {
    pub library: &'a Library,
    pub palette: Palette,
    pub formats: &'a DateFormats,
    pub zone: &'a TimeZone,
    pub now: &'a Zoned,
    pub mode: Mode,
    pub focus: Date,
    pub scroll: f32,
    pub drag: Option<&'a Drag>,
    pub selected: Option<&'a Selection>,
    pub hover: Option<&'a Hit>,
    pub results: &'a [Found],
    pub search: &'a str,
    pub sheet: Rect,
}

impl ViewModel<'_> {
    pub fn is_selected(&self, found: &Found) -> bool {
        self.selected.is_some_and(|selected| {
            self.library
                .calendars
                .get(found.calendar)
                .is_some_and(|calendar| calendar.info.id == selected.calendar)
                && found.occurrence.uid == selected.uid
                && found.occurrence.key == selected.key
        })
    }
}
