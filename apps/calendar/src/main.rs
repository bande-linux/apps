use std::collections::{HashMap, HashSet};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_calendar::alerts::{Due, due_between};
use bande_calendar::caldav::Discovered;
use bande_calendar::ical::{CalendarObject, read_file};
use bande_calendar::store::{Account, AccountKind, CalendarInfo, Library, new_uid};
use bande_calendar::sync::{apply, subscription_outcome};
use bande_calendar::ui::painter::{color_text, parse_color};
use bande_calendar::ui::{CalendarView, L10N, Mode, Request, SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
use bande_calendar::worker::{Job, Reply, Worker};
use bande_datetime::DateFormats;
use bande_design::{Accent, Scheme};
use bande_toolkit::{
    Appearance, FrameRegions, MenuPlacement, Rect, Scene, TextSystem, Window, WindowEvent,
};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, Zoned};
use zbus::zvariant::Value;

const APP_ID: &str = "org.bande.Calendar";
const PROGRAM: &str = "bande-calendar";
const WINDOW_SIZE: (u32, u32) = (1120, 740);
const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
const SUBSCRIPTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
const PUSH_DELAY: Duration = Duration::from_secs(2);
const ALERT_HORIZON: SignedDuration = SignedDuration::from_hours(1);
const NOTIFICATION_TIMEOUT: i32 = -1;

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

struct CalendarApp {
    main: WindowId,
    worker: Worker,
    zone: TimeZone,
    formats: DateFormats,
    next_sync: Instant,
    next_subscriptions: Instant,
    next_clock: Instant,
    next_alerts: Instant,
    alert_at: Timestamp,
    alerts_checked: Timestamp,
    fired: HashSet<String>,
    syncing: bool,
    sync_again: bool,
    title: String,
    errors: Vec<String>,
}

fn until_next_minute() -> Duration {
    let into_minute = Timestamp::now().as_millisecond().rem_euclid(60_000);
    Duration::from_millis(u64::try_from(60_000 - into_minute).unwrap_or(60_000))
}

fn data_dir() -> PathBuf {
    bande_xdg::data_home()
        .unwrap_or_else(std::env::temp_dir)
        .join(PROGRAM)
}

fn entry(label: &str, action: &str) -> Entry {
    Entry::item(&L10N.tr(label), action)
}

fn app_menus(mode: Mode) -> Vec<Entry> {
    let mut view: Vec<Entry> = Mode::ALL
        .iter()
        .enumerate()
        .map(|(index, shown)| {
            let label = match shown {
                Mode::Day => "day",
                Mode::Week => "week",
                Mode::Month => "month",
                Mode::Year => "year",
            };
            entry(label, &format!("mode-{index}"))
                .shortcut(&format!("Ctrl+{}", index + 1))
                .radio(*shown == mode)
        })
        .collect();
    view.extend([
        Entry::separator(),
        entry("go-to-today", "today").shortcut("Ctrl+T"),
        entry("previous-period", "previous"),
        entry("next-period", "next"),
        Entry::separator(),
        entry("find", "find").shortcut("Ctrl+F"),
    ]);
    vec![
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                entry("new-event", "new-event").shortcut("Ctrl+N"),
                Entry::separator(),
                entry("new-calendar", "new-calendar").shortcut("Alt+Ctrl+N"),
                entry("add-account", "add-account"),
                entry("refresh", "refresh").shortcut("Ctrl+R"),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(&L10N.tr("menu-view"), view),
    ]
}

fn clock(formats: &DateFormats, at: &Zoned) -> String {
    let (time, period) = formats.time(at);
    period.map_or(time.clone(), |period| format!("{time} {period}"))
}

