use bande_design::{Accent, Scheme};
use bande_toolkit::Point;
use bande_toolkit::ui::{
    Event, Key, Menu, MenuItem, Modifiers, Pointer, PointerButton, ScrollSource,
};
use jiff::ToSpan;
use jiff::civil::{DateTime, Time};

use super::geometry::{self, TimeGrid};
use super::{
    CalendarView, Confirm, Drag, Hit, L10N, Mode, Part, Request, SIDEBAR_WIDTH, Signal,
    TOOLBAR_HEIGHT,
};
use crate::edit::{Scope, new_event, shifted};
use crate::store::Found;

const DRAG_THRESHOLD: f32 = 4.0;
const WHEEL_STEP: f32 = 40.0;

pub const ACTION_TODAY: &str = "today";
pub const ACTION_PREVIOUS: &str = "previous";
pub const ACTION_NEXT: &str = "next";
pub const ACTION_NEW_EVENT: &str = "new-event";
pub const ACTION_FIND: &str = "find";
pub const ACTION_REFRESH: &str = "refresh";
pub const ACTION_ADD_ACCOUNT: &str = "add-account";
pub const ACTION_DELETE: &str = "delete";
pub const ACTION_MODE: &str = "mode-";
const ACTION_COLOR: &str = "calendar-color:";
const ACTION_REMOVE_ACCOUNT: &str = "remove-account:";
const ACTION_RENAME: &str = "calendar-rename:";
const ACTION_DELETE_CALENDAR: &str = "calendar-delete:";
const ACTION_NEW_CALENDAR: &str = "new-calendar";
const ACTION_NEW_CALENDAR_IN: &str = "new-calendar:";

const COLORS: [(Accent, &str); 8] = [
    (Accent::Rose, "color-rose"),
    (Accent::Ember, "color-ember"),
    (Accent::Amber, "color-amber"),
    (Accent::Moss, "color-moss"),
    (Accent::Teal, "color-teal"),
    (Accent::Ultramarine, "color-ultramarine"),
    (Accent::Plum, "color-plum"),
    (Accent::Graphite, "color-graphite"),
];

fn calendar_menu(view: &CalendarView, index: usize) -> Option<Menu> {
    let calendar = view.library.calendars.get(index)?;
    let current = super::painter::parse_color(&calendar.info.color).map(Accent::nearest);
    let mut items: Vec<MenuItem> = COLORS
        .iter()
        .enumerate()
        .map(|(position, (accent, label))| {
            MenuItem::new(
                &L10N.tr(label),
                &format!("{ACTION_COLOR}{index}:{position}"),
            )
            .checked(current == Some(*accent))
        })
        .collect();
    let subscription = calendar.info.read_only && calendar.info.url.is_none();
    let locals = view
        .library
        .calendars
        .iter()
        .filter(|other| other.info.account == crate::store::LOCAL_ACCOUNT)
        .count();
    items.push(MenuItem::separator());
    items.push(MenuItem::new(
        &L10N.tr("rename-calendar"),
        &format!("{ACTION_RENAME}{index}"),
    ));
    if !subscription && (calendar.info.account != crate::store::LOCAL_ACCOUNT || locals > 1) {
        items.push(MenuItem::new(
            &L10N.tr("delete-calendar"),
            &format!("{ACTION_DELETE_CALENDAR}{index}"),
        ));
    }
    if calendar.info.account != crate::store::LOCAL_ACCOUNT {
        items.push(MenuItem::separator());
        items.push(MenuItem::new(
            &L10N.tr("remove-account"),
            &format!("{ACTION_REMOVE_ACCOUNT}{}", calendar.info.account),
        ));
    }
    Some(Menu::new(items))
}

fn recolor(view: &mut CalendarView, action: &str) -> bool {
    let Some((index, position)) = action.split_once(':') else {
        return false;
    };
    let (Ok(index), Some((accent, _))) = (
        index.parse::<usize>(),
        position
            .parse::<usize>()
            .ok()
            .and_then(|position| COLORS.get(position)),
    ) else {
        return false;
    };
    let Some(calendar) = view.library.calendars.get_mut(index) else {
        return false;
    };
    calendar.info.color = super::painter::color_text(accent.color(Scheme::Light));
    view.library.save(index);
    let info = &view.library.calendars[index].info;
    if let Some(url) = info.url.clone() {
        view.requests.push(Request::Collection {
            account: info.account.clone(),
            url,
            change: crate::worker::Change::Color(format!("{}FF", info.color)),
        });
    }
    true
}

