use std::collections::{HashMap, HashSet};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_app_settings::{SettingsWatch, Stored, settings_folder};
use bande_calendar::alerts::{Due, due_between};
use bande_calendar::caldav::Discovered;
use bande_calendar::ical::{CalendarObject, read_file};
use bande_calendar::portal::{Choose, choose_file};
use bande_calendar::settings::{CalendarSettings, Change, listen};
use bande_calendar::store::{Account, AccountKind, CalendarInfo, Library, data_dir, new_uid};
use bande_calendar::sync::{apply, subscription_outcome};
use bande_calendar::ui::painter::{color_text, parse_color};
use bande_calendar::ui::{
    CalendarView, L10N, Mode, Request, SETTINGS_SIZE, SIDEBAR_WIDTH, SettingsView, TOOLBAR_HEIGHT,
};
use bande_calendar::wake::WallAlarm;
use bande_calendar::worker::{Job, Reply, Worker};
use bande_datetime::DateFormats;
use bande_design::{Accent, Scheme};
use bande_inbox::Inbox;
use bande_toolkit::{
    Appearance, FrameRegions, MenuPlacement, Rect, Scene, TextSystem, Window, WindowEvent,
};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, Zoned};
use zbus::zvariant::Value;

const APP_ID: &str = "org.bande.Calendar";
const WINDOW_SIZE: (u32, u32) = (1120, 740);
const PUSH_DELAY: Duration = Duration::from_secs(2);
const RETRY: SignedDuration = SignedDuration::from_mins(15);
const ALERT_HORIZON: SignedDuration = SignedDuration::from_hours(1);
const NOTIFICATION_TIMEOUT: i32 = -1;

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || L10N.tr("settings-title"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsView::new(appearance))
}

enum Chosen {
    Open(PathBuf),
    Failed(String),
}

