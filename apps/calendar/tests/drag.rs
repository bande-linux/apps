#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use bande_calendar::store::Pending;
use bande_calendar::ui::geometry::{HOUR_HEIGHT, TimeGrid};
use bande_calendar::ui::{CalendarView, SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
use bande_toolkit::ui::{Event, Pointer};
use bande_toolkit::{Point, Rect, SHEET_INSET, Scheme};
use jiff::civil::{DateTime, date};

const SIZE: (u32, u32) = (1120, 740);

fn grid() -> TimeGrid {
    let inset = SHEET_INSET as f32;
    TimeGrid {
        area: Rect::new(
            SIDEBAR_WIDTH,
            TOOLBAR_HEIGHT,
            SIZE.0 as f32 - SIDEBAR_WIDTH - inset,
            SIZE.1 as f32 - TOOLBAR_HEIGHT - inset,
        ),
        first: date(2026, 10, 26),
        days: 7,
        all_day_rows: 2,
        scroll: 7.5 * HOUR_HEIGHT,
    }
}

fn point(at: DateTime) -> Point {
    let grid = grid();
    let column = grid.day_index(at.date()).unwrap();
    let minutes = i32::from(at.hour()) * 60 + i32::from(at.minute());
    Point::new(
        grid.column(column).x + 20.0,
        grid.y_for_minutes(minutes) + 2.0,
    )
}

fn drag(view: &mut CalendarView, from: DateTime, to: DateTime) {
    let (from, to) = (point(from), point(to));
    view.handle(&Event::PointerDown(Pointer::at(from)));
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        (from.x + to.x) / 2.0,
        (from.y + to.y) / 2.0,
    ))));
    view.handle(&Event::PointerMove(Pointer::at(to)));
    view.handle(&Event::PointerUp(Pointer::at(to)));
    view.scene(SIZE.0, SIZE.1);
}

fn starts(view: &CalendarView, summary: &str) -> Vec<String> {
    view.library()
        .calendars
        .iter()
        .flat_map(|calendar| &calendar.objects)
        .filter_map(|stored| stored.parsed.as_ref()?.master())
        .filter(|event| event.summary == summary)
        .map(|event| {
            format!(
                "{}-{}",
                event.start.wall().strftime("%a %H:%M"),
                event.end.wall().strftime("%H:%M")
            )
        })
        .collect()
}

fn ready() -> (tempfile::TempDir, CalendarView) {
    let directory = tempfile::tempdir().unwrap();
    let mut view = fixture::view(directory.path(), Scheme::Light);
    view.scene(SIZE.0, SIZE.1);
    (directory, view)
}

#[test]
fn dragging_an_event_moves_it_to_another_day_and_time() {
    let (_directory, mut view) = ready();
    drag(
        &mut view,
        date(2026, 10, 26).at(11, 30, 0, 0),
        date(2026, 10, 27).at(9, 0, 0, 0),
    );
    assert_eq!(starts(&view, "Design review"), ["Tue 08:30-10:00"]);
    let pending = view
        .library()
        .calendars
        .iter()
        .flat_map(|calendar| &calendar.objects)
        .find(|stored| stored.data.contains("SUMMARY:Design review"))
        .and_then(|stored| stored.pending);
    assert_eq!(pending, Some(Pending::Create));
}

#[test]
fn dragging_the_bottom_edge_changes_the_duration() {
    let (_directory, mut view) = ready();
    let grid = grid();
    let edge = Point::new(
        grid.column(0).x + 20.0,
        grid.y_for_minutes(12 * 60 + 30) - 3.0,
    );
    let target = point(date(2026, 10, 26).at(13, 15, 0, 0));
    view.handle(&Event::PointerDown(Pointer::at(edge)));
    view.handle(&Event::PointerMove(Pointer::at(target)));
    view.handle(&Event::PointerUp(Pointer::at(target)));
    assert_eq!(starts(&view, "Design review"), ["Mon 11:00-13:15"]);
}

#[test]
fn dragging_on_empty_time_creates_an_event() {
    let (_directory, mut view) = ready();
    let before = view
        .library()
        .calendars
        .iter()
        .map(|calendar| calendar.objects.len())
        .sum::<usize>();
    drag(
        &mut view,
        date(2026, 10, 29).at(8, 0, 0, 0),
        date(2026, 10, 29).at(9, 30, 0, 0),
    );
    let after = view
        .library()
        .calendars
        .iter()
        .map(|calendar| calendar.objects.len())
        .sum::<usize>();
    assert_eq!(after, before + 1);
    assert_eq!(starts(&view, ""), ["Thu 08:00-09:30"]);
}