pub fn menu_command(view: &mut CalendarView, action: &str) -> bool {
    match action {
        ACTION_TODAY => signal(view, Signal::Today),
        ACTION_PREVIOUS => signal(view, Signal::Previous),
        ACTION_NEXT => signal(view, Signal::Next),
        ACTION_NEW_EVENT => signal(view, Signal::NewEvent),
        ACTION_ADD_ACCOUNT => signal(view, Signal::AddAccount),
        ACTION_REFRESH => view.requests.push(Request::Refresh),
        ACTION_FIND => {
            let search = view.toolbar.search;
            view.ui.set_focus(search);
        }
        ACTION_DELETE => {
            if let Some(found) = view.selected_found() {
                view.ask_delete(found);
            }
        }
        other if other.starts_with(ACTION_COLOR) => {
            return recolor(view, &other[ACTION_COLOR.len()..]);
        }
        ACTION_NEW_CALENDAR => view.open_new_calendar(None),
        other if other.starts_with(ACTION_NEW_CALENDAR_IN) => {
            let account = other[ACTION_NEW_CALENDAR_IN.len()..].to_owned();
            view.open_new_calendar(Some(&account));
        }
        other if other.starts_with(ACTION_RENAME) => match other[ACTION_RENAME.len()..].parse() {
            Ok(index) => view.open_rename(index),
            Err(_) => return false,
        },
        other if other.starts_with(ACTION_DELETE_CALENDAR) => {
            match other[ACTION_DELETE_CALENDAR.len()..].parse() {
                Ok(index) => view.ask_delete_calendar(index),
                Err(_) => return false,
            }
        }
        other if other.starts_with(ACTION_REMOVE_ACCOUNT) => {
            view.ask_remove_account(&other[ACTION_REMOVE_ACCOUNT.len()..])
        }
        other => match other
            .strip_prefix(ACTION_MODE)
            .and_then(|index| index.parse::<usize>().ok())
        {
            Some(index) => signal(view, Signal::Mode(index)),
            None => return false,
        },
    }
    true
}

fn hit_at(view: &CalendarView, point: Point) -> Option<Hit> {
    view.hits
        .iter()
        .rev()
        .find(|(rect, _)| rect.contains(point))
        .map(|(_, hit)| hit.clone())
}

fn writable(view: &CalendarView, found: &Found) -> bool {
    view.library
        .calendars
        .get(found.calendar)
        .is_some_and(|calendar| !calendar.info.read_only)
}

fn over_controls(view: &CalendarView, point: Point) -> bool {
    let inspector = view
        .inspector
        .as_ref()
        .map(|inspector| inspector.frame(view.sheet()))
        .is_some_and(|frame| frame.contains(point));
    let add_account = point.x < SIDEBAR_WIDTH && point.y > view.size.1 - 44.0;
    point.y < TOOLBAR_HEIGHT || inspector || add_account
}

pub fn handle(view: &mut CalendarView, event: &Event) -> bool {
    if view.ui.has_sheet() {
        let handled = view.ui.handle(event);
        view.drain();
        return handled;
    }
    let handled = match event {
        Event::Key { key, modifiers } => key_down(view, *key, *modifiers, event),
        Event::PointerDown(pointer)
            if over_controls(view, pointer.position) && view.drag.is_none() =>
        {
            view.ui.handle(event)
        }
        Event::PointerDown(pointer) => pointer_down(view, pointer),
        Event::PointerMove(pointer) => pointer_move(view, pointer, event),
        Event::PointerUp(pointer) => pointer_up(view, pointer, event),
        Event::Scroll(scroll) if !over_controls(view, scroll.position) => {
            let step = match scroll.source {
                ScrollSource::Wheel => scroll.delta.y.signum() * WHEEL_STEP,
                _ => scroll.delta.y,
            };
            scroll_by(
                view,
                scroll.position,
                step,
                scroll.source == ScrollSource::Wheel,
            )
        }
        Event::PointerLeave => {
            view.hover = None;
            view.press = None;
            let dropped = view.drag.take().is_some();
            view.ui.handle(event) | dropped
        }
        _ => view.ui.handle(event),
    };
    view.drain();
    handled
}

