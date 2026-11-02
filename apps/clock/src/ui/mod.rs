mod widgets;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use bande_app::View;
use bande_datetime::DateFormats;
use bande_datetime::daylight::{Sky, place_of, sky};
use bande_l10n::Localizer;
use bande_sound::Alert as Sound;
use bande_toolkit::ui::{
    Alert, Axis, BUTTON_HEIGHT, Button, CheckState, Checkbox, Column, Context, Form, FormLine,
    FreeLayout, Label, PopUpButton, Row, SEGMENTED_HEIGHT, ScrollState, ScrollView, SearchField,
    SegmentedControl, Stack, Stepper, Table, TextField, Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, Header, Image, Rect, Scene, TextSystem, WINDOW_HEADER_HEIGHT, Window, WindowEvent,
    window_controls_end,
};
use jiff::civil::{Date, Weekday, date};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use self::widgets::{Digits, Fixed, Line};
use crate::cities::{self, City, city_name};
use crate::model::{Alarm, Clock, DEFAULT_SOUND, Days, Timer, TimerState, boot_now};

const ENGLISH: &str = include_str!("../../locales/en-US/bande-clock.ftl");
pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

const HEADER: f32 = WINDOW_HEADER_HEIGHT as f32;
const SEGMENT_WIDTH: f32 = 96.0;
const SIDE: f32 = 12.0;
const BIG: f32 = 40.0;
const HUGE: f32 = 64.0;
const SMALL: f32 = 15.0;
const PADDING: f32 = 16.0;
const INSET: f32 = 12.0;
const CITY_LIST_HEIGHT: f32 = 260.0;
const PRESETS: [u64; 7] = [60, 180, 300, 600, 900, 1800, 3600];
const MONDAY: Date = date(2024, 1, 1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Alarms,
    Timers,
    Stopwatch,
    World,
}

impl Page {
    pub const ALL: [Self; 4] = [Self::World, Self::Alarms, Self::Stopwatch, Self::Timers];