fn notify(due: &Due, zone: &TimeZone, formats: &DateFormats) {
    let starts = due.starts.to_zoned(zone.clone());
    let when = match due.all_day {
        true => formats.date(&starts, false),
        false => formats.clock(&starts),
    };
    let body = match due.location.is_empty() {
        true => when,
        false => format!("{when}\n{}", due.location),
    };
    let title = due.title.clone();
    let spawned = std::thread::Builder::new()
        .name("alert".to_owned())
        .spawn(move || {
            let sent = zbus::blocking::Connection::session().and_then(|bus| {
                let hints: HashMap<&str, Value> = HashMap::from([
                    ("desktop-entry", Value::from(APP_ID)),
                    ("category", Value::from("x-calendar.alert")),
                ]);
                bus.call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &(
                        L10N.tr("app-name"),
                        0u32,
                        APP_ID,
                        title,
                        body,
                        Vec::<&str>::new(),
                        hints,
                        NOTIFICATION_TIMEOUT,
                    ),
                )
            });
            if let Err(error) = sent {
                tracing::warn!("cannot show an alert: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot show an alert: {error}");
    }
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or(url)
}

impl CalendarApp {
    fn sync(&mut self, view: &mut CalendarView, subscriptions: bool) {
        self.next_sync = Instant::now() + SYNC_INTERVAL;
        if subscriptions {
            self.next_subscriptions = Instant::now() + SUBSCRIPTION_INTERVAL;
        }
        if self.syncing {
            self.sync_again = true;
            return;
        }
        let library = view.library();
        let calendars: Vec<_> = library
            .calendars
            .iter()
            .filter_map(|calendar| {
                let account = library.account(&calendar.info.account)?;
                match &account.kind {
                    AccountKind::Local => None,
                    AccountKind::Subscription { .. } if !subscriptions => None,
                    kind => Some((kind.clone(), account.id.clone(), calendar.clone())),
                }
            })
            .collect();
        if calendars.is_empty() {
            return;
        }
        self.syncing = true;
        self.errors.clear();
        view.set_status(Some(L10N.tr("syncing")));
        self.worker.send(Job::Sync(calendars));
    }

    fn add_discovered(
        &mut self,
        view: &mut CalendarView,
        id: String,
        server: String,
        user: String,
        found: Discovered,
    ) {
        let name = match user.contains('@') {
            true => user.clone(),
            false => format!("{user}@{}", host_of(&found.principal)),
        };
        let calendars = found
            .calendars
            .iter()
            .enumerate()
            .map(|(index, remote)| CalendarInfo {
                id: new_uid(),
                account: id.clone(),
                url: Some(remote.url.clone()),
                name: remote.name.clone(),
                color: remote
                    .color
                    .as_deref()
                    .and_then(parse_color)
                    .map(color_text)
                    .unwrap_or_else(|| {
                        color_text(Accent::ALL[index % Accent::ALL.len()].color(Scheme::Light))
                    }),
                read_only: remote.read_only,
                visible: true,
                ctag: None,
                sync_token: None,
            })
            .collect();
        let account = Account {
            id,
            name,
            kind: AccountKind::CalDav {
                server,
                user,
                home: found.home,
            },
        };
        view.library_mut().add_account(account, calendars);
        view.account_added();
        self.sync(view, false);
    }

    fn add_subscription(&mut self, view: &mut CalendarView, url: String, data: &str) {
        let Ok(objects) = CalendarObject::parse(data) else {
            view.account_failed(&L10N.tr_args("not-a-calendar", [("url", url.into())]));
            return;
        };
        let header = |name: &str| {
            objects
                .first()
                .and_then(|object| object.calendar.value(name))
                .map(str::to_owned)
        };
        let name = header("X-WR-CALNAME").unwrap_or_else(|| host_of(&url).to_owned());
        let color = header("X-APPLE-CALENDAR-COLOR")
            .as_deref()
            .and_then(parse_color)
            .map(color_text)
            .unwrap_or_else(|| color_text(Accent::Plum.color(Scheme::Light)));
        let account = new_uid();
        let info = CalendarInfo {
            id: new_uid(),
            account: account.clone(),
            url: None,
            name: name.clone(),
            color,
            read_only: true,
            visible: true,
            ctag: None,
            sync_token: None,
        };
        let outcome = subscription_outcome(&info, data);
        view.library_mut().add_account(
            Account {
                id: account,
                name,
                kind: AccountKind::Subscription { url },
            },
            vec![info],
        );
        if let Some(outcome) = outcome {
            apply(view.library_mut(), outcome, true);
        }
        view.account_added();
    }

    fn calendar_name(view: &CalendarView, id: &str) -> String {
        let library = view.library();
        library
            .calendar_index(id)
            .map(|index| library.calendars[index].info.name.clone())
            .unwrap_or_default()
    }

    fn replies(&mut self, view: &mut CalendarView) -> bool {
        let replies = self.worker.take();
        let changed = !replies.is_empty();
        for reply in replies {
            match reply {
                Reply::Discovered {
                    id,
                    server,
                    user,
                    found,
                } => self.add_discovered(view, id, server, user, found),
                Reply::DiscoveryFailed(error) => view.account_failed(&error),
                Reply::Subscribed { url, data } => self.add_subscription(view, url, &data),
                Reply::Synced(outcome) => {
                    for title in apply(view.library_mut(), outcome, false) {
                        self.errors
                            .push(L10N.tr_args("conflict", [("title", title.into())]));
                    }
                }
                Reply::Refreshed { calendar, data } => {
                    let library = view.library_mut();
                    let outcome = library.calendar_index(&calendar).and_then(|index| {
                        subscription_outcome(&library.calendars[index].info, &data)
                    });
                    match outcome {
                        Some(outcome) => {
                            apply(library, outcome, true);
                        }
                        None => tracing::warn!("{calendar} did not answer with a calendar"),
                    }
                }
                Reply::Failed { calendar, error } => {
                    let name = Self::calendar_name(view, &calendar);
                    tracing::warn!("cannot sync {name}: {error}");
                    self.errors.push(L10N.tr_args(
                        "sync-error",
                        [("name", name.into()), ("error", error.into())],
                    ));
                }
                Reply::Made {
                    account,
                    url,
                    name,
                    color,
                } => {
                    view.library_mut().add_calendar(CalendarInfo {
                        id: new_uid(),
                        account,
                        url: Some(url),
                        name,
                        color: color.get(..7).unwrap_or(&color).to_owned(),
                        read_only: false,
                        visible: true,
                        ctag: None,
                        sync_token: None,
                    });
                }
                Reply::ChangeFailed(error) => {
                    tracing::warn!("cannot change a calendar: {error}");
                    view.set_status(Some(error));
                }
                Reply::Finished => {
                    self.syncing = false;
                    let now = Zoned::now().with_time_zone(self.zone.clone());
                    let status = self.errors.first().cloned().unwrap_or_else(|| {
                        L10N.tr_args("synced-at", [("time", clock(&self.formats, &now).into())])
                    });
                    view.set_status(Some(status));
                    if std::mem::take(&mut self.sync_again) {
                        self.next_sync = Instant::now() + PUSH_DELAY;
                    }
                }
            }
        }
        if changed {
            view.refresh_search();
            self.next_alerts = Instant::now();
        }
        changed
    }

    fn serve(&mut self, window: &mut Window, view: &mut CalendarView) {
        for request in view.take_requests() {
            match request {
                Request::Menu(menu, at) => {
                    if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
                Request::Clipboard(request) => {
                    if let Err(error) = window.serve_clipboard(request) {
                        tracing::warn!("cannot use the clipboard: {error}");
                    }
                }
                Request::AddAccount {
                    server,
                    user,
                    password,
                } => self.worker.send(Job::Discover {
                    id: new_uid(),
                    server,
                    user,
                    password,
                }),
                Request::Subscribe(url) => self.worker.send(Job::Subscribe { url }),
                Request::RemoveAccount(id) => {
                    view.library_mut().remove_account(&id);
                    self.worker.send(Job::Forget(id));
                }
                Request::Changed => {
                    self.next_sync = self.next_sync.min(Instant::now() + PUSH_DELAY);
                    self.next_alerts = Instant::now();
                    view.refresh_search();
                }
                Request::Refresh => self.sync(view, true),
                Request::Collection {
                    account,
                    url,
                    change,
                } => {
                    let user = match view.library().account(&account).map(|known| &known.kind) {
                        Some(AccountKind::CalDav { user, .. }) => user.clone(),
                        _ => continue,
                    };
                    self.worker.send(Job::Collection {
                        account,
                        user,
                        url,
                        change,
                    });
                }
            }
        }
        let title = view.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
    }

    fn check_alerts(&mut self, view: &CalendarView) {
        let now = Timestamp::now();
        let until = now
            .saturating_add(SignedDuration::from_secs(1))
            .unwrap_or(now);
        for due in due_between(
            view.library(),
            self.alerts_checked.min(now),
            until,
            &self.zone,
        ) {
            if self.fired.insert(due.key.clone()) {
                notify(&due, &self.zone, &self.formats);
            }
        }
        self.alerts_checked = until;
        let horizon = now.saturating_add(ALERT_HORIZON).unwrap_or(now);
        let next = due_between(view.library(), until, horizon, &self.zone)
            .first()
            .map_or(horizon, |due| due.at);
        let wait = now.duration_until(next).max(SignedDuration::from_secs(1));
        self.alert_at = next;
        self.next_alerts =
            Instant::now() + Duration::try_from(wait).unwrap_or(Duration::from_secs(60));
    }
}

struct Pane(CalendarView);

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        self.0.set_full_screen(full_screen);
        window.set_frame_regions(FrameRegions {
            sidebar: SIDEBAR_WIDTH,
            toolbar: match full_screen {
                true => TOOLBAR_HEIGHT,
                false => 0.0,
            },
        });
        let (width, height) = window.size();
        let scene = self.0.scene(width, height);
        window.set_header(self.0.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match event {
            WindowEvent::Input(input) => {
                let changed = self.0.handle(&input);
                window.set_cursor(self.0.cursor());
                changed
            }
            WindowEvent::Accessibility(request) => self.0.perform(&request),
            WindowEvent::MenuAction(action) => self.0.menu_action(&action),
            WindowEvent::MenuClosed => self.0.menu_closed(),
            WindowEvent::Pasted(text) => self.0.pasted(&text),
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.set_focused(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.0.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.0.animating()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.0.text_input_area()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.0.accessibility())
    }
}

impl CalendarApp {
    fn settle(&mut self, cx: &mut Cx<Self>) {
        if let Some((window, Pane(view))) = cx.window(self.main) {
            self.serve(window, view);
            window.request_redraw();
        }
        cx.refresh_menus();
    }
}

impl Program for CalendarApp {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let zone = TimeZone::try_system().unwrap_or(TimeZone::UTC);
        let library = Library::load(data_dir());
        let formats = DateFormats::from_env();
        let now = Zoned::now().with_time_zone(zone.clone());
        let mut view = CalendarView::new(
            library,
            Appearance::default().palette(),
            TextSystem::new(),
            formats.clone(),
            zone.clone(),
            now,
        );
        let title = view.title();
        let directory = std::env::current_dir().unwrap_or_default();
        open_arguments(&mut view, &arguments, &directory);
        let worker = Worker::start()?;
        let main = cx.open(Kind::Main, &title, WINDOW_SIZE, |appearance| {
            view.set_appearance(appearance);
            Pane(view)
        })?;
        let started = Instant::now();
        Ok(Self {
            main,
            worker,
            zone,
            formats,
            next_sync: started,
            next_subscriptions: started,
            next_clock: started + until_next_minute(),
            next_alerts: started,
            alert_at: Timestamp::now(),
            alerts_checked: Timestamp::now(),
            fired: HashSet::new(),
            syncing: false,
            sync_again: false,
            title,
            errors: Vec::new(),
        })
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        Menus {
            menus: cx
                .view(self.main)
                .map(|Pane(view)| app_menus(view.mode()))
                .unwrap_or_default(),
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: WindowId, command: &str) {
        if let Some((_, Pane(view))) = cx.window(self.main) {
            view.menu_action(command);
        }
        self.settle(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        if let Some((_, Pane(view))) = cx.window(self.main) {
            open_arguments(view, &arguments, directory);
        }
        self.settle(cx);
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.settle(cx);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.worker.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        if let Some((_, Pane(view))) = cx.window(self.main) {
            self.replies(view);
        }
        self.settle(cx);
    }

    fn deadline(&self) -> Option<Instant> {
        [
            self.next_sync,
            self.next_subscriptions,
            self.next_clock,
            self.next_alerts,
        ]
        .into_iter()
        .min()
    }

    fn tick(&mut self, cx: &mut Cx<Self>, now: Instant) {
        let Some((_, Pane(view))) = cx.window(self.main) else {
            return;
        };
        if now >= self.next_clock {
            view.set_now(Zoned::now().with_time_zone(self.zone.clone()));
            self.next_clock = now + until_next_minute();
        }
        if now >= self.next_alerts || Timestamp::now() >= self.alert_at {
            self.check_alerts(view);
        }
        if now >= self.next_subscriptions {
            self.sync(view, true);
        } else if now >= self.next_sync {
            self.sync(view, false);
        }
        self.settle(cx);
    }
}

fn open_arguments(view: &mut CalendarView, arguments: &[String], directory: &Path) {
    for argument in arguments {
        let lower = argument.to_ascii_lowercase();
        if lower.starts_with("webcal://") || lower.starts_with("webcals://") {
            view.offer_subscription(argument);
            continue;
        }
        let path =
            bande_xdg::path_from_file_uri(argument).unwrap_or_else(|| directory.join(argument));
        match read_file(&path).map(|text| CalendarObject::parse(&text)) {
            Ok(Ok(objects)) if !objects.is_empty() => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(first) = objects.first().and_then(|object| object.events.first()) {
                    view.go_to(first.start.date());
                }
                view.offer_import(objects, &name);
            }
            Ok(Ok(_)) => tracing::warn!("{} has no events", path.display()),
            Ok(Err(error)) => tracing::warn!("cannot read {}: {error}", path.display()),
            Err(error) => tracing::warn!("cannot open {}: {error}", path.display()),
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<CalendarApp>(&INFO)
}