fn scroll_by(view: &mut CalendarView, at: Point, step: f32, wheel: bool) -> bool {
    if at.x < SIDEBAR_WIDTH {
        if wheel || step.abs() > WHEEL_STEP {
            view.mini_month = view
                .mini_month
                .saturating_add((step.signum() as i64).months());
            return true;
        }
        return false;
    }
    match view.mode {
        Mode::Day | Mode::Week => {
            view.scroll += step;
            true
        }
        Mode::Month | Mode::Year if wheel => {
            view.shift(step.signum() as i32);
            true
        }
        _ => false,
    }
}

fn key_down(view: &mut CalendarView, key: Key, modifiers: Modifiers, event: &Event) -> bool {
    if modifiers.logo {
        return false;
    }
    let editing = view.ui.focus().is_some();
    if editing {
        match key {
            Key::Escape if view.inspector.is_some() => {
                view.close_inspector(true);
                return true;
            }
            Key::Enter
                if view
                    .inspector
                    .as_ref()
                    .is_some_and(|inspector| inspector.title_field == view.ui.focus()) =>
            {
                view.close_inspector(true);
                return true;
            }
            Key::Escape if view.inspector.is_none() => {
                view.ui.clear_focus();
                return true;
            }
            _ if view.ui.handle(event) => return true,
            _ => {}
        }
    }
    let command = modifiers.control;
    match key {
        Key::Character(character) if command => match character.to_ascii_lowercase() {
            't' => menu_command(view, ACTION_TODAY),
            'n' if modifiers.alt => menu_command(view, ACTION_NEW_CALENDAR),
            'n' => menu_command(view, ACTION_NEW_EVENT),
            'f' => menu_command(view, ACTION_FIND),
            'r' => menu_command(view, ACTION_REFRESH),
            digit @ '1'..='4' => {
                menu_command(view, &format!("{ACTION_MODE}{}", digit as u32 - '1' as u32))
            }
            _ => view.ui.handle(event),
        },
        Key::Left => menu_command(view, ACTION_PREVIOUS),
        Key::Right => menu_command(view, ACTION_NEXT),
        Key::Up | Key::Down if matches!(view.mode, Mode::Day | Mode::Week) => {
            view.scroll += match key {
                Key::Up => -geometry::HOUR_HEIGHT,
                _ => geometry::HOUR_HEIGHT,
            };
            true
        }
        Key::Delete | Key::Backspace => menu_command(view, ACTION_DELETE),
        Key::Enter => match view.selected_found() {
            Some(found) => {
                view.open_inspector(found);
                true
            }
            None => false,
        },
        Key::Escape => {
            let had = view.selected.take().is_some() || view.inspector.is_some();
            view.close_inspector(true);
            had
        }
        _ => view.ui.handle(event),
    }
}

fn grid(view: &CalendarView) -> Option<TimeGrid> {
    view.grid
}

fn time_at(view: &CalendarView, point: Point) -> Option<(DateTime, bool)> {
    let grid = grid(view)?;
    let band = grid.all_day_area();
    let column = grid.clamped_column(point.x);
    if point.y < band.y + band.height {
        return Some((grid.day(column).to_datetime(Time::midnight()), true));
    }
    let minutes = geometry::snap(grid.minutes_at(point.y));
    Some((geometry::at_minutes(grid.day(column), minutes), false))
}

fn day_at(view: &CalendarView, point: Point) -> Option<DateTime> {
    match hit_at(view, point)? {
        Hit::MonthDay(day) | Hit::AllDay(day) | Hit::YearDay(day) => {
            Some(day.to_datetime(Time::midnight()))
        }
        Hit::Event(index, _) => view
            .shown
            .get(index)
            .map(|found| found.occurrence.first_day.to_datetime(Time::midnight())),
        _ => None,
    }
}

