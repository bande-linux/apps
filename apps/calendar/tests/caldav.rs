#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use bande_calendar::caldav::{Client, Credentials, Error};
use bande_calendar::ical::{CalendarObject, Event, Moment};
use bande_calendar::store::{Account, AccountKind, CalendarInfo, Library, Pending};
use bande_calendar::sync::{apply, sync_calendar};
use jiff::civil::date;
use jiff::tz::TimeZone;
use support::{PASSWORD, Radicale, USER};

fn library_for(radicale: &Radicale, directory: &std::path::Path) -> (Library, usize) {
    let found = radicale.client().discover(&radicale.url, USER).unwrap();
    let mut library = Library::load(directory.to_owned());
    let account = Account {
        id: "radicale".to_owned(),
        name: "Radicale".to_owned(),
        kind: AccountKind::CalDav {
            server: radicale.url.clone(),
            user: USER.to_owned(),
            home: found.home.clone(),
        },
    };
    let calendars = found
        .calendars
        .iter()
        .map(|remote| CalendarInfo {
            id: remote.url.clone(),
            account: account.id.clone(),
            url: Some(remote.url.clone()),
            name: remote.name.clone(),
            color: remote.color.clone().unwrap_or_default(),
            read_only: remote.read_only,
            visible: true,
            ctag: None,
            sync_token: None,
        })
        .collect();
    library.add_account(account, calendars);
    let index = library
        .calendars
        .iter()
        .position(|calendar| calendar.info.name == "Work")
        .unwrap();
    (library, index)
}

fn sync(radicale: &Radicale, library: &mut Library, index: usize) -> Vec<String> {
    let outcome = sync_calendar(&radicale.client(), &library.calendars[index]).unwrap();
    apply(library, outcome, false)
}

fn meeting(uid: &str, summary: &str) -> CalendarObject {
    let zone = TimeZone::get("Europe/Berlin").unwrap();
    let start = date(2026, 11, 2).at(10, 0, 0, 0).to_zoned(zone).unwrap();
    let end = start.checked_add(jiff::Span::new().hours(1)).unwrap();
    CalendarObject::new(Event::new(
        uid.to_owned(),
        summary,
        Moment::in_zone(start),
        Moment::in_zone(end),
    ))
}

fn summaries(library: &Library, index: usize) -> Vec<String> {
    let mut found: Vec<String> = library.calendars[index]
        .objects
        .iter()
        .filter_map(|stored| Some(stored.parsed.as_ref()?.master()?.summary.clone()))
        .collect();
    found.sort();
    found
}

#[test]
fn discovery_finds_the_principal_home_and_calendars() {
    let radicale = Radicale::start();
    let work = radicale.calendar("work", "Work");
    radicale.calendar("home", "Home");
    let found = radicale.client().discover(&radicale.url, USER).unwrap();
    assert_eq!(found.principal, format!("{}{USER}/", radicale.url));
    assert_eq!(found.home, format!("{}{USER}/", radicale.url));
    let mut names: Vec<&str> = found
        .calendars
        .iter()
        .map(|calendar| calendar.name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["Home", "Work"]);
    let calendar = found
        .calendars
        .iter()
        .find(|calendar| calendar.url == work)
        .unwrap();
    assert_eq!(calendar.color.as_deref(), Some("#FF9500FF"));
    assert!(!calendar.read_only);
    assert!(calendar.sync_token.is_some());

    let wrong = Client::new(Some(&Credentials {
        user: USER.to_owned(),
        password: format!("{PASSWORD}!"),
    }));
    assert_eq!(
        wrong.discover(&radicale.url, USER),
        Err(Error::Unauthorized)
    );
}

