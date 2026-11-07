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
use bande_calendar::notice::{Notice, Notices, OPEN, SNOOZE as SNOOZE_ACTION, Subject};
use bande_calendar::online::System;
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

const APP_ID: &str = "org.bande.Calendar";
const NEW_EVENT: &str = "new-event";
const NEW_EVENT_FLAG: &str = "--new-event";
const SHOW_CALENDAR: &str = "show-calendar";
const REFRESH: &str = "refresh";
const IMPORT: &str = "import";
const EXPORT: &str = "export";
const WINDOW_SIZE: (u32, u32) = (1120, 740);
const PUSH_DELAY: Duration = Duration::from_secs(2);
const RETRY: SignedDuration = SignedDuration::from_mins(15);
const ALERT_HORIZON: SignedDuration = SignedDuration::from_hours(24);
const FIRED_MEMORY: SignedDuration = SignedDuration::from_hours(24);
const SNOOZE: SignedDuration = SignedDuration::from_mins(5);

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
                Some((_, data)) => match bande_xdg::replace_file(&chosen, data) {
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
    online: Inbox<System>,
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
    fired: HashMap<String, Timestamp>,
    notices: Notices,
    shown: HashMap<u32, Subject>,
    snoozed: Vec<(Timestamp, Due)>,
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

fn zone_menu(view: &CalendarView) -> Entry {
    let shown = view.shown_zone();
    let items =
        std::iter::once(Entry::item(&L10N.tr("zone-system"), "zone:").radio(shown.is_none()))
            .chain(view.zones().into_iter().map(|zone| {
                let chosen = shown == Some(zone.as_str());
                Entry::item(&zone, &format!("zone:{zone}")).radio(chosen)
            }))
            .collect();
    Entry::submenu(&L10N.tr("zone-menu"), items)
}

fn app_menus(view: &CalendarView, time_zones: bool) -> Vec<Entry> {
    let mode = view.mode();
    let mut view_menu: Vec<Entry> = Mode::ALL
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
    if time_zones {
        view_menu.extend([Entry::separator(), zone_menu(view)]);
    }
    view_menu.extend([
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
                entry("new-event", NEW_EVENT).shortcut("Ctrl+N"),
                Entry::separator(),
                entry("new-calendar", "new-calendar"),
                entry("add-account", "add-account"),
                entry("refresh", REFRESH).shortcut("Ctrl+R"),
                Entry::separator(),
                entry("import-menu", IMPORT),
                entry("export", EXPORT),
                Entry::separator(),
                bande_app_menu::print_item(true),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(&L10N.tr("menu-view"), view_menu),
    ]
}

fn clock(formats: &DateFormats, at: &Zoned) -> String {
    let (time, period) = formats.time(at);
    period.map_or(time.clone(), |period| format!("{time} {period}"))
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or(url)
}

impl CalendarApp {
    fn subscription_due(&self, library: &Library, info: &CalendarInfo) -> Option<Timestamp> {
        let every = match library.account(&info.account).map(|account| &account.kind) {
            Some(AccountKind::Subscription {
                refresh_minutes: Some(0),
                ..
            }) => None,
            Some(AccountKind::Subscription {
                refresh_minutes: Some(minutes),
                ..
            }) => Some(Duration::from_secs(u64::from(*minutes) * 60)),
            _ => self.settings.subscription_refresh(),
        }?;
        let refreshed = info
            .refreshed
            .and_then(|second| Timestamp::from_second(second).ok())
            .map(|at| at.min(Timestamp::now()))
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

    fn caldav_due(&self, library: &Library) -> Option<Timestamp> {
        if !library
            .accounts
            .iter()
            .any(|account| matches!(account.kind, AccountKind::CalDav { .. }))
        {
            return None;
        }
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
        let caldav = refresh == Refresh::All
            || push
            || self
                .caldav_due(view.library())
                .is_some_and(|due| now >= due);
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
                                .subscription_due(library, &calendar.info)
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
            .filter_map(|calendar| self.subscription_due(view.library(), &calendar.info))
            .min()
            .filter(|_| !syncing);
        let caldav = self.caldav_due(view.library()).filter(|_| !syncing);
        let at = [Some(self.alert_at), self.next_clock, caldav, subscriptions]
            .into_iter()
            .flatten()
            .min();
        self.alarm.set(at);
    }

    fn wake(&mut self, view: &mut CalendarView) {
        let now = Timestamp::now();
        self.next_clock = self.next_clock.map(|at| at.min(next_minute()));
        self.synced = self.synced.map(|at| at.min(now));
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
        let outcome = subscription_outcome(&info, data, false);
        view.library_mut().add_account(
            Account {
                id: account,
                name,
                kind: AccountKind::Subscription {
                    url,
                    refresh_minutes: None,
                    alerts: false,
                },
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
                        let alerts = matches!(
                            library
                                .account(&library.calendars[index].info.account)
                                .map(|account| &account.kind),
                            Some(AccountKind::Subscription { alerts: true, .. })
                        );
                        let info = &mut library.calendars[index].info;
                        info.refreshed = Some(Timestamp::now().as_second());
                        subscription_outcome(info, &data, alerts)
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
                        if self.main.is_none()
                            && let Some(known) = view.library().account(&account)
                        {
                            self.notices.sign_in_failed(account, &known.name);
                        }
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

    fn serve(&mut self, mut window: Option<&mut Window>, view: &mut CalendarView) {
        for request in view.take_requests() {
            match request {
                Request::Menu(menu, at) => {
                    if let Some(window) = window.as_deref_mut()
                        && let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at))
                    {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
                Request::Clipboard(request) => {
                    if let Some(window) = window.as_deref_mut()
                        && let Err(error) = window.serve_clipboard(request)
                    {
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
                Request::SetPassword {
                    account,
                    user,
                    password,
                } => {
                    self.asked.remove(&account);
                    self.withdraw_sign_in(&account);
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
        if let Some(window) = window
            && title != self.title
        {
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
            if self.fired.insert(due.key.clone(), due.at).is_none() {
                self.notices.notify(due, &self.zone, &self.formats);
            }
        }
        let (woken, waiting) = std::mem::take(&mut self.snoozed)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.snoozed = waiting;
        for (_, due) in woken {
            self.notices.notify(due, &self.zone, &self.formats);
        }
        let forget = now.saturating_sub(FIRED_MEMORY).unwrap_or(now);
        self.fired.retain(|_, at| *at >= forget);
        self.alerts_checked = until;
        let horizon = now.saturating_add(ALERT_HORIZON).unwrap_or(now);
        let next = due_between(view.library(), until, horizon, &self.zone)
            .first()
            .map_or(horizon, |due| due.at);
        self.alert_at = self
            .snoozed
            .iter()
            .map(|(at, _)| *at)
            .fold(next, Timestamp::min);
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
            self.serve(Some(&mut *window), view);
            window.request_redraw();
        } else if let Some(mut pane) = self.parked.take() {
            self.serve(None, &mut pane.0);
            self.parked = Some(pane);
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
        if matches!(
            change,
            Change::AddAccount | Change::RemoveAccount(_) | Change::Password(_)
        ) {
            self.raise(cx);
        }
        self.with_view(cx, |_, view| match &change {
            Change::AddAccount => view.open_account_setup(),
            Change::RemoveAccount(id) => view.ask_remove_account(id),
            Change::Password(id) => view.ask_password(id),
            Change::SubscriptionRefresh(id, minutes) => {
                view.library_mut()
                    .set_subscription(id, Some(*minutes), None)
            }
            Change::SubscriptionAlerts(id, alerts) => {
                view.library_mut().set_subscription(id, None, Some(*alerts))
            }
        });
    }

    fn notice(&mut self, cx: &mut Cx<Self>, notice: Notice) {
        match notice {
            Notice::Shown(id, Subject::SignIn(account)) if !self.asked.contains(&account) => {
                self.notices.withdraw(id);
            }
            Notice::Shown(id, subject) => {
                self.shown.insert(id, subject);
            }
            Notice::Closed(id) => {
                self.shown.remove(&id);
            }
            Notice::Invoked(id, action) => match (self.shown.remove(&id), action.as_str()) {
                (Some(Subject::Alert(due)), SNOOZE_ACTION) => {
                    let now = Timestamp::now();
                    self.snoozed
                        .push((now.saturating_add(SNOOZE).unwrap_or(now), due));
                    self.with_view(cx, |app, view| app.check_alerts(view));
                }
                (Some(Subject::Alert(due)), OPEN) => {
                    self.raise(cx);
                    let day = due.starts.to_zoned(self.zone.clone()).date();
                    self.with_view(cx, |_, view| view.go_to(day));
                }
                (Some(Subject::SignIn(account)), OPEN) => {
                    self.raise(cx);
                    self.with_view(cx, |_, view| view.ask_password(&account));
                }
                _ => {}
            },
        }
    }

    fn withdraw_sign_in(&mut self, account: &str) {
        let notices = &self.notices;
        self.shown.retain(|id, subject| match subject {
            Subject::SignIn(asked) if asked == account => {
                notices.withdraw(*id);
                false
            }
            _ => true,
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
        let (new_event, arguments) = new_event_asked(arguments);
        let zone = bande_datetime::local_zone();
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
        let online = Inbox::new()?;
        bande_calendar::online::watch(online.outbox());
        let settings_watch = settings_folder()
            .as_deref()
            .and_then(|folder| SettingsWatch::start(folder, CalendarSettings::FILE));
        let (main, parked) =
            match cx.launched_in_background() || (arguments.is_empty() && !new_event) {
                true => (None, Some(Pane(view))),
                false => (Some(show(cx, &title, Pane(view))?), None),
            };
        let mut app = Self {
            main,
            parked,
            worker,
            alarm: WallAlarm::new()?,
            changes,
            chosen: Inbox::new()?,
            online,
            settings,
            settings_watch,
            zone,
            formats,
            next_push: Some(Instant::now()),
            synced: None,
            retry: HashMap::new(),
            asked: HashSet::new(),
            next_clock: main.map(|_| next_minute()),
            alert_at: Timestamp::now(),
            alerts_checked: Timestamp::now(),
            fired: HashMap::new(),
            notices: Notices::start()?,
            shown: HashMap::new(),
            snoozed: Vec::new(),
            syncing: false,
            sync_again: false,
            title,
            errors: Vec::new(),
        };
        if new_event {
            app.command(cx, main, NEW_EVENT);
        }
        Ok(app)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        Menus {
            menus: self
                .main
                .and_then(|main| cx.view(main))
                .or(self.parked.as_ref())
                .map(|Pane(view)| app_menus(view, self.settings.time_zones))
                .unwrap_or_default(),
            window: vec![entry("app-name", SHOW_CALENDAR)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        match command {
            REFRESH => self.with_view(cx, |app, view| app.sync(view, Refresh::All)),
            bande_app_menu::PRINT => {
                let mut printable = None;
                self.with_view(cx, |_, view| printable = Some(view.printable()));
                if let Some((title, text)) = printable {
                    let document = bande_app::text_document(format!("{title}\n{text}"), false);
                    cx.print(&title, document);
                }
            }
            IMPORT => choose(self.chosen.outbox(), None),
            _ => {
                if self.main.is_none() && command != EXPORT {
                    self.reopen(cx);
                } else if command == SHOW_CALENDAR {
                    self.raise(cx);
                }
                self.with_view(cx, |_, view| {
                    view.menu_action(command);
                });
            }
        }
        self.settle(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        let (new_event, arguments) = new_event_asked(arguments);
        self.with_view(cx, |_, view| {
            open_arguments(view, &arguments, directory);
        });
        self.reopen(cx);
        self.settle(cx);
        if new_event {
            self.command(cx, None, NEW_EVENT);
        }
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
            self.online.fd(),
            self.notices.fd(),
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
                    let mut shown = false;
                    self.with_view(cx, |_, view| match chosen {
                        Chosen::Open(path) => {
                            shown = open_arguments(
                                view,
                                &[path.to_string_lossy().into_owned()],
                                Path::new("/"),
                            );
                        }
                        Chosen::Failed(error) => {
                            view.set_status(Some(error));
                            shown = true;
                        }
                    });
                    if shown {
                        self.raise(cx);
                    }
                }
            }
            4 => {
                for change in self.online.drain() {
                    match change {
                        System::Online => {
                            self.retry.clear();
                            self.push_soon(Duration::ZERO);
                        }
                        System::Zone(name) => {
                            self.zone = TimeZone::get(&name)
                                .unwrap_or_else(|_| bande_datetime::local_zone());
                            self.with_view(cx, |app, view| {
                                view.set_system_zone(app.zone.clone());
                                app.check_alerts(view);
                            });
                        }
                    }
                }
            }
            5 => {
                for notice in self.notices.take() {
                    self.notice(cx, notice);
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
    fn runs_in_background() -> bool {
        load_settings().alerts_when_closed
    }
}

fn new_event_asked(arguments: Vec<String>) -> (bool, Vec<String>) {
    let (flags, arguments): (Vec<String>, Vec<String>) = arguments
        .into_iter()
        .partition(|argument| argument == NEW_EVENT_FLAG);
    (!flags.is_empty(), arguments)
}

fn open_arguments(view: &mut CalendarView, arguments: &[String], directory: &Path) -> bool {
    let mut offered = false;
    for argument in arguments {
        let lower = argument.to_ascii_lowercase();
        if lower.starts_with("webcal://") || lower.starts_with("webcals://") {
            view.offer_subscription(argument);
            offered = true;
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
                offered |= view.offer_import(objects, &name);
            }
            Ok(Ok(_)) => tracing::warn!("{} has no events", path.display()),
            Ok(Err(error)) => tracing::warn!("cannot read {}: {error}", path.display()),
            Err(error) => tracing::warn!("cannot open {}: {error}", path.display()),
        }
    }
    offered
}

fn main() -> ExitCode {
    bande_app::run::<CalendarApp>(&INFO)
}