fn pointer_down(view: &mut CalendarView, pointer: &Pointer) -> bool {
    if view.inspector.is_some() {
        view.close_inspector(true);
        return true;
    }
    if view.ui.focus().is_some() {
        view.ui.clear_focus();
    }
    let point = pointer.position;
    let double = pointer.count >= 2;
    view.press = Some(point);
    let hit = hit_at(view, point);
    if pointer.button == PointerButton::Secondary {
        return match hit {
            Some(Hit::Calendar(index)) => match calendar_menu(view, index) {
                Some(menu) => {
                    view.requests.push(Request::Menu(menu, point));
                    true
                }
                None => false,
            },
            Some(Hit::Account(id)) => {
                let Some(account) = view.library.account(&id) else {
                    return false;
                };
                let mut items = Vec::new();
                if !matches!(account.kind, crate::store::AccountKind::Subscription { .. }) {
                    items.push(MenuItem::new(
                        &L10N.tr("new-calendar"),
                        &format!("{ACTION_NEW_CALENDAR_IN}{id}"),
                    ));
                }
                if id != crate::store::LOCAL_ACCOUNT {
                    items.push(MenuItem::new(
                        &L10N.tr("remove-account"),
                        &format!("{ACTION_REMOVE_ACCOUNT}{id}"),
                    ));
                }
                view.requests.push(Request::Menu(Menu::new(items), point));
                true
            }
            Some(Hit::Event(index, _)) => match view.shown.get(index).cloned() {
                Some(found) => {
                    view.selected = Some(view.selection_of(&found));
                    view.open_inspector(found);
                    true
                }
                None => false,
            },
            _ => false,
        };
    }
    match hit {
        Some(Hit::Calendar(index)) => {
            if let Some(calendar) = view.library.calendars.get_mut(index) {
                calendar.info.visible = !calendar.info.visible;
                view.library.save(index);
            }
            true
        }
        Some(Hit::Account(_)) => false,
        Some(Hit::MiniDay(day)) => {
            view.go_to(day);
            true
        }
        Some(Hit::MiniShift(steps)) => {
            view.mini_month = view.mini_month.saturating_add(i64::from(steps).months());
            true
        }
        Some(Hit::Result(index)) => {
            if let Some(found) = view.results.get(index).cloned() {
                view.go_to(found.occurrence.first_day);
                view.selected = Some(view.selection_of(&found));
                if double {
                    view.pending_open = view.selected.clone();
                }
            }
            true
        }
        Some(Hit::Event(index, part)) => {
            let Some(found) = view.shown.get(index).cloned() else {
                return false;
            };
            view.selected = Some(view.selection_of(&found));
            if double {
                view.open_inspector(found);
                return true;
            }
            if writable(view, &found) {
                let at = match view.mode {
                    Mode::Month => day_at(view, point)
                        .map(|day| (day.date().to_datetime(found.occurrence.start.time()), true)),
                    _ => time_at(view, point),
                };
                if let Some((grab, all_day)) = at {
                    view.drag = Some(match part {
                        Part::End => Drag::Resize {
                            end: found.occurrence.end.datetime(),
                            found,
                        },
                        Part::Body => Drag::Move {
                            found,
                            grab,
                            current: grab,
                            all_day: match view.mode {
                                Mode::Month => false,
                                _ => all_day,
                            },
                            moved: false,
                        },
                    });
                }
            }
            true
        }
        Some(Hit::AllDay(day)) => {
            view.selected = None;
            let at = day.to_datetime(Time::midnight());
            view.drag = Some(Drag::Create {
                anchor: at,
                current: at,
                all_day: true,
            });
            if double {
                create(view, at, at, true);
            }
            true
        }
        Some(Hit::MonthDay(day)) => {
            view.selected = None;
            match (view.mode, double) {
                (Mode::Month, true) => create(
                    view,
                    day.to_datetime(Time::midnight()),
                    day.to_datetime(Time::midnight()),
                    true,
                ),
                (Mode::Month, false) => view.focus = day,
                (_, true) => {
                    view.go_to(day);
                    view.set_mode(Mode::Day);
                }
                (_, false) => {}
            }
            true
        }
        Some(Hit::YearDay(day)) => {
            view.go_to(day);
            if double {
                view.set_mode(Mode::Day);
            }
            true
        }
        Some(Hit::More(day)) => {
            view.go_to(day);
            view.set_mode(Mode::Day);
            true
        }
        Some(Hit::YearMonth(first)) => {
            view.go_to(first);
            view.set_mode(Mode::Month);
            true
        }
        None => match time_at(view, point).filter(|_| view.sheet().contains(point)) {
            Some((at, all_day)) => {
                view.selected = None;
                view.drag = Some(Drag::Create {
                    anchor: at,
                    current: at,
                    all_day,
                });
                if double && !all_day {
                    let end = at.checked_add(1.hour()).unwrap_or(at);
                    create(view, at, end, false);
                }
                true
            }
            None => view.selected.take().is_some(),
        },
    }
}