#[test]
fn a_click_without_a_drag_changes_nothing() {
    let (_directory, mut view) = ready();
    let at = point(date(2026, 10, 26).at(11, 30, 0, 0));
    view.handle(&Event::PointerDown(Pointer::at(at)));
    view.handle(&Event::PointerUp(Pointer::at(at)));
    assert_eq!(starts(&view, "Design review"), ["Mon 11:00-12:30"]);
}

#[test]
fn the_calendar_menu_recolours_a_calendar() {
    let (_directory, mut view) = ready();
    let work = view.library().calendar_index("work").unwrap();
    assert!(view.menu_action(&format!("calendar-color:{work}:6")));
    let color = view.library().calendars[work].info.color.clone();
    assert_ne!(color, "#3478F6");
    assert!(view.take_requests().iter().any(|request| matches!(
        request,
        bande_calendar::ui::Request::Collection { change: bande_calendar::worker::Change::Color(sent), .. } if *sent == format!("{color}FF")
    )));
}

#[test]
fn dragging_in_the_month_view_moves_an_event_by_days_and_keeps_its_time() {
    let (_directory, mut view) = ready();
    view.set_mode(bande_calendar::ui::Mode::Month);
    view.scene(SIZE.0, SIZE.1);
    let inset = SHEET_INSET as f32;
    let area = Rect::new(
        SIDEBAR_WIDTH,
        TOOLBAR_HEIGHT,
        SIZE.0 as f32 - SIDEBAR_WIDTH - inset,
        SIZE.1 as f32 - TOOLBAR_HEIGHT - inset,
    );
    let cell = |week: usize, day: usize| {
        bande_calendar::ui::geometry::month_cell(area, 28.0, 6, week, day)
    };
    let dentist = cell(4, 3);
    let from = Point::new(dentist.x + 30.0, dentist.y + 26.0 + 18.0 * 1.0 + 8.0);
    let saturday = cell(4, 5);
    let to = Point::new(saturday.x + 30.0, saturday.y + 60.0);
    view.handle(&Event::PointerDown(Pointer::at(from)));
    view.handle(&Event::PointerMove(Pointer::at(to)));
    view.handle(&Event::PointerUp(Pointer::at(to)));
    assert_eq!(starts(&view, "Dentist"), ["Sat 16:00-17:30"]);
}

fn press(view: &mut CalendarView, key: bande_toolkit::ui::Key) {
    view.handle(&Event::Key {
        key,
        modifiers: bande_toolkit::ui::Modifiers::default(),
    });
    view.scene(SIZE.0, SIZE.1);
}

fn type_text(view: &mut CalendarView, text: &str) {
    for character in text.chars() {
        press(view, bande_toolkit::ui::Key::Character(character));
    }
}

fn names(view: &CalendarView) -> Vec<String> {
    view.library()
        .calendars
        .iter()
        .map(|calendar| calendar.info.name.clone())
        .collect()
}

#[test]
fn calendars_are_created_renamed_and_deleted_from_the_menus() {
    use bande_toolkit::ui::Key;
    let (_directory, mut view) = ready();
    assert!(view.menu_action("new-calendar:local"));
    view.scene(SIZE.0, SIZE.1);
    type_text(&mut view, "Gym");
    press(&mut view, Key::Enter);
    assert!(!view.has_sheet());
    assert!(names(&view).contains(&"Gym".to_owned()));
    let gym = names(&view).iter().position(|name| name == "Gym").unwrap();

    assert!(view.menu_action(&format!("calendar-rename:{gym}")));
    view.scene(SIZE.0, SIZE.1);
    for _ in 0..3 {
        press(&mut view, Key::Backspace);
    }
    type_text(&mut view, "Sport");
    press(&mut view, Key::Enter);
    assert_eq!(view.library().calendars[gym].info.name, "Sport");

    let work = view.library().calendar_index("work").unwrap();
    assert!(view.menu_action(&format!("calendar-rename:{work}")));
    view.scene(SIZE.0, SIZE.1);
    type_text(&mut view, " 2");
    press(&mut view, Key::Enter);
    let requests = view.take_requests();
    assert!(requests.iter().any(|request| matches!(
        request,
        bande_calendar::ui::Request::Collection { change: bande_calendar::worker::Change::Rename(name), .. } if name == "Work 2"
    )));

    assert!(view.menu_action(&format!("calendar-delete:{gym}")));
    view.scene(SIZE.0, SIZE.1);
    assert!(view.has_sheet());
    press(&mut view, Key::Space);
    assert!(!names(&view).contains(&"Sport".to_owned()));
}
