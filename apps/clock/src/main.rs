use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, WindowId};
use bande_app_menu::Entry;
use bande_clock::bus::{Bus, Summary, Waker};
use bande_clock::deadline::{Deadline, Timeline};
use bande_clock::model::{Clock, SNOOZE, TimerState, boot_now, state_file};
use bande_clock::notice::{
    Notice, Notices, OPEN, REPEAT, Ring, SNOOZE as SNOOZE_ACTION, Subject, sound_name,
};
use bande_clock::ui::{Change, ClockView, L10N, Page, alarm_name, clock_time, timer_name};
use bande_datetime::DateFormats;
use bande_inbox::{Inbox, Outbox};
use bande_toolkit::Appearance;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use zbus::blocking::Connection;

const APP_ID: &str = "org.bande.Clock";
const WINDOW_SIZE: (u32, u32) = (640, 560);
const SHOW_CLOCK: &str = "show-clock";
const FLAGS: [(&str, &str); 2] = [("--new-alarm", "new-alarm"), ("--new-timer", "new-timer")];
const RING_EVERY: Duration = Duration::from_secs(3);
const RING_FOR: Duration = Duration::from_secs(5 * 60);

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

struct Ringing {
    id: Option<u32>,
    subject: Subject,
    sound: String,
    until: Instant,
}

struct ClockApp {
    main: Option<WindowId>,
    parked: Option<ClockView>,
    clock: Clock,
    file: PathBuf,
    dirty: bool,
    zone: TimeZone,
    formats: DateFormats,
    wall: Deadline,
    boot: Deadline,
    zones: Inbox<String>,
    notices: Notices,
    bus: Option<Bus>,
    waker: Waker,
    ringing: Vec<Ringing>,
    next_ring: Option<Instant>,
}