fn pointer_move(view: &mut CalendarView, pointer: &Pointer, event: &Event) -> bool {
    let point = pointer.position;
    let moved_enough = view.press.is_some_and(|press| {
        (press.x - point.x).abs() + (press.y - point.y).abs() > DRAG_THRESHOLD
    });
    let target = match view.mode {
        Mode::Month => day_at(view, point).map(|day| (day, true)),
        _ => time_at(view, point),
    };
    let mode = view.mode;
    match (&mut view.drag, target) {
        (
            Some(Drag::Create {
                current, all_day, ..
            }),
            Some((at, at_all_day)),
        ) => {
            let at = match (*all_day, at_all_day) {
                (true, _) => at.date().to_datetime(Time::midnight()),
                (false, true) => *current,
                (false, false) => at,
            };
            let changed = *current != at;
            *current = at;
            changed
        }
        (
            Some(Drag::Move {
                current,
                all_day,
                moved,
                found,
                ..
            }),
            Some((at, at_all_day)),
        ) => {
            *moved |= moved_enough;
            let at = match mode {
                Mode::Month => at.date().to_datetime(found.occurrence.start.time()),
                _ => at,
            };
            let changed = *current != at || *all_day != at_all_day && mode != Mode::Month;
            *current = at;
            if mode != Mode::Month {
                *all_day = at_all_day;
            }
            changed && *moved
        }
        (Some(Drag::Resize { end, found }), Some((at, false))) => {
            let at = at.max(
                found
                    .occurrence
                    .start
                    .datetime()
                    .checked_add(15.minutes())
                    .unwrap_or(at),
            );
            let changed = *end != at;
            *end = at;
            changed
        }
        (Some(_), _) => false,
        (None, _) => {
            let hover = match point.x < SIDEBAR_WIDTH || view.mode != Mode::Month {
                true => hit_at(view, point).filter(|hit| {
                    matches!(
                        hit,
                        Hit::Calendar(_) | Hit::MiniShift(_) | Hit::Event(_, Part::End)
                    )
                }),
                false => None,
            };
            let changed = hover != view.hover;
            view.hover = hover;
            view.ui.handle(event) | changed
        }
    }
}

fn pointer_up(view: &mut CalendarView, pointer: &Pointer, event: &Event) -> bool {
    view.press = None;
    let Some(drag) = view.drag.take() else {
        return view.ui.handle(event);
    };
    let _ = pointer;
    match drag {
        Drag::Create {
            anchor,
            current,
            all_day,
        } if anchor != current => {
            let (start, end) = (anchor.min(current), anchor.max(current));
            let end = match all_day {
                true => end,
                false => end.max(start.checked_add(15.minutes()).unwrap_or(end)),
            };
            create(view, start, end, all_day);
            true
        }
        Drag::Create { .. } => true,
        Drag::Move {
            found,
            grab,
            current,
            all_day,
            moved: true,
        } if grab != current || all_day != found.occurrence.all_day => {
            let (start, end) = shifted(
                grab,
                current,
                found.occurrence.start.datetime(),
                found.occurrence.end.datetime(),
            );
            let (start, end) = match (found.occurrence.all_day, all_day) {
                (false, true) => (
                    start.date().to_datetime(Time::midnight()),
                    start
                        .date()
                        .tomorrow()
                        .unwrap_or(start.date())
                        .to_datetime(Time::midnight()),
                ),
                (true, false) => (current, current.checked_add(1.hour()).unwrap_or(current)),
                _ => (start, end),
            };
            reschedule(view, found, start, end, all_day);
            true
        }
        Drag::Move { .. } => true,
        Drag::Resize { found, end } if end != found.occurrence.end.datetime() => {
            let start = found.occurrence.start.datetime();
            reschedule(view, found, start, end, false);
            true
        }
        Drag::Resize { .. } => true,
    }
}

fn reschedule(
    view: &mut CalendarView,
    found: Found,
    start: DateTime,
    end: DateTime,
    all_day: bool,
) {
    match found.occurrence.recurring {
        true => view.ask_scope(
            Confirm::Reschedule {
                found,
                start,
                end,
                all_day,
            },
            &L10N.tr("change-recurring-title"),
        ),
        false => view.apply_reschedule(&found, start, end, all_day, Scope::Series),
    }
}

fn create(view: &mut CalendarView, start: DateTime, end: DateTime, all_day: bool) {
    let Some(calendar) = view.default_calendar() else {
        return;
    };
    view.drag = None;
    let mut object = new_event(start, end, all_day, "", &view.zone);
    let alert = match all_day {
        true => view.settings.all_day_alert,
        false => view.settings.timed_alert,
    };
    if let Some(event) = object.events.first_mut() {
        event.alarms = alert.alarms(&[]);
    }
    let Some(event) = object.events.first() else {
        return;
    };
    let selection = super::Selection {
        calendar: view.library.calendars[calendar].info.id.clone(),
        uid: event.uid.clone(),
        key: event.start.key(),
    };
    if view.library.put_object(calendar, object).is_some() {
        view.selected = Some(selection.clone());
        view.pending_open = Some(selection);
        view.requests.push(Request::Changed);
    }
}

