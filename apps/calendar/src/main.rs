use std::collections::{HashMap, HashSet};
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_calendar::alerts::{Due, due_between};
use bande_calendar::caldav::Discovered;
use bande_calendar::ical::CalendarObject;
use bande_calendar::store::{Account, AccountKind, CalendarInfo, Library, new_uid};
use bande_calendar::sync::{apply, subscription_outcome};
use bande_calendar::ui::painter::{color_text, parse_color};
use bande_calendar::ui::{CalendarView, L10N, Mode, Request, SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
use bande_calendar::worker::{Job, Reply, Worker};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, Zoned};
use unbound_app_menu::{AppMenu, Entry, Standard, app_submenu, close_window, window_submenu};
use unbound_datetime::DateFormats;
use unbound_design::{Accent, Scheme};
use unbound_toolkit::{
    App, Appearance, FrameRegions, MenuPlacement, Rect, Role, Scene, TextSystem, Window,
    WindowError, WindowEvent,
};
use zbus::zvariant::Value;

const APP_ID: &str = "org.bande.Calendar";
const PROGRAM: &str = "bande-calendar";
const WINDOW_SIZE: (u32, u32) = (1120, 740);
const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
const SUBSCRIPTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
const PUSH_DELAY: Duration = Duration::from_secs(2);
const CLOCK_INTERVAL: Duration = Duration::from_secs(30);
const ALERT_HORIZON: SignedDuration = SignedDuration::from_hours(1);
const NOTIFICATION_TIMEOUT: i32 = -1;

struct CalendarApp {
    view: CalendarView,
    worker: Worker,
    app_menu: Option<AppMenu>,
    zone: TimeZone,
    formats: DateFormats,
    next_sync: Instant,
    next_subscriptions: Instant,
    next_clock: Instant,
    next_alerts: Instant,
    alerts_checked: Timestamp,
    fired: HashSet<String>,
    syncing: bool,
    sync_again: bool,
    title: String,
    published_mode: Mode,
    errors: Vec<String>,
}

