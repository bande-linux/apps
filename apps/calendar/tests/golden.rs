#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use bande_calendar::ui::{CalendarView, Mode, SIDEBAR_WIDTH};
use fixture::view;
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{CpuRenderer, FrameRegions, Palette, Scheme, framed_scene};

const SIZE: (u32, u32) = (1120, 740);

fn render(name: &str, view: &mut CalendarView, scheme: Scheme) {
    view.scene(SIZE.0, SIZE.1);
    let content = view.scene(SIZE.0, SIZE.1);
    let scene = framed_scene(
        &content,
        &view.title(),
        Some(FrameRegions {
            sidebar: SIDEBAR_WIDTH,
            toolbar: 0.0,
        }),
        Some(view.header()),
        &Palette::for_scheme(scheme),
    );
    let frame = CpuRenderer::new().render(&scene).expect("a frame");
    unbound_golden::assert_golden_rgba!(name, SIZE, frame.rgba);
}

#[test]
fn each_view_matches_its_golden() {
    for (scheme, suffix) in [(Scheme::Light, ""), (Scheme::Dark, "-dark")] {
        for (mode, name) in [
            (Mode::Week, "week"),
            (Mode::Day, "day"),
            (Mode::Month, "month"),
            (Mode::Year, "year"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut view = view(directory.path(), scheme);
            view.set_mode(mode);
            render(&format!("{name}{suffix}"), &mut view, scheme);
        }
    }
}

#[test]
fn a_new_event_opens_the_inspector() {
    let directory = tempfile::tempdir().unwrap();
    let mut view = view(directory.path(), Scheme::Light);
    view.scene(SIZE.0, SIZE.1);
    assert!(view.menu_action("new-event"));
    render("inspector", &mut view, Scheme::Light);
}

#[test]
fn search_lists_matching_events() {
    let directory = tempfile::tempdir().unwrap();
    let mut view = view(directory.path(), Scheme::Light);
    view.set_mode(Mode::Month);
    view.scene(SIZE.0, SIZE.1);
    assert!(view.menu_action("find"));
    for character in "review".chars() {
        view.handle(&Event::Key {
            key: Key::Character(character),
            modifiers: Modifiers::default(),
        });
    }
    render("search", &mut view, Scheme::Light);
}

fn press(view: &mut CalendarView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn the_inspector_names_a_new_event_and_saves_it_on_escape() {
    let directory = tempfile::tempdir().unwrap();
    let mut view = view(directory.path(), Scheme::Light);
    view.scene(SIZE.0, SIZE.1);
    view.menu_action("new-event");
    view.scene(SIZE.0, SIZE.1);
    for character in "Team lunch".chars() {
        press(&mut view, Key::Character(character));
    }
    press(&mut view, Key::Tab);
    for character in "Cafe".chars() {
        press(&mut view, Key::Character(character));
    }
    press(&mut view, Key::Escape);
    view.scene(SIZE.0, SIZE.1);
    let found = view
        .library()
        .calendars
        .iter()
        .flat_map(|calendar| &calendar.objects)
        .filter_map(|stored| stored.parsed.as_ref()?.master())
        .find(|event| event.summary == "Team lunch")
        .map(|event| event.location.clone());
    assert_eq!(found.as_deref(), Some("Cafe"));
}