#[test]
fn two_devices_sync_creates_edits_and_deletes() {
    let radicale = Radicale::start();
    radicale.calendar("work", "Work");
    let (first_dir, second_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (mut first, work) = library_for(&radicale, first_dir.path());
    let (mut second, other) = library_for(&radicale, second_dir.path());

    first.put_object(work, meeting("m1", "Planning"));
    first.put_object(work, meeting("m2", "Retro"));
    assert!(sync(&radicale, &mut first, work).is_empty());
    assert!(
        first.calendars[work]
            .objects
            .iter()
            .all(|stored| stored.pending.is_none() && stored.etag.is_some())
    );
    let token = first.calendars[work].info.sync_token.clone();
    assert!(token.is_some());

    sync(&radicale, &mut second, other);
    assert_eq!(summaries(&second, other), ["Planning", "Retro"]);
    let local = TimeZone::get("Europe/Berlin").unwrap();
    let day = date(2026, 11, 2).to_zoned(local.clone()).unwrap();
    let shown = second.occurrences(day.timestamp(), day.tomorrow().unwrap().timestamp(), &local);
    assert_eq!(shown.len(), 2);
    assert_eq!(
        shown[0].occurrence.start.strftime("%H:%M").to_string(),
        "10:00"
    );

    let index = second.calendars[other].find("m1").unwrap();
    let mut edited = second.calendars[other].objects[index]
        .parsed
        .clone()
        .unwrap();
    edited.master_mut().unwrap().summary = "Planning (room 4)".to_owned();
    second.put_object(other, edited);
    let index = second.calendars[other].find("m2").unwrap();
    second.remove_object(other, index);
    assert_eq!(
        second.calendars[other].objects[index].pending,
        Some(Pending::Delete)
    );
    sync(&radicale, &mut second, other);
    assert_eq!(summaries(&second, other), ["Planning (room 4)"]);

    sync(&radicale, &mut first, work);
    assert_eq!(summaries(&first, work), ["Planning (room 4)"]);
    assert_ne!(first.calendars[work].info.sync_token, token);

    let reloaded = Library::load(first_dir.path().to_owned());
    let work = reloaded
        .calendar_index(&first.calendars[work].info.id)
        .unwrap();
    assert_eq!(summaries(&reloaded, work), ["Planning (room 4)"]);
}

#[test]
fn a_stale_edit_gets_412_and_keeps_the_server_copy() {
    let radicale = Radicale::start();
    radicale.calendar("work", "Work");
    let (first_dir, second_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (mut first, work) = library_for(&radicale, first_dir.path());
    let (mut second, other) = library_for(&radicale, second_dir.path());
    first.put_object(work, meeting("m1", "Budget"));
    sync(&radicale, &mut first, work);
    sync(&radicale, &mut second, other);

    let index = second.calendars[other].find("m1").unwrap();
    let mut theirs = second.calendars[other].objects[index]
        .parsed
        .clone()
        .unwrap();
    theirs.master_mut().unwrap().summary = "Budget (moved to Friday)".to_owned();
    second.put_object(other, theirs);
    sync(&radicale, &mut second, other);

    let index = first.calendars[work].find("m1").unwrap();
    let mut mine = first.calendars[work].objects[index].parsed.clone().unwrap();
    mine.master_mut().unwrap().summary = "Budget (cancelled?)".to_owned();
    first.put_object(work, mine);
    let conflicts = sync(&radicale, &mut first, work);
    assert_eq!(conflicts, ["Budget (cancelled?)"]);
    assert_eq!(summaries(&first, work), ["Budget (moved to Friday)"]);
    assert!(
        first.calendars[work]
            .objects
            .iter()
            .all(|stored| stored.pending.is_none())
    );

    let index = second.calendars[other].find("m1").unwrap();
    second.calendars[other].objects[index].etag = Some("\"stale\"".to_owned());
    second.remove_object(other, index);
    let conflicts = sync(&radicale, &mut second, other);
    assert_eq!(conflicts, ["Budget (moved to Friday)"]);
    assert_eq!(summaries(&second, other), ["Budget (moved to Friday)"]);
}

#[test]
fn an_invalid_sync_token_falls_back_to_a_full_sync() {
    let radicale = Radicale::start();
    radicale.calendar("work", "Work");
    let directory = tempfile::tempdir().unwrap();
    let (mut library, work) = library_for(&radicale, directory.path());
    library.put_object(work, meeting("m1", "Kept"));
    sync(&radicale, &mut library, work);
    library.calendars[work].objects[0].data = library.calendars[work].objects[0]
        .data
        .replace("Kept", "Stale");
    library.calendars[work].objects[0].etag = Some("\"old\"".to_owned());
    library.calendars[work].info.sync_token =
        Some("http://radicale.org/ns/sync/unknown".to_owned());
    sync(&radicale, &mut library, work);
    assert!(
        library.calendars[work].objects[0]
            .data
            .contains("SUMMARY:Kept")
    );
}

#[test]
fn an_edit_made_while_the_create_is_in_flight_is_pushed_as_an_update() {
    let radicale = Radicale::start();
    radicale.calendar("work", "Work");
    let directory = tempfile::tempdir().unwrap();
    let (mut library, work) = library_for(&radicale, directory.path());
    library.put_object(work, meeting("m1", ""));
    let snapshot = library.calendars[work].clone();
    let index = library.calendars[work].find("m1").unwrap();
    let mut named = library.calendars[work].objects[index]
        .parsed
        .clone()
        .unwrap();
    named.master_mut().unwrap().summary = "Team lunch".to_owned();
    library.put_object(work, named);
    let outcome = sync_calendar(&radicale.client(), &snapshot).unwrap();
    assert!(apply(&mut library, outcome, false).is_empty());
    assert!(sync(&radicale, &mut library, work).is_empty());
    assert!(
        library.calendars[work]
            .objects
            .iter()
            .all(|stored| stored.pending.is_none())
    );
    let (mut other, other_work) = library_for(&radicale, tempfile::tempdir().unwrap().path());
    sync(&radicale, &mut other, other_work);
    assert_eq!(summaries(&other, other_work), ["Team lunch"]);
}

#[test]
fn a_create_that_already_reached_the_server_is_sent_again_as_an_update() {
    let radicale = Radicale::start();
    radicale.calendar("work", "Work");
    let directory = tempfile::tempdir().unwrap();
    let (mut library, work) = library_for(&radicale, directory.path());
    library.put_object(work, meeting("m1", ""));
    let snapshot = library.calendars[work].clone();
    sync_calendar(&radicale.client(), &snapshot).unwrap();
    let index = library.calendars[work].find("m1").unwrap();
    let mut named = library.calendars[work].objects[index]
        .parsed
        .clone()
        .unwrap();
    named.master_mut().unwrap().summary = "Team lunch".to_owned();
    library.put_object(work, named);
    assert_eq!(
        library.calendars[work].objects[index].pending,
        Some(Pending::Create)
    );
    assert!(sync(&radicale, &mut library, work).is_empty());
    assert_eq!(summaries(&library, work), ["Team lunch"]);
    assert!(
        library.calendars[work]
            .objects
            .iter()
            .all(|stored| stored.pending.is_none())
    );
}

#[test]
fn a_new_colour_is_written_to_the_server() {
    let radicale = Radicale::start();
    let work = radicale.calendar("work", "Work");
    radicale.client().set_color(&work, "#AF52DEFF").unwrap();
    let found = radicale.client().discover(&radicale.url, USER).unwrap();
    assert_eq!(found.calendars[0].color.as_deref(), Some("#AF52DEFF"));
}

#[test]
fn calendars_are_created_renamed_and_deleted_on_the_server() {
    let radicale = Radicale::start();
    radicale.calendar("work", "Work");
    let client = radicale.client();
    let found = client.discover(&radicale.url, USER).unwrap();
    let trips = format!("{}trips/", found.home);
    client.make_calendar(&trips, "Trips", "#34C759FF").unwrap();
    client.rename(&trips, "Travel").unwrap();
    let names = |client: &Client| {
        let mut names: Vec<String> = client
            .calendars(&found.home)
            .unwrap()
            .into_iter()
            .map(|calendar| calendar.name)
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&client), ["Travel", "Work"]);
    client.delete(&trips, None).unwrap();
    assert_eq!(names(&client), ["Work"]);
}