fn data_dir() -> PathBuf {
    unbound_xdg::data_home()
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
                .shortcut(&format!("Super+{}", index + 1))
                .radio(*shown == mode)
        })
        .collect();
    view.extend([
        Entry::separator(),
        entry("go-to-today", "today").shortcut("Super+T"),
        entry("previous-period", "previous"),
        entry("next-period", "next"),
        Entry::separator(),
        entry("find", "find").shortcut("Super+F"),
    ]);
    vec![
        app_submenu(&L10N.tr("app-name"), Vec::new()),
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                entry("new-event", "new-event").shortcut("Super+N"),
                Entry::separator(),
                entry("add-account", "add-account"),
                entry("refresh", "refresh").shortcut("Super+R"),
                Entry::separator(),
                close_window(),
            ],
        ),
        Entry::submenu(&L10N.tr("menu-view"), view),
        window_submenu(Vec::new()),
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
    fn sync(&mut self, subscriptions: bool) {
        self.next_sync = Instant::now() + SYNC_INTERVAL;
        if subscriptions {
            self.next_subscriptions = Instant::now() + SUBSCRIPTION_INTERVAL;
        }
        if self.syncing {
            self.sync_again = true;
            return;
        }
        let library = self.view.library();
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
        self.view.set_status(Some(L10N.tr("syncing")));
        self.worker.send(Job::Sync(calendars));
    }

    fn add_discovered(&mut self, id: String, server: String, user: String, found: Discovered) {
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
        self.view.library_mut().add_account(account, calendars);
        self.view.account_added();
        self.sync(false);
    }

    fn add_subscription(&mut self, url: String, data: &str) {
        let objects = CalendarObject::parse(data).unwrap_or_default();
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
        self.view.library_mut().add_account(
            Account {
                id: account,
                name,
                kind: AccountKind::Subscription { url },
            },
            vec![info],
        );
        apply(self.view.library_mut(), outcome, true);
        self.view.account_added();
    }

    fn calendar_name(&self, id: &str) -> String {
        let library = self.view.library();
        library
            .calendar_index(id)
            .map(|index| library.calendars[index].info.name.clone())
            .unwrap_or_default()
    }

    fn replies(&mut self) -> bool {
        let replies = self.worker.take();
        let changed = !replies.is_empty();
        for reply in replies {
            match reply {
                Reply::Discovered {
                    id,
                    server,
                    user,
                    found,
                } => self.add_discovered(id, server, user, found),
                Reply::DiscoveryFailed(error) => self.view.account_failed(&error),
                Reply::Subscribed { url, data } => self.add_subscription(url, &data),
                Reply::Synced(outcome) => {
                    for title in apply(self.view.library_mut(), outcome, false) {
                        self.errors
                            .push(L10N.tr_args("conflict", [("title", title.into())]));
                    }
                }
                Reply::Refreshed { calendar, data } => {
                    let library = self.view.library_mut();
                    if let Some(index) = library.calendar_index(&calendar) {
                        let outcome = subscription_outcome(&library.calendars[index].info, &data);
                        apply(library, outcome, true);
                    }
                }
                Reply::Failed { calendar, error } => {
                    let name = self.calendar_name(&calendar);
                    tracing::warn!("cannot sync {name}: {error}");
                    self.errors.push(L10N.tr_args(
                        "sync-error",
                        [("name", name.into()), ("error", error.into())],
                    ));
                }
                Reply::Finished => {
                    self.syncing = false;
                    let now = Zoned::now().with_time_zone(self.zone.clone());
                    let status = self.errors.first().cloned().unwrap_or_else(|| {
                        L10N.tr_args("synced-at", [("time", clock(&self.formats, &now).into())])
                    });
                    self.view.set_status(Some(status));
                    if std::mem::take(&mut self.sync_again) {
                        self.next_sync = Instant::now() + PUSH_DELAY;
                    }
                }
            }
        }
        if changed {
            self.view.refresh_search();
            self.next_alerts = Instant::now();
        }
        changed
    }

    fn serve(&mut self, window: &mut Window) {
        for request in self.view.take_requests() {
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
                    self.view.library_mut().remove_account(&id);
                    self.worker.send(Job::Forget(id));
                }
                Request::Changed => {
                    self.next_sync = self.next_sync.min(Instant::now() + PUSH_DELAY);
                    self.next_alerts = Instant::now();
                    self.view.refresh_search();
                }
                Request::Refresh => self.sync(true),
                Request::SetColor(index) => {
                    let library = self.view.library();
                    let job = library.calendars.get(index).and_then(|calendar| {
                        let account = library.account(&calendar.info.account)?;
                        match (&account.kind, &calendar.info.url) {
                            (AccountKind::CalDav { user, .. }, Some(url)) => Some(Job::SetColor {
                                account: account.id.clone(),
                                user: user.clone(),
                                url: url.clone(),
                                color: format!("{}FF", calendar.info.color),
                            }),
                            _ => None,
                        }
                    });
                    if let Some(job) = job {
                        self.worker.send(job);
                    }
                }
            }
        }
        let title = self.view.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
        let mode = self.view.mode();
        if mode != self.published_mode {
            self.published_mode = mode;
            if let Some(app_menu) = &self.app_menu
                && let Err(error) = app_menu.update(app_menus(mode))
            {
                tracing::debug!("cannot update the menus: {error}");
            }
        }
    }

    fn check_alerts(&mut self) {
        let now = Timestamp::now();
        let until = now
            .saturating_add(SignedDuration::from_secs(1))
            .unwrap_or(now);
        for due in due_between(
            self.view.library(),
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
        let next = due_between(self.view.library(), until, horizon, &self.zone)
            .first()
            .map_or(horizon, |due| due.at);
        let wait = now.duration_until(next).max(SignedDuration::from_secs(1));
        self.next_alerts =
            Instant::now() + Duration::try_from(wait).unwrap_or(Duration::from_secs(60));
    }

    fn menu_action(&mut self, window: &mut Window, action: &str) -> bool {
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit | Standard::CloseWindow) => {
                window.close();
                Ok(())
            }
            None => return self.view.menu_action(action),
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
        false
    }
}