fn choose(outbox: bande_inbox::Outbox<Chosen>, export: Option<(String, String)>) {
    let spawned = std::thread::Builder::new()
        .name("choose".to_owned())
        .spawn(move || {
            let (title, save) = match &export {
                Some((name, _)) => (L10N.tr("export-dialog-title"), Choose::Save(name)),
                None => (L10N.tr("import-dialog-title"), Choose::Open),
            };
            let chosen = match choose_file(&title, save) {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(error) => {
                    tracing::warn!("cannot choose a file: {error}");
                    return;
                }
            };
            let done = match export {
                None => Chosen::Open(chosen),
                Some((_, data)) => match std::fs::write(&chosen, data) {
                    Ok(()) => return,
                    Err(error) => Chosen::Failed(L10N.tr_args(
                        "export-failed",
                        [
                            ("file", chosen.display().to_string().into()),
                            ("error", error.to_string().into()),
                        ],
                    )),
                },
            };
            outbox.send(done);
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot choose a file: {error}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refresh {
    Due,
    All,
}

struct CalendarApp {
    main: Option<WindowId>,
    parked: Option<Pane>,
    worker: Worker,
    alarm: WallAlarm,
    changes: Inbox<Change>,
    chosen: Inbox<Chosen>,
    settings: CalendarSettings,
    settings_watch: Option<SettingsWatch>,
    zone: TimeZone,
    formats: DateFormats,
    next_push: Option<Instant>,
    synced: Option<Timestamp>,
    retry: HashMap<String, Timestamp>,
    asked: HashSet<String>,
    next_clock: Option<Timestamp>,
    alert_at: Timestamp,
    alerts_checked: Timestamp,
    fired: HashSet<String>,
    syncing: bool,
    sync_again: bool,
    title: String,
    errors: Vec<String>,
}

fn next_minute() -> Timestamp {
    let now = Timestamp::now().as_second();
    Timestamp::from_second(now - now.rem_euclid(60) + 60).unwrap_or(Timestamp::MAX)
}

fn later(at: Timestamp, by: Duration) -> Timestamp {
    SignedDuration::try_from(by)
        .ok()
        .and_then(|by| at.checked_add(by).ok())
        .unwrap_or(Timestamp::MAX)
}

fn load_settings() -> CalendarSettings {
    settings_folder()
        .as_deref()
        .map(CalendarSettings::load)
        .unwrap_or_default()
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
                Entry::separator(),
                entry("import-menu", "import"),
                entry("export", "export"),
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
    fn subscription_due(&self, info: &CalendarInfo) -> Option<Timestamp> {
        let every = self.settings.subscription_refresh()?;
        let refreshed = info
            .refreshed
            .and_then(|second| Timestamp::from_second(second).ok())
            .map_or(Timestamp::UNIX_EPOCH, |at| later(at, every));
        Some(
            refreshed.max(
                self.retry
                    .get(&info.id)
                    .copied()
                    .unwrap_or(Timestamp::UNIX_EPOCH),
            ),
        )
    }

    fn caldav_due(&self) -> Option<Timestamp> {
        let every = self.settings.refresh()?;
        Some(
            self.synced
                .map_or(Timestamp::UNIX_EPOCH, |at| later(at, every)),
        )
    }

    fn sync(&mut self, view: &mut CalendarView, refresh: Refresh) {
        if self.syncing {
            if self.next_push.take().is_some() || refresh == Refresh::All {
                self.sync_again = true;
            }
            return;
        }
        let now = Timestamp::now();
        let push = self.next_push.is_some_and(|at| Instant::now() >= at);
        let caldav =
            refresh == Refresh::All || push || self.caldav_due().is_some_and(|due| now >= due);
        let library = view.library();
        let calendars: Vec<_> = library
            .calendars
            .iter()
            .filter_map(|calendar| {
                let account = library.account(&calendar.info.account)?;
                let wanted = match &account.kind {
                    AccountKind::Local => false,
                    AccountKind::CalDav { .. } => caldav,
                    AccountKind::Subscription { .. } => {
                        refresh == Refresh::All
                            || self
                                .subscription_due(&calendar.info)
                                .is_some_and(|due| now >= due)
                    }
                };
                wanted.then(|| {
                    (
                        account.kind.clone(),
                        account.id.clone(),
                        calendar.for_sync(),
                    )
                })
            })
            .collect();
        if caldav {
            self.next_push = None;
            self.synced = Some(now);
        }
        if calendars.is_empty() {
            return;
        }
        let retry = self
            .settings
            .subscription_refresh()
            .and_then(|every| SignedDuration::try_from(every).ok())
            .map_or(RETRY, |every| every.min(RETRY));
        for (kind, _, calendar) in &calendars {
            if matches!(kind, AccountKind::Subscription { .. }) {
                self.retry.insert(
                    calendar.info.id.clone(),
                    now.saturating_add(retry).unwrap_or(now),
                );
            }
        }
        self.syncing = true;
        self.errors.clear();
        view.set_status(Some(L10N.tr("syncing")));
        self.worker.send(Job::Sync(calendars));
    }

    fn rearm(&mut self, view: &CalendarView) {
        let syncing = self.syncing;
        let subscriptions = view
            .library()
            .calendars
            .iter()
            .filter(|calendar| {
                view.library()
                    .account(&calendar.info.account)
                    .is_some_and(|account| matches!(account.kind, AccountKind::Subscription { .. }))
            })
            .filter_map(|calendar| self.subscription_due(&calendar.info))
            .min()
            .filter(|_| !syncing);
        let caldav = self.caldav_due().filter(|_| !syncing);
        let at = [Some(self.alert_at), self.next_clock, caldav, subscriptions]
            .into_iter()
            .flatten()
            .min();
        self.alarm.set(at);
    }

    fn wake(&mut self, view: &mut CalendarView) {
        let now = Timestamp::now();
        if self.next_clock.is_some_and(|at| now >= at) {
            view.set_now(Zoned::now().with_time_zone(self.zone.clone()));
            self.next_clock = Some(next_minute());
        }
        if now >= self.alert_at {
            self.check_alerts(view);
        }
        self.sync(view, Refresh::Due);
        self.rearm(view);
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
                refreshed: None,
            })
            .collect();
        let account = Account {
            id,
            name,
            kind: AccountKind::CalDav {
                server,
                user,
                home: found.home,
                addresses: found.addresses,
            },
        };
        view.library_mut().add_account(account, calendars);
        view.account_added();
        self.push_soon(Duration::ZERO);
    }

    fn push_soon(&mut self, delay: Duration) {
        let at = Instant::now() + delay;
        self.next_push = Some(self.next_push.map_or(at, |next| next.min(at)));
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
            refreshed: Some(Timestamp::now().as_second()),
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
                        self.sync_again = true;
                        self.errors
                            .push(L10N.tr_args("conflict", [("title", title.into())]));
                    }
                }
                Reply::Refreshed { calendar, data } => {
                    let library = view.library_mut();
                    let outcome = library.calendar_index(&calendar).and_then(|index| {
                        let info = &mut library.calendars[index].info;
                        info.refreshed = Some(Timestamp::now().as_second());
                        subscription_outcome(info, &data)
                    });
                    self.retry.remove(&calendar);
                    match outcome {
                        Some(outcome) => {
                            apply(library, outcome, true);
                        }
                        None => tracing::warn!("{calendar} did not answer with a calendar"),
                    }
                }
                Reply::SignInFailed { account } => {
                    if self.asked.insert(account.clone()) {
                        view.ask_password(&account);
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
                        refreshed: None,
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
                        self.push_soon(PUSH_DELAY);
                    }
                }
            }
        }
        if changed {
            view.refresh_search();
            self.check_alerts(view);
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
                    self.push_soon(PUSH_DELAY);
                    self.check_alerts(view);
                    view.refresh_search();
                }
                Request::Refresh => self.sync(view, Refresh::All),
                Request::Import => choose(self.chosen.outbox(), None),
                Request::SetPassword {
                    account,
                    user,
                    password,
                } => {
                    self.asked.remove(&account);
                    self.worker.send(Job::SetPassword {
                        account,
                        user,
                        password,
                    });
                    self.push_soon(Duration::ZERO);
                }
                Request::Export { name, data } => choose(self.chosen.outbox(), Some((name, data))),
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
        self.alert_at = next;
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
            WindowEvent::ToplevelChanged(state) => !state.suspended,
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
    fn follow_clock(&mut self, cx: &mut Cx<Self>) {
        let on_screen = self
            .main
            .and_then(|main| cx.window(main))
            .is_some_and(|(shown, _)| !shown.toplevel_state().suspended);
        if !on_screen {
            self.next_clock = None;
        } else if self.next_clock.is_none() {
            self.with_view(cx, |app, view| {
                view.set_now(Zoned::now().with_time_zone(app.zone.clone()));
            });
            self.next_clock = Some(next_minute());
        }
    }

    fn settle(&mut self, cx: &mut Cx<Self>) {
        if let Some((window, Pane(view))) = self.main.and_then(|main| cx.window(main)) {
            self.serve(window, view);
            window.request_redraw();
        }
        self.with_view(cx, |app, view| app.rearm(view));
        cx.refresh_menus();
    }

    fn raise(&mut self, cx: &mut Cx<Self>) {
        self.reopen(cx);
        if let Some((window, _)) = self.main.and_then(|main| cx.window(main))
            && let Err(error) = window
                .activation_token(APP_ID)
                .and_then(|token| window.activate(&token))
        {
            tracing::warn!("cannot raise Calendar: {error}");
        }
    }

    fn apply_change(&mut self, cx: &mut Cx<Self>, change: Change) {
        self.raise(cx);
        self.with_view(cx, |_, view| match &change {
            Change::AddAccount => view.open_account_setup(),
            Change::RemoveAccount(id) => view.ask_remove_account(id),
            Change::Password(id) => view.ask_password(id),
        });
    }

    fn follow_settings(&mut self, cx: &mut Cx<Self>) {
        if let Some(watch) = &mut self.settings_watch {
            watch.drain();
        }
        let settings = load_settings();
        if settings == self.settings {
            return;
        }
        self.settings = settings.clone();
        self.with_view(cx, |_, view| view.set_settings(settings));
    }

    fn with_view(&mut self, cx: &mut Cx<Self>, work: impl FnOnce(&mut Self, &mut CalendarView)) {
        if let Some((_, Pane(view))) = self.main.and_then(|main| cx.window(main)) {
            return work(self, view);
        }
        if let Some(mut pane) = self.parked.take() {
            work(self, &mut pane.0);
            self.parked = Some(pane);
        }
    }
}