    fn key(self) -> &'static str {
        match self {
            Self::Alarms => "alarms",
            Self::Timers => "timers",
            Self::Stopwatch => "stopwatch",
            Self::World => "world-clock",
        }
    }

    pub fn title(self) -> String {
        L10N.tr(self.key())
    }

    pub fn command(self) -> String {
        format!("show-{}", self.key())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    SaveAlarm(Alarm),
    DeleteAlarm(u64),
    EnableAlarm(u64, bool),
    NewTimer(Timer),
    StartPause(u64),
    ResetTimer(u64),
    DeleteTimer(u64),
    StopwatchToggle,
    StopwatchLap,
    StopwatchReset,
    AddCity(String),
    RemoveCity(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Field {
    Hour(f64),
    Minute(f64),
    Second(f64),
    Day(Weekday),
    Label(String),
    Sound(usize),
    Snooze(bool),
}

#[derive(Debug, Clone, PartialEq)]
enum Intent {
    Page(usize),
    Add,
    Edit(u64),
    Change(Change),
    Field(Field),
    Sheet(String),
    Search(String),
    Chosen(Vec<String>),
}

type Intents = Rc<RefCell<Vec<Intent>>>;

fn sender(intents: &Intents) -> impl Fn(Intent) + 'static {
    let intents = Rc::clone(intents);
    move |intent| intents.borrow_mut().push(intent)
}

#[derive(Debug, Clone, PartialEq)]
struct TimerDraft {
    hours: u64,
    minutes: u64,
    seconds: u64,
    label: String,
    sound: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Sheet {
    Alarm(Alarm),
    Timer(TimerDraft),
    City { list: WidgetId, chosen: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Live {
    Timer(u64),
    Stopwatch,
    Lap,
    City(String),
}

pub struct ClockView {
    ui: Ui,
    root: WidgetId,
    tabs: WidgetId,
    add: WidgetId,
    body: WidgetId,
    page: Page,
    clock: Clock,
    zone: TimeZone,
    formats: DateFormats,
    intents: Intents,
    changes: Vec<Change>,
    sheet: Option<Sheet>,
    cities: Vec<City>,
    live: Vec<(WidgetId, Live)>,
    scroll: Rc<RefCell<ScrollState>>,
    visible: bool,
}

const PLUS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" stroke="#201E1A" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>"##;
const SYMBOL_PIXELS: u32 = 16;

fn add_button(on_click: impl FnMut() + 'static) -> Button {
    match Image::from_svg(PLUS.as_bytes(), SYMBOL_PIXELS, SYMBOL_PIXELS) {
        Some(image) => Button::symbol(image, &L10N.tr("add"), on_click),
        None => Button::new("+", on_click),
    }
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

pub fn clock_time(formats: &DateFormats, at: &Zoned) -> String {
    let (time, period) = formats.time(at);
    period.map_or(time.clone(), |period| format!("{time} {period}"))
}

fn whole_seconds(left: Duration) -> u64 {
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

pub fn countdown(left: Duration) -> String {
    let seconds = whole_seconds(left);
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    match hours {
        0 => format!("{minutes:02}:{seconds:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

fn lap_time(elapsed: Duration) -> String {
    let centis = elapsed.as_millis() / 10;
    let (hours, minutes, seconds, centis) = (
        centis / 360_000,
        centis / 6000 % 60,
        centis / 100 % 60,
        centis % 100,
    );
    match hours {
        0 => format!("{minutes:02}:{seconds:02}.{centis:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}.{centis:02}"),
    }
}

pub fn length_name(length: Duration) -> String {
    let seconds = length.as_secs();
    let (hours, minutes, rest) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(L10N.tr_args("hours", [("count", hours.into())]));
    }
    if minutes > 0 {
        parts.push(L10N.tr_args("minutes", [("count", minutes.into())]));
    }
    if rest > 0 || parts.is_empty() {
        parts.push(L10N.tr_args("seconds", [("count", rest.into())]));
    }
    parts.join(" ")
}

pub fn timer_name(timer: &Timer) -> String {
    match timer.label.trim().is_empty() {
        true => length_name(timer.length),
        false => timer.label.clone(),
    }
}

pub fn alarm_name(alarm: &Alarm) -> String {
    match alarm.label.trim().is_empty() {
        true => L10N.tr("alarm"),
        false => alarm.label.clone(),
    }
}

fn weekday_date(day: Weekday) -> Date {
    MONDAY
        .checked_add(jiff::Span::new().days(day.to_monday_zero_offset()))
        .unwrap_or(MONDAY)
}

fn short_weekday(formats: &DateFormats, day: Weekday) -> String {
    formats.short_weekday(weekday_date(day))
}

fn repeat_text(formats: &DateFormats, days: Days) -> String {
    match days {
        Days::ONCE => String::new(),
        Days::WEEKDAYS => L10N.tr("repeat-weekdays"),
        Days::WEEKEND => L10N.tr("repeat-weekends"),
        Days::EVERY_DAY => L10N.tr("repeat-every-day"),
        days => weekdays()
            .filter(|day| days.has(*day))
            .map(|day| short_weekday(formats, day))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn weekdays() -> impl Iterator<Item = Weekday> {
    (0..7).filter_map(|offset| Weekday::from_monday_zero_offset(offset).ok())
}

fn sound_title(sound: Sound) -> String {
    L10N.tr(&format!("sound-{}", sound.keyword()))
}

fn sound_index(keyword: &str) -> usize {
    Sound::ALL
        .iter()
        .position(|sound| sound.keyword() == keyword)
        .unwrap_or(0)
}

fn offset_text(city: &Zoned, here: &Zoned) -> String {
    let day = match city.date().cmp(&here.date()) {
        std::cmp::Ordering::Less => L10N.tr("yesterday"),
        std::cmp::Ordering::Equal => L10N.tr("today"),
        std::cmp::Ordering::Greater => L10N.tr("tomorrow"),
    };
    let difference = city.offset().seconds() - here.offset().seconds();
    if difference == 0 {
        return day;
    }
    let sign = if difference > 0 { "+" } else { "−" };
    let minutes = difference.abs() / 60;
    let hours = match minutes % 60 {
        0 => L10N.tr_args("hours-short", [("count", (minutes / 60).into())]),
        rest => format!("{}:{rest:02}", minutes / 60),
    };
    L10N.tr_args(
        "city-offset",
        [
            ("day", day.into()),
            ("difference", format!("{sign}{hours}").into()),
        ],
    )
}

impl ClockView {
    pub fn new(
        appearance: Appearance,
        clock: &Clock,
        zone: TimeZone,
        formats: DateFormats,
    ) -> Self {
        let intents = Intents::default();
        let mut root = FreeLayout::new();
        let page = Page::Alarms;
        let titles: Vec<String> = Page::ALL.iter().map(|page| page.title()).collect();
        let send = sender(&intents);
        let tabs = root.insert_in_header(
            SegmentedControl::new(
                &L10N.tr("app-name"),
                &titles,
                Page::ALL.iter().position(|shown| *shown == page),
                move |index| send(Intent::Page(index)),
            )
            .fill_equally(),
            Rect::default(),
        );
        let send = sender(&intents);
        let add = root.insert_in_header(
            add_button(move || send(Intent::Add)).tooltip(&L10N.tr("add")),
            Rect::default(),
        );
        let body = root.insert(Label::new(""), Rect::default());
        let root_id = root.id();
        let mut ui = Ui::new(
            Box::new(root),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui.set_sheet_top(HEADER);
        let mut view = Self {
            ui,
            root: root_id,
            tabs,
            add,
            body,
            page,
            clock: clock.clone(),
            zone,
            formats,
            intents,
            changes: Vec::new(),
            sheet: None,
            cities: Vec::new(),
            live: Vec::new(),
            scroll: Rc::default(),
            visible: true,
        };
        view.rebuild();
        view
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn show(&mut self, clock: &Clock) {
        if *clock != self.clock {
            self.clock = clock.clone();
            self.rebuild();
        }
    }

    pub fn set_zone(&mut self, zone: TimeZone) {
        self.zone = zone;
        self.rebuild();
    }

    pub fn set_page(&mut self, page: Page) {
        if page == self.page {
            return;
        }
        self.page = page;
        self.scroll = Rc::default();
        if let Some(index) = Page::ALL.iter().position(|shown| *shown == page) {
            let selected = index;
            self.ui
                .update_widget::<SegmentedControl>(self.tabs, |tabs, cx| {
                    tabs.set_selected(cx, selected)
                });
        }
        self.rebuild();
    }

    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    pub fn command(&mut self, command: &str) -> bool {
        if let Some(page) = Page::ALL.iter().find(|page| page.command() == command) {
            self.set_page(*page);
            return true;
        }
        let change = match command {
            "add" => {
                self.open_sheet(None);
                return true;
            }
            "stopwatch-toggle" => Change::StopwatchToggle,
            "stopwatch-lap" if self.clock.stopwatch.started.is_some() => Change::StopwatchLap,
            "stopwatch-lap" => Change::StopwatchReset,
            _ => return false,
        };
        self.set_page(Page::Stopwatch);
        self.changes.push(change);
        true
    }

    pub fn can_add(&self) -> bool {
        self.page != Page::Stopwatch
    }

    fn pump(&mut self) -> bool {
        let intents = std::mem::take(&mut *self.intents.borrow_mut());
        let acted = !intents.is_empty();
        for intent in intents {
            match intent {
                Intent::Page(index) => {
                    if let Some(page) = Page::ALL.get(index) {
                        self.page = *page;
                        self.scroll = Rc::default();
                        self.rebuild();
                    }
                }
                Intent::Add => self.open_sheet(None),
                Intent::Edit(id) => self.open_sheet(Some(id)),
                Intent::Change(change) => self.changes.push(change),
                Intent::Field(field) => self.edit_draft(field),
                Intent::Sheet(action) => self.finish_sheet(&action),
                Intent::Search(text) => self.search(&text),
                Intent::Chosen(keys) => {
                    if let Some(Sheet::City { chosen, .. }) = &mut self.sheet {
                        *chosen = keys;
                    }
                }
            }
        }
        acted
    }

    fn edit_draft(&mut self, field: Field) {
        let clamp = |value: f64, top: u64| (value.max(0.0) as u64).min(top);
        match (&mut self.sheet, field) {
            (Some(Sheet::Alarm(alarm)), Field::Hour(value)) => alarm.hour = clamp(value, 23) as i8,
            (Some(Sheet::Alarm(alarm)), Field::Minute(value)) => {
                alarm.minute = clamp(value, 59) as i8
            }
            (Some(Sheet::Alarm(alarm)), Field::Day(day)) => alarm.days = alarm.days.toggled(day),
            (Some(Sheet::Alarm(alarm)), Field::Label(text)) => alarm.label = text,
            (Some(Sheet::Alarm(alarm)), Field::Sound(index)) => {
                alarm.sound = Sound::ALL
                    .get(index)
                    .map_or(DEFAULT_SOUND, |sound| sound.keyword())
                    .to_owned()
            }
            (Some(Sheet::Alarm(alarm)), Field::Snooze(on)) => alarm.snooze = on,
            (Some(Sheet::Timer(draft)), Field::Hour(value)) => draft.hours = clamp(value, 23),
            (Some(Sheet::Timer(draft)), Field::Minute(value)) => draft.minutes = clamp(value, 59),
            (Some(Sheet::Timer(draft)), Field::Second(value)) => draft.seconds = clamp(value, 59),
            (Some(Sheet::Timer(draft)), Field::Label(text)) => draft.label = text,
            (Some(Sheet::Timer(draft)), Field::Sound(index)) => {
                draft.sound = Sound::ALL
                    .get(index)
                    .map_or(DEFAULT_SOUND, |sound| sound.keyword())
                    .to_owned()
            }
            _ => {}
        }
    }

    fn finish_sheet(&mut self, action: &str) {
        let Some(sheet) = self.sheet.take() else {
            return;
        };
        match (sheet, action) {
            (Sheet::Alarm(alarm), "save") => self.changes.push(Change::SaveAlarm(alarm)),
            (Sheet::Alarm(alarm), "delete") => self.changes.push(Change::DeleteAlarm(alarm.id)),
            (Sheet::Timer(draft), "start") => {
                let length =
                    Duration::from_secs(draft.hours * 3600 + draft.minutes * 60 + draft.seconds);
                if !length.is_zero() {
                    self.changes.push(Change::NewTimer(Timer {
                        id: 0,
                        label: draft.label.trim().to_owned(),
                        length,
                        sound: draft.sound,
                        state: TimerState::Idle,
                    }));
                }
            }
            (Sheet::City { chosen, .. }, "add") => {
                self.changes.extend(chosen.into_iter().map(Change::AddCity));
            }
            _ => {}
        }
    }

    fn search(&mut self, text: &str) {
        let Some(Sheet::City { list, .. }) = &self.sheet else {
            return;
        };
        let rows = city_rows(&self.cities, text);
        self.ui
            .update_widget::<Table>(*list, |table, _| table.set_rows(rows));
    }

    fn open_sheet(&mut self, alarm: Option<u64>) {
        if self.ui.has_sheet() {
            return;
        }
        let page = match alarm {
            Some(_) => Page::Alarms,
            None => self.page,
        };
        let sheet = match page {
            Page::Alarms => {
                let existing =
                    alarm.and_then(|id| self.clock.alarms.iter().find(|alarm| alarm.id == id));
                let draft = existing.cloned().unwrap_or_else(|| {
                    let next = Zoned::now().with_time_zone(self.zone.clone());
                    Alarm::new(0, next.hour(), next.minute())
                });
                Sheet::Alarm(draft)
            }
            Page::Timers => Sheet::Timer(TimerDraft {
                hours: 0,
                minutes: 5,
                seconds: 0,
                label: String::new(),
                sound: DEFAULT_SOUND.to_owned(),
            }),
            Page::World => {
                if self.cities.is_empty() {
                    self.cities = cities::all();
                }
                Sheet::City {
                    list: WidgetId::next(),
                    chosen: Vec::new(),
                }
            }
            Page::Stopwatch => return,
        };
        let alert = self.sheet_alert(&sheet);
        let sheet = match (sheet, &alert) {
            (Sheet::City { chosen, .. }, (_, Some(list))) => Sheet::City {
                list: *list,
                chosen,
            },
            (sheet, _) => sheet,
        };
        self.ui.present_sheet(alert.0);
        self.sheet = Some(sheet);
    }

    fn sheet_alert(&self, sheet: &Sheet) -> (Alert, Option<WidgetId>) {
        let send = sender(&self.intents);
        let choose = move |action: &str| send(Intent::Sheet(action.to_owned()));
        let field = |make: fn(f64) -> Field| {
            let send = sender(&self.intents);
            move |value: f64| send(Intent::Field(make(value)))
        };
        let two_digits = |value: f64| format!("{:02}", value as u64);
        let stepper = |label: &str, value: u64, top: f64, make: fn(f64) -> Field| {
            boxed(
                Stepper::new(label, value as f64, (0.0, top), 1.0, field(make)).format(two_digits),
            )
        };
        let label_field = |text: &str| {
            let send = sender(&self.intents);
            boxed(TextField::new(&L10N.tr("label"), text, move |text| {
                send(Intent::Field(Field::Label(text.to_owned())))
            }))
        };
        let sound_choice = |keyword: &str| {
            let send = sender(&self.intents);
            let titles: Vec<String> = Sound::ALL.into_iter().map(sound_title).collect();
            boxed(PopUpButton::new(
                &L10N.tr("sound"),
                &titles,
                sound_index(keyword),
                move |index| send(Intent::Field(Field::Sound(index))),
            ))
        };
        match sheet {
            Sheet::Alarm(alarm) => {
                let mut days: Vec<Box<dyn Widget>> = weekdays()
                    .map(|day| {
                        let send = sender(&self.intents);
                        let state = match alarm.days.has(day) {
                            true => CheckState::On,
                            false => CheckState::Off,
                        };
                        boxed(Checkbox::new(
                            &short_weekday(&self.formats, day),
                            state,
                            move |_| send(Intent::Field(Field::Day(day))),
                        ))
                    })
                    .collect();
                let later_days = days.split_off(4);
                let send = sender(&self.intents);
                let lines = vec![
                    FormLine::Row(
                        L10N.tr("time"),
                        boxed(Stack::new(
                            Axis::Horizontal,
                            6.0,
                            0.0,
                            vec![
                                stepper(&L10N.tr("hour"), alarm.hour as u64, 23.0, Field::Hour),
                                boxed(Label::new(":")),
                                stepper(
                                    &L10N.tr("minute"),
                                    alarm.minute as u64,
                                    59.0,
                                    Field::Minute,
                                ),
                            ],
                        )),
                    ),
                    FormLine::Wide(boxed(Stack::new(
                        Axis::Vertical,
                        6.0,
                        0.0,
                        vec![
                            boxed(Stack::new(Axis::Horizontal, 8.0, 0.0, days)),
                            boxed(Stack::new(Axis::Horizontal, 8.0, 0.0, later_days)),
                        ],
                    ))),
                    FormLine::Row(L10N.tr("label"), label_field(&alarm.label)),
                    FormLine::Row(L10N.tr("sound"), sound_choice(&alarm.sound)),
                    FormLine::Row(
                        L10N.tr("snooze"),
                        boxed(Toggle::new(&L10N.tr("snooze"), alarm.snooze, move |on| {
                            send(Intent::Field(Field::Snooze(on)))
                        })),
                    ),
                ];
                let title = match alarm.id {
                    0 => L10N.tr("new-alarm"),
                    _ => L10N.tr("edit-alarm"),
                };
                let mut alert = Alert::new(&title, "", choose).accessory(Form::new(&title, lines));
                if alarm.id != 0 {
                    alert = alert.other(&L10N.tr("delete"), "delete");
                }
                (
                    alert
                        .cancel(&L10N.tr("cancel"), "cancel")
                        .primary(&L10N.tr("save"), "save"),
                    None,
                )
            }
            Sheet::Timer(draft) => {
                let lines = vec![
                    FormLine::Wide(boxed(Stack::new(
                        Axis::Horizontal,
                        6.0,
                        0.0,
                        vec![
                            stepper(&L10N.tr("hour"), draft.hours, 23.0, Field::Hour),
                            boxed(Label::new(":")),
                            stepper(&L10N.tr("minute"), draft.minutes, 59.0, Field::Minute),
                            boxed(Label::new(":")),
                            stepper(&L10N.tr("second"), draft.seconds, 59.0, Field::Second),
                        ],
                    ))),
                    FormLine::Row(L10N.tr("label"), label_field(&draft.label)),
                    FormLine::Row(L10N.tr("sound"), sound_choice(&draft.sound)),
                ];
                let title = L10N.tr("new-timer");
                (
                    Alert::new(&title, "", choose)
                        .accessory(Form::new(&title, lines))
                        .cancel(&L10N.tr("cancel"), "cancel")
                        .primary(&L10N.tr("start"), "start"),
                    None,
                )
            }
            Sheet::City { .. } => {
                let send = sender(&self.intents);
                let search = SearchField::new(&L10N.tr("search-cities"), "", move |text| {
                    send(Intent::Search(text.to_owned()))
                });
                let send = sender(&self.intents);
                let mut list = Table::new(vec![
                    Column::new(&L10N.tr("city"), 160.0).flexible(),
                    Column::new(&L10N.tr("country"), 140.0),
                ])
                .on_selection(move |keys| send(Intent::Chosen(keys.to_vec())));
                list.set_rows(city_rows(&self.cities, ""));
                let id = list.id();
                let fields = Stack::new(
                    Axis::Vertical,
                    8.0,
                    0.0,
                    vec![boxed(search), boxed(Fixed::new(CITY_LIST_HEIGHT, list))],
                );
                (
                    Alert::new(&L10N.tr("add-city"), "", choose)
                        .accessory(fields)
                        .cancel(&L10N.tr("cancel"), "cancel")
                        .primary(&L10N.tr("add"), "add"),
                    Some(id),
                )
            }
        }
    }

    fn rebuild(&mut self) {
        self.live.clear();
        let body = match self.page {
            Page::Alarms => self.alarms_page(),
            Page::Timers => self.timers_page(),
            Page::Stopwatch => self.stopwatch_page(),
            Page::World => self.world_page(),
        };
        let (root, old) = (self.root, self.body);
        let mut replaced = None;
        if let Some(layout) = self.ui.widget_as::<FreeLayout>(root) {
            replaced = layout.replace(
                old,
                ScrollView::shared(boxed(body), Rc::clone(&self.scroll)),
            );
        }
        if let Some(body) = replaced {
            self.body = body;
        }
        self.refresh_live();
    }

    fn empty(&self, key: &str) -> Box<dyn Widget> {
        boxed(Stack::new(
            Axis::Vertical,
            0.0,
            PADDING * 2.0,
            vec![boxed(Label::new(&L10N.tr(key)).secondary())],
        ))
    }

    fn list(&self, lines: Vec<Box<dyn Widget>>) -> Stack {
        Stack::new(Axis::Vertical, 0.0, PADDING, lines)
    }

    fn alarms_page(&mut self) -> Stack {
        if self.clock.alarms.is_empty() {
            return self.list(vec![self.empty("no-alarms")]);
        }
        let mut alarms = self.clock.alarms.clone();
        alarms.sort_by_key(|alarm| (alarm.hour, alarm.minute));
        let lines = alarms
            .iter()
            .map(|alarm| {
                let at = MONDAY
                    .at(alarm.hour, alarm.minute, 0, 0)
                    .to_zoned(TimeZone::UTC);
                let time = at
                    .map(|at| clock_time(&self.formats, &at))
                    .unwrap_or_default();
                let repeat = repeat_text(&self.formats, alarm.days);
                let snoozed = alarm.snoozed.map(|until| {
                    L10N.tr_args(
                        "snoozed-until",
                        [(
                            "time",
                            clock_time(&self.formats, &until.to_zoned(self.zone.clone())).into(),
                        )],
                    )
                });
                let detail = [
                    Some(alarm_name(alarm)),
                    (!repeat.is_empty()).then_some(repeat),
                    snoozed,
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
                let (send, id) = (sender(&self.intents), alarm.id);
                let toggle = Toggle::new(&alarm_name(alarm), alarm.enabled, move |on| {
                    send(Intent::Change(Change::EnableAlarm(id, on)))
                });
                let send = sender(&self.intents);
                let edit = Button::new(&L10N.tr("edit"), move || send(Intent::Edit(id))).quiet();
                boxed(Line::new(
                    &format!("{time} {detail}"),
                    vec![
                        boxed(Digits::new(&time, BIG).light().muted(!alarm.enabled)),
                        boxed(Label::new(&detail).secondary()),
                    ],
                    vec![boxed(toggle), boxed(edit)],
                ))
            })
            .collect();
        self.list(lines)
    }

    fn timers_page(&mut self) -> Stack {
        let presets = PRESETS
            .iter()
            .map(|seconds| {
                let send = sender(&self.intents);
                let length = Duration::from_secs(*seconds);
                boxed(Button::new(&length_name(length), move || {
                    send(Intent::Change(Change::NewTimer(Timer {
                        id: 0,
                        label: String::new(),
                        length,
                        sound: DEFAULT_SOUND.to_owned(),
                        state: TimerState::Idle,
                    })))
                }))
            })
            .collect();
        let mut lines = vec![boxed(Stack::new(Axis::Horizontal, 6.0, INSET, presets))];
        if self.clock.timers.is_empty() {
            lines.push(self.empty("no-timers"));
        }
        let boot = boot_now();
        for timer in &self.clock.timers {
            let id = timer.id;
            let (title, running) = match timer.state {
                TimerState::Idle => (L10N.tr("start"), false),
                TimerState::Running { .. } => (L10N.tr("pause"), true),
                TimerState::Paused { .. } => (L10N.tr("resume"), false),
            };
            let send = sender(&self.intents);
            let start = Button::new(&title, move || send(Intent::Change(Change::StartPause(id))));
            let start = match running {
                true => start,
                false => start.primary(),
            };
            let send = sender(&self.intents);
            let second = match timer.state {
                TimerState::Idle => Button::new(&L10N.tr("delete"), move || {
                    send(Intent::Change(Change::DeleteTimer(id)))
                })
                .quiet(),
                _ => Button::new(&L10N.tr("cancel"), move || {
                    send(Intent::Change(Change::ResetTimer(id)))
                }),
            };
            let digits = Digits::new(&countdown(timer.left(boot)), BIG)
                .light()
                .muted(!running);
            self.live.push((digits.id(), Live::Timer(id)));
            lines.push(boxed(Line::new(
                &timer_name(timer),
                vec![
                    boxed(digits),
                    boxed(Label::new(&timer_name(timer)).secondary()),
                ],
                vec![boxed(start), boxed(second)],
            )));
        }
        self.list(lines)
    }

    fn stopwatch_page(&mut self) -> Stack {
        let stopwatch = &self.clock.stopwatch;
        let running = stopwatch.started.is_some();
        let elapsed = Digits::new(&lap_time(stopwatch.elapsed(boot_now())), HUGE).light();
        self.live.push((elapsed.id(), Live::Stopwatch));
        let send = sender(&self.intents);
        let toggle = match running {
            true => Button::new(&L10N.tr("stop"), move || {
                send(Intent::Change(Change::StopwatchToggle))
            }),
            false => Button::new(&L10N.tr("start"), move || {
                send(Intent::Change(Change::StopwatchToggle))
            })
            .primary(),
        };
        let send = sender(&self.intents);
        let mut lap = match running || stopwatch.banked.is_zero() {
            true => Button::new(&L10N.tr("lap"), move || {
                send(Intent::Change(Change::StopwatchLap))
            }),
            false => Button::new(&L10N.tr("reset"), move || {
                send(Intent::Change(Change::StopwatchReset))
            }),
        };
        lap.set_enabled(running || !stopwatch.banked.is_zero());
        let mut lines = vec![boxed(Stack::new(
            Axis::Vertical,
            8.0,
            INSET,
            vec![
                boxed(elapsed),
                boxed(Stack::new(
                    Axis::Horizontal,
                    8.0,
                    0.0,
                    vec![boxed(lap), boxed(toggle)],
                )),
            ],
        ))];
        let laps = &stopwatch.laps;
        if running || !laps.is_empty() {
            let current = Digits::new("", SMALL);
            self.live.push((current.id(), Live::Lap));
            lines.push(boxed(Line::new(
                &L10N.tr_args("lap-number", [("number", (laps.len() + 1).into())]),
                vec![boxed(Label::new(&L10N.tr_args(
                    "lap-number",
                    [("number", (laps.len() + 1).into())],
                )))],
                vec![boxed(current)],
            )));
        }
        let mut previous = Duration::ZERO;
        let mut finished = Vec::new();
        for (index, total) in laps.iter().enumerate() {
            let title = L10N.tr_args("lap-number", [("number", (index + 1).into())]);
            let time = lap_time(total.saturating_sub(previous));
            previous = *total;
            finished.push(boxed(Line::new(
                &format!("{title} {time}"),
                vec![boxed(Label::new(&title))],
                vec![boxed(Digits::new(&time, SMALL))],
            )));
        }
        lines.extend(finished.into_iter().rev());
        self.list(lines)
    }

    fn world_page(&mut self) -> Stack {
        if self.clock.cities.is_empty() {
            return self.list(vec![self.empty("no-cities")]);
        }
        let here = Zoned::now().with_time_zone(self.zone.clone());
        let lines = self
            .clock
            .cities
            .iter()
            .filter_map(|zone| {
                let city_zone = jiff::tz::db().get(zone).ok()?;
                let there = here.with_time_zone(city_zone.clone());
                let night =
                    sky(&there, place_of(&city_zone)).is_some_and(|(sky, _)| sky == Sky::Night);
                let detail = offset_text(&there, &here);
                let detail = match night {
                    true => format!("{detail} · {}", L10N.tr("night")),
                    false => detail,
                };
                let time = Digits::new(&clock_time(&self.formats, &there), BIG).light();
                self.live.push((time.id(), Live::City(zone.clone())));
                let (send, zone) = (sender(&self.intents), zone.clone());
                let name = city_name(&zone);
                let remove = Button::new(&L10N.tr("remove"), move || {
                    send(Intent::Change(Change::RemoveCity(zone.clone())))
                })
                .quiet();
                Some(boxed(Line::new(
                    &name,
                    vec![
                        boxed(Digits::new(&name, SMALL + 2.0)),
                        boxed(Label::new(&detail).secondary()),
                    ],
                    vec![boxed(remove), boxed(time)],
                )))
            })
            .collect();
        self.list(lines)
    }

    fn refresh_live(&mut self) -> bool {
        let boot = boot_now();
        let now = Timestamp::now();
        let stopwatch = &self.clock.stopwatch;
        let texts: Vec<(WidgetId, String)> =
            self.live
                .iter()
                .filter_map(|(id, live)| {
                    let text =
                        match live {
                            Live::Timer(timer) => self
                                .clock
                                .timers
                                .iter()
                                .find(|shown| shown.id == *timer)
                                .map(|timer| countdown(timer.left(boot)))?,
                            Live::Stopwatch => lap_time(stopwatch.elapsed(boot)),
                            Live::Lap => lap_time(stopwatch.elapsed(boot).saturating_sub(
                                stopwatch.laps.last().copied().unwrap_or_default(),
                            )),
                            Live::City(zone) => {
                                let zone = jiff::tz::db().get(zone).ok()?;
                                clock_time(&self.formats, &now.to_zoned(zone))
                            }
                        };
                    Some((*id, text))
                })
                .collect();
        let mut changed = false;
        for (id, text) in texts {
            self.ui
                .update_widget::<Digits>(id, |digits, _| changed |= digits.set_text(&text));
        }
        changed
    }

    fn ticking(&self) -> bool {
        self.visible && self.page == Page::Stopwatch && self.clock.stopwatch.started.is_some()
    }

    fn next_tick(&self) -> Option<Instant> {
        if !self.visible {
            return None;
        }
        let now = Instant::now();
        match self.page {
            Page::Timers => {
                let boot = boot_now();
                self.clock
                    .timers
                    .iter()
                    .filter(|timer| timer.ends().is_some())
                    .map(|timer| {
                        let left = timer.left(boot);
                        let fraction = Duration::from_nanos(u64::from(left.subsec_nanos()));
                        match fraction.is_zero() {
                            true => Duration::from_secs(1),
                            false => fraction,
                        }
                    })
                    .min()
                    .map(|wait| now + wait)
            }
            Page::World if !self.clock.cities.is_empty() => {
                let second = Timestamp::now().as_second().rem_euclid(60);
                Some(now + Duration::from_secs(u64::try_from(60 - second).unwrap_or(60)))
            }
            _ => None,
        }
    }

    fn header(&self, width: f32) -> (Rect, Rect) {
        let tabs_width = SEGMENT_WIDTH * Page::ALL.len() as f32;
        let leading = window_controls_end(true) + SIDE;
        let x = ((width - tabs_width) / 2.0).round().max(leading);
        let tabs = Rect::new(
            x,
            ((HEADER - SEGMENTED_HEIGHT) / 2.0).round(),
            tabs_width,
            SEGMENTED_HEIGHT,
        );
        let add = match self.can_add() {
            true => Rect::new(
                width - SIDE - BUTTON_HEIGHT,
                ((HEADER - BUTTON_HEIGHT) / 2.0).round(),
                BUTTON_HEIGHT,
                BUTTON_HEIGHT,
            ),
            false => Rect::default(),
        };
        (tabs, add)
    }
}

fn city_rows(cities: &[City], text: &str) -> Vec<Row> {
    cities::matching(cities, text)
        .map(|city| {
            Row::new(
                city.zone.clone(),
                vec![
                    bande_toolkit::ui::Cell::new(city.name.as_str()),
                    bande_toolkit::ui::Cell::new(city.country.as_str()).muted(),
                ],
            )
        })
        .collect()
}

impl View for ClockView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let (tabs, add) = self.header(width as f32);
        let body = Rect::new(0.0, HEADER, width as f32, (height as f32 - HEADER).max(0.0));
        let (tabs_id, add_id, body_id) = (self.tabs, self.add, self.body);
        if let Some(layout) = self.ui.widget_as::<FreeLayout>(self.root) {
            layout.set_frame(tabs_id, tabs);
            layout.set_frame(add_id, add);
            layout.set_frame(body_id, body);
        }
        window.set_header(Header {
            title_x: window_controls_end(true) + SIDE,
            items: [tabs, add]
                .into_iter()
                .filter(|rect| rect.width > 0.0)
                .collect(),
            tabs: Vec::new(),
        });
        self.refresh_live();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let shown = match &event {
            WindowEvent::ToplevelChanged(state) => {
                let visible = !state.suspended;
                let changed = visible != self.visible;
                self.visible = visible;
                changed
            }
            _ => false,
        };
        let handled = window
            .drive(&mut self.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        self.pump() || handled || shown
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
        self.rebuild();
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_tick().into_iter().chain(self.ui.deadline()).min()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        self.refresh_live();
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating() || self.ticking()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