impl App for CalendarApp {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        self.view.set_full_screen(full_screen);
        window.set_frame_regions(FrameRegions {
            sidebar: SIDEBAR_WIDTH,
            toolbar: match full_screen {
                true => TOOLBAR_HEIGHT,
                false => 0.0,
            },
        });
        let (width, height) = window.size();
        let scene = self.view.scene(width, height);
        window.set_header(self.view.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_appearance(appearance);
                true
            }
            WindowEvent::Focus(focused) => {
                self.view.set_focused(focused);
                if focused && let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                    tracing::warn!("{error}");
                }
                true
            }
            WindowEvent::Input(input) => {
                let changed = self.view.handle(&input);
                window.set_cursor(self.view.cursor());
                changed
            }
            WindowEvent::Accessibility(request) => self.view.perform(&request),
            WindowEvent::MenuAction(action) => self.menu_action(window, &action),
            WindowEvent::MenuClosed => self.view.menu_closed(),
            WindowEvent::Pasted(text) => self.view.pasted(&text),
            _ => false,
        };
        self.serve(window);
        if changed {
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.worker.fd()];
        sources.extend(self.app_menu.iter().map(AppMenu::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let changed = match index {
            0 => self.replies(),
            _ => {
                let actions = self
                    .app_menu
                    .as_mut()
                    .map(AppMenu::take_actions)
                    .unwrap_or_default();
                actions.iter().fold(false, |changed, action| {
                    self.menu_action(window, action) | changed
                })
            }
        };
        self.serve(window);
        if changed {
            window.request_redraw();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [
            Some(self.next_sync),
            Some(self.next_subscriptions),
            Some(self.next_clock),
            Some(self.next_alerts),
            self.view.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if now >= self.next_clock {
            self.view
                .set_now(Zoned::now().with_time_zone(self.zone.clone()));
            self.next_clock = now + CLOCK_INTERVAL;
            window.request_redraw();
        }
        if now >= self.next_alerts {
            self.check_alerts();
        }
        if now >= self.next_subscriptions {
            self.sync(true);
        } else if now >= self.next_sync {
            self.sync(false);
        }
        self.serve(window);
    }

    fn animating(&self) -> bool {
        self.view.animating()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view.text_input_area()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.view.accessibility())
    }
}

fn open_arguments(view: &mut CalendarView, arguments: &[String]) {
    for argument in arguments {
        let lower = argument.to_ascii_lowercase();
        if lower.starts_with("webcal://") || lower.starts_with("webcals://") {
            view.offer_subscription(argument);
            continue;
        }
        let path =
            unbound_xdg::path_from_file_uri(argument).unwrap_or_else(|| PathBuf::from(argument));
        match std::fs::read_to_string(&path).map(|text| CalendarObject::parse(&text)) {
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

fn run(arguments: Vec<String>) -> Result<(), WindowError> {
    let zone = TimeZone::try_system().unwrap_or(TimeZone::UTC);
    let library = Library::load(data_dir());
    let formats = DateFormats::from_env();
    let now = Zoned::now().with_time_zone(zone.clone());
    let palette = Appearance::default().palette();
    let mut view = CalendarView::new(
        library,
        palette,
        TextSystem::new(),
        formats.clone(),
        zone.clone(),
        now,
    );
    let title = view.title();
    let mut window = Window::open(Role::toplevel(&title, APP_ID), WINDOW_SIZE)?;
    view.set_appearance(window.appearance());
    open_arguments(&mut view, &arguments);
    let app_menu = AppMenu::export(APP_ID, app_menus(view.mode()))
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    let worker = Worker::start()?;
    let started = Instant::now();
    let mut app = CalendarApp {
        published_mode: view.mode(),
        view,
        worker,
        app_menu,
        zone,
        formats,
        next_sync: started,
        next_subscriptions: started,
        next_clock: started + CLOCK_INTERVAL,
        next_alerts: started,
        alerts_checked: Timestamp::now(),
        fired: HashSet::new(),
        syncing: false,
        sync_again: false,
        title,
        errors: Vec::new(),
    };
    unbound_toolkit::run(&mut window, &mut app)
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("--version") {
        println!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    unbound_log::init(PROGRAM);
    match run(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