fn show(
    cx: &mut Cx<CalendarApp>,
    title: &str,
    mut pane: Pane,
) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(Kind::Main, title, WINDOW_SIZE, |appearance| {
        pane.0.set_appearance(appearance);
        pane
    })
}

impl Program for CalendarApp {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let zone = TimeZone::try_system().unwrap_or(TimeZone::UTC);
        let library = Library::load(data_dir());
        let formats = DateFormats::from_env();
        let settings = load_settings();
        let now = Zoned::now().with_time_zone(zone.clone());
        let mut view = CalendarView::new(
            library,
            Appearance::default().palette(),
            TextSystem::new(),
            formats.clone(),
            settings.clone(),
            zone.clone(),
            now,
        );
        let title = view.title();
        let directory = std::env::current_dir().unwrap_or_default();
        open_arguments(&mut view, &arguments, &directory);
        let worker = Worker::start()?;
        let changes = Inbox::new()?;
        listen(changes.outbox());
        let settings_watch = settings_folder()
            .as_deref()
            .and_then(|folder| SettingsWatch::start(folder, CalendarSettings::FILE));
        let main = show(cx, &title, Pane(view))?;
        Ok(Self {
            main: Some(main),
            parked: None,
            worker,
            alarm: WallAlarm::new()?,
            changes,
            chosen: Inbox::new()?,
            settings,
            settings_watch,
            zone,
            formats,
            next_push: Some(Instant::now()),
            synced: None,
            retry: HashMap::new(),
            asked: HashSet::new(),
            next_clock: Some(next_minute()),
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
            menus: self
                .main
                .and_then(|main| cx.view(main))
                .map(|Pane(view)| app_menus(view.mode()))
                .unwrap_or_default(),
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        self.with_view(cx, |_, view| {
            view.menu_action(command);
        });
        self.settle(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        self.with_view(cx, |_, view| open_arguments(view, &arguments, directory));
        self.reopen(cx);
        self.settle(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.main.is_some() {
            return;
        }
        let Some(pane) = self.parked.take() else {
            return;
        };
        let title = pane.0.title();
        match show(cx, &title, pane) {
            Ok(main) => {
                self.main = Some(main);
                self.title = title;
                self.follow_clock(cx);
            }
            Err(error) => tracing::warn!("cannot open Calendar: {error}"),
        }
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, view: Pane) {
        if self.main == Some(window) {
            self.main = None;
            self.parked = Some(view);
            self.next_clock = None;
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.follow_clock(cx);
        self.settle(cx);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        [
            self.worker.fd(),
            self.alarm.fd(),
            self.changes.fd(),
            self.chosen.fd(),
        ]
        .into_iter()
        .chain(self.settings_watch.iter().map(SettingsWatch::fd))
        .collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        match index {
            0 => self.with_view(cx, |app, view| {
                app.replies(view);
                app.wake(view);
            }),
            1 => {
                self.alarm.take();
                self.with_view(cx, |app, view| app.wake(view));
            }
            2 => {
                for change in self.changes.drain() {
                    self.apply_change(cx, change);
                }
            }
            3 => {
                for chosen in self.chosen.drain() {
                    self.raise(cx);
                    self.with_view(cx, |_, view| match chosen {
                        Chosen::Open(path) => open_arguments(
                            view,
                            &[path.to_string_lossy().into_owned()],
                            Path::new("/"),
                        ),
                        Chosen::Failed(error) => view.set_status(Some(error)),
                    });
                }
            }
            _ => self.follow_settings(cx),
        }
        self.settle(cx);
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_push
    }

    fn tick(&mut self, cx: &mut Cx<Self>, _now: Instant) {
        self.with_view(cx, |app, view| app.wake(view));
        self.follow_clock(cx);
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