fn follow_zone(zones: Outbox<String>) {
    let spawned = std::thread::Builder::new()
        .name("zone".to_owned())
        .spawn(move || {
            let followed = Connection::system().and_then(|system| {
                for zone in bande_datetime::timedate::zone_changes(&system)? {
                    if !zones.send(zone) {
                        break;
                    }
                }
                Ok(())
            });
            if let Err(error) = followed {
                tracing::debug!("Clock does not follow time zone changes: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("Clock does not follow time zone changes: {error}");
    }
}

fn entry(label: &str, action: &str) -> Entry {
    Entry::item(&L10N.tr(label), action)
}

fn show(
    cx: &mut Cx<ClockApp>,
    mut view: ClockView,
) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(
        Kind::Main,
        &L10N.tr("app-name"),
        WINDOW_SIZE,
        |appearance| {
            bande_app::View::set_appearance(&mut view, appearance);
            view
        },
    )
}

fn asked(arguments: &[String]) -> Option<&'static str> {
    FLAGS
        .iter()
        .find(|(flag, _)| arguments.iter().any(|argument| argument == flag))
        .map(|(_, command)| *command)
}

fn wall_seconds(ends: Duration, now: Timestamp, boot: Duration) -> i64 {
    let ahead = i64::try_from(ends.saturating_sub(boot).as_secs()).unwrap_or(i64::MAX);
    now.as_second().saturating_add(ahead)
}

impl ClockApp {
    fn with_view(&mut self, cx: &mut Cx<Self>, work: impl FnOnce(&mut ClockView)) {
        if let Some((_, view)) = self.main.and_then(|main| cx.window(main)) {
            return work(view);
        }
        if let Some(view) = &mut self.parked {
            work(view);
        }
    }

    fn raise(&mut self, cx: &mut Cx<Self>) {
        self.reopen(cx);
        if let Some((window, _)) = self.main.and_then(|main| cx.window(main))
            && let Err(error) = window
                .activation_token(APP_ID)
                .and_then(|token| window.activate(&token))
        {
            tracing::warn!("cannot raise Clock: {error}");
        }
    }

    fn apply(&mut self, change: Change) {
        let (now, boot) = (Timestamp::now(), boot_now());
        let clock = &mut self.clock;
        match change {
            Change::SaveAlarm(mut alarm) => {
                alarm.enabled = true;
                alarm.snoozed = None;
                if alarm.id == 0 {
                    alarm.id = clock.new_id();
                    clock.alarms.push(alarm);
                } else if let Some(saved) =
                    clock.alarms.iter_mut().find(|saved| saved.id == alarm.id)
                {
                    *saved = alarm;
                }
                clock.checked = Some(now);
            }
            Change::DeleteAlarm(id) => clock.alarms.retain(|alarm| alarm.id != id),
            Change::EnableAlarm(id, on) => {
                if let Some(alarm) = clock.alarms.iter_mut().find(|alarm| alarm.id == id) {
                    alarm.enabled = on;
                    alarm.snoozed = None;
                }
                clock.checked = Some(now);
            }
            Change::NewTimer(mut timer) => {
                timer.id = clock.new_id();
                timer.start(boot);
                clock.timers.insert(0, timer);
            }
            Change::StartPause(id) => {
                if let Some(timer) = clock.timers.iter_mut().find(|timer| timer.id == id) {
                    match timer.state {
                        TimerState::Running { .. } => timer.pause(boot),
                        _ => timer.start(boot),
                    }
                }
            }
            Change::ResetTimer(id) => {
                if let Some(timer) = clock.timers.iter_mut().find(|timer| timer.id == id) {
                    timer.state = TimerState::Idle;
                }
            }
            Change::DeleteTimer(id) => clock.timers.retain(|timer| timer.id != id),
            Change::StopwatchToggle => clock.stopwatch.toggle(boot),
            Change::StopwatchLap => clock.stopwatch.lap(boot),
            Change::StopwatchReset => clock.stopwatch.reset(),
            Change::AddCity(zone) => {
                if !clock.cities.contains(&zone) {
                    clock.cities.push(zone);
                }
            }
            Change::RemoveCity(zone) => clock.cities.retain(|city| *city != zone),
        }
        self.dirty = true;
    }

    fn check(&mut self) {
        let (now, boot) = (Timestamp::now(), boot_now());
        let alarms = self.clock.take_due_alarms(now, &self.zone);
        let timers = self.clock.take_due_timers(boot);
        self.dirty |= !alarms.is_empty() || !timers.is_empty() || self.clock.keeps_running();
        let time = clock_time(&self.formats, &now.to_zoned(self.zone.clone()));
        let rings = alarms
            .into_iter()
            .map(|alarm| Ring {
                subject: Subject::Alarm(alarm.id),
                title: alarm_name(&alarm),
                body: time.clone(),
                sound: alarm.sound.clone(),
                snooze: alarm.snooze,
            })
            .chain(timers.into_iter().map(|timer| Ring {
                subject: Subject::Timer(timer.id),
                title: L10N.tr("timer-done"),
                body: timer_name(&timer),
                sound: timer.sound.clone(),
                snooze: false,
            }));
        for ring in rings {
            self.ringing
                .retain(|ringing| ringing.subject != ring.subject);
            self.ringing.push(Ringing {
                id: None,
                subject: ring.subject,
                sound: ring.sound.clone(),
                until: Instant::now() + RING_FOR,
            });
            self.next_ring = Some(Instant::now() + RING_EVERY);
            self.notices.ring(ring);
        }
    }

    fn summary(&self) -> Summary {
        let (now, boot) = (Timestamp::now(), boot_now());
        let mut timers: Vec<(Duration, String)> = self
            .clock
            .timers
            .iter()
            .filter_map(|timer| Some((timer.ends()?, timer_name(timer))))
            .collect();
        timers.sort();
        Summary {
            timers: timers
                .into_iter()
                .map(|(ends, name)| (name, wall_seconds(ends, now, boot)))
                .collect(),
            next_alarm: self
                .clock
                .next_alarm(now, &self.zone)
                .map(|(at, alarm)| (alarm_name(alarm), at.as_second()))
                .unwrap_or_default(),
        }
    }

    fn settle(&mut self, cx: &mut Cx<Self>) {
        if std::mem::take(&mut self.dirty)
            && let Err(error) = self.clock.save(&self.file)
        {
            tracing::warn!("cannot save {}: {error}", self.file.display());
        }
        let (now, boot) = (Timestamp::now(), boot_now());
        self.wall
            .set_wall(self.clock.next_alarm(now, &self.zone).map(|(at, _)| at));
        self.boot.set_boot(self.clock.next_timer());
        self.waker
            .wake_at(self.clock.wake_at(now, boot, &self.zone));
        let summary = self.summary();
        if let Some(bus) = &mut self.bus {
            bus.publish(summary);
        }
        let clock = self.clock.clone();
        if let Some((window, view)) = self.main.and_then(|main| cx.window(main)) {
            view.show(&clock);
            window.request_redraw();
        } else if let Some(view) = &mut self.parked {
            view.show(&clock);
        }
        cx.refresh_menus();
    }

    fn take_changes(&mut self, cx: &mut Cx<Self>) {
        let mut changes = Vec::new();
        self.with_view(cx, |view| changes = view.take_changes());
        for change in changes {
            self.apply(change);
        }
    }

    fn notice(&mut self, cx: &mut Cx<Self>, notice: Notice) {
        match notice {
            Notice::Shown(id, subject) => {
                match self
                    .ringing
                    .iter_mut()
                    .find(|ringing| ringing.subject == subject && ringing.id.is_none())
                {
                    Some(ringing) => ringing.id = Some(id),
                    None => self.notices.withdraw(id),
                }
            }
            Notice::Closed(id) => self.ringing.retain(|ringing| ringing.id != Some(id)),
            Notice::Invoked(id, action) => {
                let Some(at) = self
                    .ringing
                    .iter()
                    .position(|ringing| ringing.id == Some(id))
                else {
                    return;
                };
                let subject = self.ringing.remove(at).subject;
                match (subject, action.as_str()) {
                    (Subject::Alarm(alarm), SNOOZE_ACTION) => {
                        let until = Timestamp::now().checked_add(SNOOZE).ok();
                        if let Some(alarm) =
                            self.clock.alarms.iter_mut().find(|shown| shown.id == alarm)
                        {
                            alarm.snoozed = until;
                            self.dirty = true;
                        }
                    }
                    (Subject::Timer(timer), REPEAT) => self.apply(Change::StartPause(timer)),
                    (subject, OPEN) => {
                        self.raise(cx);
                        let page = match subject {
                            Subject::Alarm(_) => Page::Alarms,
                            Subject::Timer(_) => Page::Timers,
                        };
                        self.with_view(cx, |view| view.set_page(page));
                    }
                    _ => {}
                }
            }
        }
        if self.ringing.is_empty() {
            self.next_ring = None;
        }
    }
}

impl Program for ClockApp {
    type View = ClockView;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let file = state_file();
        let clock = Clock::load(&file);
        let zones = Inbox::new()?;
        follow_zone(zones.outbox());
        let bus = cx
            .session()
            .map(Bus::serve)
            .transpose()
            .inspect_err(|error| tracing::warn!("cannot serve org.bande.Clock: {error}"))
            .ok()
            .flatten();
        let zone = bande_datetime::local_zone();
        let formats = DateFormats::from_env();
        let parked = ClockView::new(Appearance::default(), &clock, zone.clone(), formats.clone());
        let mut app = Self {
            main: None,
            parked: Some(parked),
            clock,
            file,
            dirty: false,
            zone,
            formats,
            wall: Deadline::new(Timeline::Wall)?,
            boot: Deadline::new(Timeline::Boot)?,
            zones,
            notices: Notices::start()?,
            bus,
            waker: Waker::start()?,
            ringing: Vec::new(),
            next_ring: None,
        };
        tracing::debug!(
            "alarms wake the computer: {}, timers: {}",
            app.wall.wakes_system(),
            app.boot.wakes_system()
        );
        app.check();
        if !cx.launched_in_background() {
            app.reopen(cx);
        }
        app.settle(cx);
        if let Some(command) = asked(&arguments) {
            app.command(cx, None, command);
        }
        Ok(app)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let page = self
            .main
            .and_then(|main| cx.view(main))
            .or(self.parked.as_ref())
            .map_or(Page::Alarms, ClockView::page);
        let pages = Page::ALL.iter().enumerate().map(|(index, shown)| {
            Entry::item(&shown.title(), &shown.command())
                .shortcut(&format!("Ctrl+{}", index + 1))
                .radio(*shown == page)
        });
        let stopwatch = &self.clock.stopwatch;
        let running = stopwatch.started.is_some();
        let toggle = match running {
            true => "stop-stopwatch",
            false => "start-stopwatch",
        };
        let lap = match running || stopwatch.banked.is_zero() {
            true => "lap",
            false => "reset",
        };
        let view_menu = pages
            .chain([
                Entry::separator(),
                entry(toggle, "stopwatch-toggle").shortcut("Ctrl+T"),
                entry(lap, "stopwatch-lap")
                    .shortcut("Ctrl+L")
                    .enabled(running || !stopwatch.banked.is_zero()),
            ])
            .collect();
        Menus {
            menus: vec![
                Entry::submenu(
                    &L10N.tr("menu-file"),
                    vec![
                        entry("new-alarm-menu", "new-alarm").shortcut("Ctrl+N"),
                        entry("new-timer-menu", "new-timer").shortcut("Shift+Ctrl+N"),
                        entry("add-city-menu", "add-city"),
                    ],
                ),
                Entry::submenu(&L10N.tr("menu-view"), view_menu),
            ],
            window: vec![entry("app-name", SHOW_CLOCK)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        if command == SHOW_CLOCK {
            self.raise(cx);
            return self.settle(cx);
        }
        let (page, command) = match command {
            "new-alarm" => (Some(Page::Alarms), "add"),
            "new-timer" => (Some(Page::Timers), "add"),
            "add-city" => (Some(Page::World), "add"),
            other => (None, other),
        };
        if page.is_some() || command.starts_with("show-") {
            self.reopen(cx);
        }
        self.with_view(cx, |view| {
            if let Some(page) = page {
                view.set_page(page);
            }
            view.command(command);
        });
        self.take_changes(cx);
        self.settle(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        match asked(&arguments) {
            Some(command) => self.command(cx, None, command),
            None => self.raise(cx),
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.main.is_some() {
            return;
        }
        let Some(view) = self.parked.take() else {
            return;
        };
        match show(cx, view) {
            Ok(main) => self.main = Some(main),
            Err(error) => tracing::warn!("cannot open Clock: {error}"),
        }
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, view: ClockView) {
        if self.main == Some(window) {
            self.main = None;
            self.parked = Some(view);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.take_changes(cx);
        self.settle(cx);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![
            self.wall.fd(),
            self.boot.fd(),
            self.zones.fd(),
            self.notices.fd(),
        ]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        match index {
            0 => {
                self.wall.take();
                self.check();
            }
            1 => {
                self.boot.take();
                self.check();
            }
            2 => {
                if let Some(zone) = self.zones.drain().pop() {
                    self.zone =
                        TimeZone::get(&zone).unwrap_or_else(|_| bande_datetime::local_zone());
                    let zone = self.zone.clone();
                    self.with_view(cx, |view| view.set_zone(zone));
                }
            }
            _ => {
                for notice in self.notices.take() {
                    self.notice(cx, notice);
                }
            }
        }
        self.settle(cx);
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_ring
    }

    fn tick(&mut self, cx: &mut Cx<Self>, now: Instant) {
        self.ringing.retain(|ringing| ringing.until > now);
        match (self.ringing.last(), cx.session()) {
            (Some(ringing), Some(session)) => {
                bande_sound::play(session, &sound_name(&ringing.sound));
                self.next_ring = Some(now + RING_EVERY);
            }
            _ => self.next_ring = None,
        }
    }

    fn runs_in_background() -> bool {
        Clock::load(&state_file()).keeps_running()
    }
}

fn main() -> ExitCode {
    bande_app::run::<ClockApp>(&INFO)
}
