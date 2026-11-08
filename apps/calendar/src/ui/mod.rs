mod custom;
pub mod geometry;
mod input;
mod inspector;
mod naming;
mod paint;
pub mod painter;
mod panel;
mod settings_view;
mod setup;
mod sidebar;
mod toolbar;

use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::tr;
use accesskit::{ActionRequest, TreeUpdate};
use bande_datetime::DateFormats;
use bande_toolkit::ui::{Alert, ClipboardRequest, Context, Cursor, FreeLayout, Menu, Ui, WidgetId};
use bande_toolkit::{
    Appearance, Header, Item, Palette, Point, Radius, Rect, SHEET_INSET, Scene, Shape, TextSystem,
};
use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};

use crate::edit::{Details, Scope};
use crate::ical::{CalendarObject, InstanceKey};
use crate::settings::CalendarSettings;
use crate::store::{AccountKind, CalendarInfo, Found, LOCAL_ACCOUNT, Library, Pending, new_uid};
use bande_design::{Accent, Scheme};

pub use settings_view::{SETTINGS_SIZE, SettingsView};
pub use toolbar::TOOLBAR_HEIGHT;

pub const SIDEBAR_WIDTH: f32 = 220.0;
const MAX_ZONES: usize = 12;

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
        match self {
            Self::Day => tr!("Day"),
            Self::Week => tr!("Week"),
            Self::Month => tr!("Month"),
            Self::Year => tr!("Year"),
        }
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
    Collection {
        account: String,
        url: String,
        change: crate::worker::Change,
    },
    Changed,
    SetPassword {
        account: String,
        user: String,
        password: String,
    },
    Export {
        name: String,
        data: String,
    },
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
    EndRepeatKind(usize),
    CustomFrequency(usize),
    CustomInterval(String),
    CustomDay(usize, bool),
    CustomOnWeekday(usize, bool),
    CustomDone,
    CustomCancel,
    Invitees(String),
    TimeZone(String),
    Answer(usize),
    EndRepeatText(String),
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
    NameText(String),
    NameAccount(usize),
    NameSubmit,
    NameCancel,
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
    More(Date),
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
        details: Box<Details>,
        calendar: Option<usize>,
    },
    Delete(Found),
    RemoveAccount(String),
    Import(Vec<CalendarObject>, usize),
    Subscribe(String),
    DeleteCalendar(usize),
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
    settings: CalendarSettings,
    zone: TimeZone,
    system_zone: TimeZone,
    shown_zone: Option<String>,
    now: Zoned,
    mode: Mode,
    focus: Date,
    mini_month: Date,
    scroll: f32,
    hour_height: f32,
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
    custom: Option<custom::Custom>,
    naming: Option<naming::Naming>,
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
        settings: CalendarSettings,
        zone: TimeZone,
        now: Zoned,
    ) -> Self {
        let signals: Signals = Rc::new(RefCell::new(Vec::new()));
        let mut host = FreeLayout::new();
        let toolbar = toolbar::Toolbar::new(&mut host, &signals, Mode::Week);
        let host_id = bande_toolkit::ui::Widget::id(&host);
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
            system_zone: zone.clone(),
            shown_zone: None,
            zone,
            now,
            mode: Mode::Week,
            focus: today,
            mini_month: today.first_of_month(),
            scroll: f32::from(settings.day_start) * geometry::HOUR_HEIGHT,
            hour_height: geometry::HOUR_HEIGHT,
            settings,
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
            custom: None,
            naming: None,
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
            .update_widget::<bande_toolkit::ui::SegmentedControl>(id, |control, cx| {
                control.set_selected(cx, index)
            });
    }

    pub fn show_event(&mut self, uid: &str) {
        let day = self.library.calendars.iter().find_map(|calendar| {
            let object = calendar.objects.get(calendar.find(uid)?)?;
            Some(object.parsed.as_ref()?.events.first()?.start.date())
        });
        if let Some(day) = day {
            self.go_to(day);
        }
    }

    pub fn go_to(&mut self, day: Date) {
        self.focus = day;
        self.mini_month = day.first_of_month();
    }

    pub fn set_settings(&mut self, settings: CalendarSettings) {
        if !settings.time_zones && self.shown_zone.is_some() {
            self.view_in(None);
        }
        if settings.day_start != self.settings.day_start {
            self.scroll = f32::from(settings.day_start) * self.hour_height;
        }
        self.settings = settings;
    }

    pub(crate) fn first_weekday(&self) -> Weekday {
        self.settings
            .first_weekday
            .unwrap_or(self.formats.first_weekday())
    }

    pub fn set_system_zone(&mut self, zone: TimeZone) {
        self.system_zone = zone;
        let shown = self.shown_zone.take();
        self.view_in(shown);
    }

    pub fn shown_zone(&self) -> Option<&str> {
        self.shown_zone.as_deref()
    }

    pub(crate) fn view_in(&mut self, name: Option<String>) {
        let zone = name.as_deref().and_then(|name| TimeZone::get(name).ok());
        self.shown_zone = name.filter(|_| zone.is_some());
        self.zone = zone.unwrap_or_else(|| self.system_zone.clone());
        self.now = self.now.with_time_zone(self.zone.clone());
        self.close_inspector(true);
        self.refresh_search();
    }

    pub fn zones(&self) -> Vec<String> {
        let zones: std::collections::BTreeSet<&str> = self
            .library
            .calendars
            .iter()
            .filter(|calendar| calendar.info.visible)
            .flat_map(|calendar| &calendar.objects)
            .filter_map(|stored| match &stored.parsed.as_ref()?.master()?.start {
                crate::ical::Moment::Zoned {
                    tzid: Some(tzid), ..
                } => Some(tzid.as_str()),
                crate::ical::Moment::Zoned { tzid: None, .. } => Some("UTC"),
                _ => None,
            })
            .chain(self.shown_zone.as_deref())
            .collect();
        zones
            .into_iter()
            .take(MAX_ZONES)
            .map(str::to_owned)
            .collect()
    }

    pub fn set_now(&mut self, now: Zoned) {
        self.now = now.with_time_zone(self.zone.clone());
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

    pub fn printable(&self) -> (String, String) {
        let first = self.focus.first_of_month();
        let next = first.saturating_add(1.month());
        let moment = |day: Date| {
            day.to_zoned(self.zone.clone())
                .map_or_else(|_| self.now.timestamp(), |zoned| zoned.timestamp())
        };
        let mut found = self
            .library
            .occurrences(moment(first), moment(next), &self.zone);
        found.sort_by(|a, b| a.occurrence.start.cmp(&b.occurrence.start));
        let mut text = String::new();
        let mut day = None;
        for one in &found {
            let start = &one.occurrence.start;
            if day != Some(start.date()) {
                day = Some(start.date());
                text.push_str(&format!("\n{}\n", self.formats.long_date(start)));
            }
            let summary = self
                .library
                .event(one)
                .map_or_else(String::new, |event| event.summary.clone());
            match one.occurrence.all_day {
                true => text.push_str(&format!("{summary}\n")),
                false => text.push_str(&format!("{}  {summary}\n", start.strftime("%H:%M"))),
            }
        }
        (self.formats.month_title(first), text)
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

    pub fn handle(&mut self, event: &bande_toolkit::ui::Event) -> bool {
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

    pub fn open_new_calendar(&mut self, account: Option<&str>) {
        self.close_inspector(true);
        let accounts: Vec<(String, String)> = self
            .library
            .accounts
            .iter()
            .filter_map(|known| match known.kind {
                AccountKind::Local => Some((known.id.clone(), tr!("On My Computer"))),
                AccountKind::CalDav { .. } => Some((known.id.clone(), known.name.clone())),
                AccountKind::Subscription { .. } => None,
            })
            .collect();
        let chosen = account
            .and_then(|account| accounts.iter().position(|(id, _)| id == account))
            .unwrap_or(0);
        let labels: Vec<String> = accounts.iter().map(|(_, label)| label.clone()).collect();
        let ids = accounts.into_iter().map(|(id, _)| id).collect();
        let (panel, naming) =
            naming::Naming::build(&self.signals, naming::Target::New(ids), "", &labels, chosen);
        self.present_naming(panel, naming);
    }

    pub(crate) fn open_rename(&mut self, index: usize) {
        let Some(calendar) = self.library.calendars.get(index) else {
            return;
        };
        let name = calendar.info.name.clone();
        let (panel, naming) =
            naming::Naming::build(&self.signals, naming::Target::Rename(index), &name, &[], 0);
        self.present_naming(panel, naming);
    }

    fn present_naming(&mut self, panel: panel::Panel, naming: naming::Naming) {
        let field = naming.field;
        self.naming = Some(naming);
        self.ui.present_sheet(panel);
        if let Some(field) = field {
            self.ui.set_focus(field);
        }
    }

    pub(crate) fn submit_naming(&mut self) {
        let Some(naming) = self.naming.take() else {
            return;
        };
        naming.close(&mut self.ui);
        let name = match naming.name.trim() {
            "" => tr!("Untitled"),
            name => name.to_owned(),
        };
        match naming.target {
            naming::Target::Rename(index) => {
                self.library.rename_calendar(index, &name);
                let info = &self.library.calendars[index].info;
                if let Some(url) = info.url.clone() {
                    self.requests.push(Request::Collection {
                        account: info.account.clone(),
                        url,
                        change: crate::worker::Change::Rename(name),
                    });
                }
            }
            naming::Target::New(accounts) => {
                let Some(account) = accounts
                    .get(naming.account)
                    .and_then(|id| self.library.account(id))
                    .cloned()
                else {
                    return;
                };
                let taken: Vec<Accent> = self
                    .library
                    .calendars
                    .iter()
                    .filter_map(|calendar| painter::parse_color(&calendar.info.color))
                    .map(Accent::nearest)
                    .collect();
                let accent = Accent::ALL
                    .into_iter()
                    .find(|accent| !taken.contains(accent))
                    .unwrap_or(Accent::ALL[self.library.calendars.len() % Accent::ALL.len()]);
                let color = painter::color_text(accent.color(Scheme::Light));
                match account.kind {
                    AccountKind::CalDav { home, .. } => self.requests.push(Request::Collection {
                        account: account.id,
                        url: format!("{}/{}/", home.trim_end_matches('/'), new_uid()),
                        change: crate::worker::Change::Make {
                            name,
                            color: format!("{color}FF"),
                        },
                    }),
                    _ => {
                        self.library.add_calendar(CalendarInfo {
                            id: new_uid(),
                            account: account.id,
                            url: None,
                            name,
                            color,
                            read_only: false,
                            visible: true,
                            ctag: None,
                            sync_token: None,
                            refreshed: None,
                        });
                    }
                }
            }
        }
    }

    pub(crate) fn cancel_naming(&mut self) {
        if let Some(naming) = self.naming.take() {
            naming.close(&mut self.ui);
        }
    }

    pub(crate) fn ask_delete_calendar(&mut self, index: usize) {
        let Some(calendar) = self.library.calendars.get(index) else {
            return;
        };
        let title = tr!("Delete “{name}”?", name = calendar.info.name.clone());
        self.confirm = Some(Confirm::DeleteCalendar(index));
        let alert = Alert::new(
            &title,
            &tr!("Its events are deleted too, also on the server."),
            self.choice(),
        )
        .cancel(&tr!("Cancel"), "cancel")
        .destructive(&tr!("Delete"), "accept");
        self.ui.present_dialog(alert);
    }

    pub(crate) fn delete_calendar(&mut self, index: usize) {
        let Some(info) = self
            .library
            .calendars
            .get(index)
            .map(|calendar| calendar.info.clone())
        else {
            return;
        };
        if let Some(url) = info.url {
            self.requests.push(Request::Collection {
                account: info.account,
                url,
                change: crate::worker::Change::Delete,
            });
        }
        self.library.remove_calendar(index);
        self.selected = None;
        self.shown.clear();
        self.results.clear();
    }

    pub(crate) fn export_calendar(&mut self, index: Option<usize>) {
        let index = index
            .or_else(|| self.selected_found().map(|found| found.calendar))
            .or_else(|| self.default_calendar());
        let Some(calendar) = index.and_then(|index| self.library.calendars.get(index)) else {
            return;
        };
        let data = crate::ical::export(
            &calendar.info.name,
            calendar
                .objects
                .iter()
                .filter(|stored| stored.pending != Some(Pending::Delete))
                .map(|stored| stored.data.as_str()),
        );
        self.requests.push(Request::Export {
            name: format!("{}.ics", calendar.info.name),
            data,
        });
    }

    pub fn ask_password(&mut self, account: &str) {
        if self.setup.is_some() {
            return;
        }
        let Some((name, user)) =
            self.library
                .account(account)
                .and_then(|known| match &known.kind {
                    AccountKind::CalDav { user, .. } => Some((known.name.clone(), user.clone())),
                    _ => None,
                })
        else {
            return;
        };
        self.close_inspector(true);
        let (panel, setup) = setup::Setup::sign_in(&self.signals, account, &name, &user);
        let field = setup.password_field();
        self.setup = Some(setup);
        self.ui.present_sheet(panel);
        if let Some(field) = field {
            self.ui.set_focus(field);
        }
    }

    pub(crate) fn open_custom(&mut self) {
        let Some(found) = self
            .inspector
            .as_ref()
            .map(|inspector| inspector.found.clone())
        else {
            return;
        };
        let rule = self
            .library
            .object(found.calendar, found.object)
            .and_then(|object| object.master())
            .and_then(|master| master.rule.clone());
        let (panel, custom) = custom::Custom::build(
            &self.signals,
            rule.as_ref(),
            found.occurrence.start.date(),
            &self.formats,
            self.first_weekday(),
        );
        self.custom = Some(custom);
        self.ui.present_sheet(panel);
    }

    pub(crate) fn finish_custom(&mut self, done: bool) {
        let Some(custom) = self.custom.take() else {
            return;
        };
        if done && let Some(inspector) = &mut self.inspector {
            inspector.set_custom(custom.rule());
        }
        custom.close(&mut self.ui);
    }

    pub fn open_account_setup(&mut self) {
        self.close_inspector(true);
        let (panel, setup) = setup::Setup::build(&self.signals);
        self.setup = Some(setup);
        self.ui.present_sheet(panel);
    }

    pub fn offer_import(&mut self, objects: Vec<CalendarObject>, file: &str) -> bool {
        let Some(calendar) = self.default_calendar() else {
            return false;
        };
        let name = self.library.calendars[calendar].info.name.clone();
        let title = tr!(
            "Add {count} events to “{calendar}”?",
            count = objects.len(),
            calendar = name
        );
        let message = tr!("From {file}", file = file.to_owned());
        self.confirm = Some(Confirm::Import(objects, calendar));
        let alert = Alert::new(&title, &message, self.choice())
            .cancel(&tr!("Cancel"), "cancel")
            .primary(&tr!("Add"), "accept");
        self.ui.present_dialog(alert);
        true
    }

    pub fn offer_subscription(&mut self, url: &str) {
        let title = tr!("Subscribe to “{url}”?", url = url.to_owned());
        self.confirm = Some(Confirm::Subscribe(url.to_owned()));
        let alert = Alert::new(
            &title,
            &tr!("The calendar is read-only and refreshes as set in Calendar Settings."),
            self.choice(),
        )
        .cancel(&tr!("Cancel"), "cancel")
        .primary(&tr!("Subscribe"), "accept");
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
            first_weekday: self.first_weekday(),
            work_hours: self.settings.work_hours(),
            week_numbers: self.settings.week_numbers,
            hours_shown: self.settings.hours_shown,
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
        if let Some(grid) = paint::time_grid(
            self.mode,
            self.focus,
            self.first_weekday(),
            sheet,
            self.scroll,
            self.settings.hours_shown,
        ) {
            if grid.hour_height != self.hour_height {
                self.scroll *= grid.hour_height / self.hour_height;
                self.hour_height = grid.hour_height;
            }
            if !self.scrolled_once {
                self.scrolled_once = true;
                let now =
                    geometry::minutes_of(self.now.datetime()) as f32 * grid.hour_height / 60.0;
                let visible = grid.hours_area().height;
                if now > self.scroll + visible - grid.hour_height {
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
        self.library
            .calendar_index(&self.settings.default_calendar)
            .filter(usable)
            .or_else(|| {
                self.selected
                    .as_ref()
                    .and_then(|selected| self.library.calendar_index(&selected.calendar))
                    .filter(usable)
            })
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
            .cancel(&tr!("Cancel"), "cancel")
            .other(&tr!("Only This Event"), "occurrence")
            .primary(&tr!("All Events"), "series");
        self.ui.present_dialog(alert);
    }

    pub fn ask_remove_account(&mut self, id: &str) {
        let Some(account) = self.library.account(id) else {
            return;
        };
        let title = tr!("Remove “{name}”?", name = account.name.clone());
        self.confirm = Some(Confirm::RemoveAccount(id.to_owned()));
        let alert = Alert::new(
            &title,
            &tr!("Its calendars are removed from this computer. Nothing is deleted on the server."),
            self.choice(),
        )
        .cancel(&tr!("Cancel"), "cancel")
        .destructive(&tr!("Remove Account"), "accept");
        self.ui.present_dialog(alert);
    }

    pub(crate) fn ask_delete(&mut self, found: Found) {
        self.close_inspector(false);
        match found.occurrence.recurring {
            true => self.ask_scope(
                Confirm::Delete(found),
                &tr!("Do you want to delete only this event, or all events in the series?"),
            ),
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
        let mut edited = edited;
        if let Some(me) = self.me(found.calendar)
            && let Some(master) = edited.master_mut()
        {
            crate::invite::invite(master, &me, &details.invitees);
        }
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

    pub(crate) fn me(&self, calendar: usize) -> Option<Vec<String>> {
        let info = &self.library.calendars.get(calendar)?.info;
        match &self.library.account(&info.account)?.kind {
            AccountKind::CalDav { addresses, .. } if !addresses.is_empty() => {
                Some(addresses.clone())
            }
            AccountKind::CalDav { user, .. } if user.contains('@') => Some(vec![user.clone()]),
            _ => None,
        }
    }

    pub(crate) fn answer(&mut self, found: &Found, index: usize) {
        let (Some(me), Some(answer)) = (
            self.me(found.calendar),
            crate::invite::Answer::CHOICES.get(index),
        ) else {
            return;
        };
        let Some(object) = self.library.object(found.calendar, found.object) else {
            return;
        };
        let answered = crate::invite::answer(object, &me, *answer);
        self.library.put_object(found.calendar, answered);
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
        let me = self.me(found.calendar);
        let (panel, inspector) = inspector::Inspector::build(
            &self.signals,
            found,
            &event,
            read_only,
            calendars,
            anchor,
            &inspector::Context {
                local: &self.zone,
                me: me.as_deref(),
                time_zones: self.settings.time_zones,
            },
        );
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
                    details: Box::new(details),
                    calendar,
                },
                &tr!("Do you want to change only this event, or all events in the series?"),
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
    pub first_weekday: Weekday,
    pub work_hours: (u8, u8),
    pub week_numbers: bool,
    pub hours_shown: u8,
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
