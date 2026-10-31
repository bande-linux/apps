#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_calendar::edit::{Alert, new_event};
use bande_calendar::ical::{DaySpec, Frequency, Rule};
use bande_calendar::store::{Account, AccountKind, CalendarInfo, LOCAL_ACCOUNT, Library};
use bande_calendar::ui::{CalendarView, Mode, SIDEBAR_WIDTH};
use jiff::civil::{Weekday, date};
use jiff::tz::TimeZone;
use unbound_datetime::{DateFormats, posix_locale};
use unbound_toolkit::{
    Appearance, CpuRenderer, FrameRegions, Palette, Scheme, TextSystem, framed_scene,
};

const SIZE: (u32, u32) = (1120, 740);

fn library(directory: &std::path::Path, zone: &TimeZone) -> Library {
    let mut library = Library::load(directory.to_owned());
    library.add_account(
        Account {
            id: "work".to_owned(),
            name: "jane@example.com".to_owned(),
            kind: AccountKind::CalDav {
                server: "https://dav.example.com".to_owned(),
                user: "jane@example.com".to_owned(),
                home: "https://dav.example.com/jane/".to_owned(),
            },
        },
        ["Work", "Family"]
            .iter()
            .zip(["#3478F6", "#34A853"])
            .map(|(name, color)| CalendarInfo {
                id: name.to_lowercase(),
                account: "work".to_owned(),
                url: Some(format!(
                    "https://dav.example.com/jane/{}/",
                    name.to_lowercase()
                )),
                name: (*name).to_owned(),
                color: color.to_owned(),
                read_only: false,
                visible: true,
                ctag: None,
                sync_token: None,
            })
            .collect(),
    );
    let local = library
        .calendars
        .iter()
        .position(|calendar| calendar.info.account == LOCAL_ACCOUNT)
        .unwrap();
    library.calendars[local].info.color = "#E8590C".to_owned();
    let work = library.calendar_index("work").unwrap();
    let family = library.calendar_index("family").unwrap();
    let mut standup = new_event(
        date(2026, 10, 19).at(9, 30, 0, 0),
        date(2026, 10, 19).at(9, 45, 0, 0),
        false,
        "Standup",
        zone,
    );
    let master = standup.master_mut().unwrap();
    master.rule = Some(Rule {
        by_day: [
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
        ]
        .into_iter()
        .map(|weekday| DaySpec {
            ordinal: None,
            weekday,
        })
        .collect(),
        ..Rule::new(Frequency::Weekly)
    });
    master.alarms = Alert::Minutes(5).alarms(&[]);
    library.put_object(work, standup);
    for (day, start, end, title, location, calendar) in [
        (26, (11, 0), (12, 30), "Design review", "Room 4", work),
        (27, (14, 0), (15, 0), "1:1 with Sam", "", work),
        (27, (14, 30), (16, 0), "Release planning", "", work),
        (
            28,
            (12, 0),
            (13, 0),
            "Lunch with Alex",
            "Café Lumen",
            family,
        ),
        (29, (16, 0), (17, 30), "Dentist", "Main St 12", local),
        (30, (18, 30), (21, 0), "Dinner", "", family),
        (11, (10, 0), (11, 0), "Quarterly review", "", work),
    ] {
        let month = if day == 11 { 11 } else { 10 };
        let mut object = new_event(
            date(2026, month, day).at(start.0, start.1, 0, 0),
            date(2026, month, day).at(end.0, end.1, 0, 0),
            false,
            title,
            zone,
        );
        object.master_mut().unwrap().location = location.to_owned();
        library.put_object(calendar, object);
    }
    library.put_object(
        family,
        new_event(
            date(2026, 10, 30).at(0, 0, 0, 0),
            date(2026, 11, 2).at(0, 0, 0, 0),
            true,
            "Autumn break",
            zone,
        ),
    );
    library.put_object(
        local,
        new_event(
            date(2026, 10, 31).at(0, 0, 0, 0),
            date(2026, 11, 1).at(0, 0, 0, 0),
            true,
            "Halloween",
            zone,
        ),
    );
    library
}

fn view(directory: &std::path::Path, scheme: Scheme) -> CalendarView {
    let zone = TimeZone::get("Europe/Berlin").unwrap();
    let now = date(2026, 10, 28)
        .at(10, 20, 0, 0)
        .to_zoned(zone.clone())
        .unwrap();
    let formats = DateFormats::for_locale(&posix_locale("en_GB.UTF-8"));
    let mut view = CalendarView::new(
        library(directory, &zone),
        Palette::for_scheme(scheme),
        TextSystem::new(),
        formats,
        zone,
        now,
    );
    view.set_appearance(Appearance {
        scheme,
        ..Appearance::default()
    });
    view
}

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