fn new_event_now(view: &mut CalendarView) {
    let now = view.now.datetime();
    let day = view.focus;
    let start = match day == now.date() {
        true => now
            .date()
            .to_datetime(Time::constant(0, 0, 0, 0))
            .checked_add(i64::from(now.hour() + 1).hours())
            .unwrap_or(now),
        false => day.to_datetime(Time::constant(9, 0, 0, 0)),
    };
    let end = start.checked_add(1.hour()).unwrap_or(start);
    create(view, start, end, false);
    if view.mode == Mode::Year {
        view.set_mode(Mode::Day);
    }
}

pub fn signal(view: &mut CalendarView, signal: Signal) {
    if let Some(inspector) = &mut view.inspector {
        inspector.apply(&signal);
    }
    if let Some(setup) = &mut view.setup {
        setup.apply(&signal);
    }
    if let Some(naming) = &mut view.naming {
        naming.apply(&signal);
    }
    match signal {
        Signal::Today => {
            let today = view.now.date();
            view.close_inspector(true);
            view.go_to(today);
        }
        Signal::Previous => view.shift(-1),
        Signal::Next => view.shift(1),
        Signal::Mode(index) => {
            if let Some(mode) = Mode::ALL.get(index) {
                view.set_mode(*mode);
            }
        }
        Signal::Search(text) => {
            view.search = text;
            view.refresh_search();
        }
        Signal::NewEvent => new_event_now(view),
        Signal::AddAccount => view.open_account_setup(),
        Signal::DeleteEvent => {
            if let Some(found) = view
                .inspector
                .as_ref()
                .map(|inspector| inspector.found.clone())
            {
                view.ask_delete(found);
            }
        }
        Signal::Choice(action) => choose(view, &action),
        Signal::SetupSubmit => submit_setup(view),
        Signal::NameSubmit => view.submit_naming(),
        Signal::NameCancel => view.cancel_naming(),
        Signal::SetupCancel => {
            if let Some(setup) = view.setup.take() {
                setup.dismiss();
            }
        }
        _ => {}
    }
}

fn submit_setup(view: &mut CalendarView) {
    let Some(setup) = &mut view.setup else {
        return;
    };
    if setup.busy {
        return;
    }
    if setup.password.is_none() && setup.collect_password(&mut view.ui) {
        return;
    }
    let password = setup.password.clone().unwrap_or_default();
    let request = match setup.kind {
        super::setup::Kind::CalDav if setup.user.is_empty() => None,
        super::setup::Kind::CalDav => Some(Request::AddAccount {
            server: setup.server.clone(),
            user: setup.user.clone(),
            password,
        }),
        super::setup::Kind::Subscription if setup.server.is_empty() => None,
        super::setup::Kind::Subscription => Some(Request::Subscribe(setup.server.clone())),
    };
    match request {
        Some(request) => {
            setup.busy = true;
            setup.show_note(&mut view.ui, &L10N.tr("syncing"));
            view.requests.push(request);
        }
        None => setup.password = None,
    }
}

fn choose(view: &mut CalendarView, action: &str) {
    let Some(confirm) = view.confirm.take() else {
        return;
    };
    let scope = match action {
        "occurrence" => Scope::Occurrence,
        "series" | "accept" => Scope::Series,
        _ => return,
    };
    match confirm {
        Confirm::Reschedule {
            found,
            start,
            end,
            all_day,
        } => view.apply_reschedule(&found, start, end, all_day, scope),
        Confirm::Details {
            found,
            details,
            calendar,
        } => view.apply_details(&found, &details, calendar, scope),
        Confirm::Delete(found) => view.apply_delete(&found, scope),
        Confirm::RemoveAccount(id) => view.requests.push(Request::RemoveAccount(id)),
        Confirm::Import(objects, calendar) => {
            for object in objects {
                view.library.put_object(calendar, object);
            }
            view.requests.push(Request::Changed);
        }
        Confirm::Subscribe(url) => view.requests.push(Request::Subscribe(url)),
        Confirm::DeleteCalendar(index) => view.delete_calendar(index),
    }
}
