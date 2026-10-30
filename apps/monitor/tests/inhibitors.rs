#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use fixture::process;
use unbound_monitor::{
    Catalog, Inhibitor, Monitor, SleepBlocker, Snapshot, Tab, list_sleep_blockers, sleep_blockers,
};
use unbound_test_bus::SessionBus;

fn inhibitor(what: &str, who: &str, why: &str, mode: &str, pid: u32) -> Inhibitor {
    (
        what.to_owned(),
        who.to_owned(),
        why.to_owned(),
        mode.to_owned(),
        1000,
        pid,
    )
}

struct FakeManager {
    inhibitors: Vec<Inhibitor>,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    fn list_inhibitors(&self) -> Vec<Inhibitor> {
        self.inhibitors.clone()
    }
}

#[test]
fn only_blocking_sleep_and_idle_inhibitors_keep_the_computer_awake() {
    assert_eq!(
        sleep_blockers(vec![
            inhibitor("shutdown:sleep", "Videos", "Playing", "block", 10),
            inhibitor("idle", "Backup", "Copying files", "block", 11),
            inhibitor("sleep", "NetworkManager", "", "delay", 12),
            inhibitor("handle-lid-switch", "unbound-power", "", "block", 13),
        ]),
        [
            SleepBlocker {
                pid: 10,
                who: "Videos".to_owned(),
                why: "Playing".to_owned()
            },
            SleepBlocker {
                pid: 11,
                who: "Backup".to_owned(),
                why: "Copying files".to_owned()
            },
        ]
    );
}

#[test]
fn a_sleep_blocker_from_logind_marks_its_app_in_the_energy_tab() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let logind = bus.connect().expect("logind connection");
    logind
        .object_server()
        .at(
            "/org/freedesktop/login1",
            FakeManager {
                inhibitors: vec![
                    inhibitor("sleep:idle", "Videos", "Playing a video", "block", 500),
                    inhibitor("sleep", "NetworkManager", "", "delay", 600),
                ],
            },
        )
        .expect("manager");
    logind
        .request_name("org.freedesktop.login1")
        .expect("logind name");
    let blockers = list_sleep_blockers(&bus.connect().expect("client")).expect("inhibitors");

    let mut snapshot = Snapshot {
        seconds: 2.0,
        battery: true,
        processes: vec![
            process(500, 1, "/usr/bin/videos"),
            process(501, 500, "/usr/bin/videos-decoder"),
            process(600, 1, "/usr/sbin/NetworkManager"),
        ],
        ..Snapshot::default()
    };
    snapshot.mark_sleep_blockers(&blockers);
    assert_eq!(
        snapshot.processes[0].preventing_sleep.as_deref(),
        Some("Playing a video")
    );
    let mut monitor = Monitor::new(Catalog::new(&[]), 1000);
    monitor.update(snapshot);
    monitor.set_tab(Tab::Energy);
    let column = monitor
        .columns()
        .iter()
        .position(|column| *column == unbound_monitor::Column::PreventingSleep)
        .expect("a Preventing Sleep column");
    let rows = monitor
        .rows()
        .iter()
        .map(|row| (row.name.clone(), row.cells[column].clone()))
        .collect::<Vec<_>>();
    assert!(
        rows.contains(&("videos".to_owned(), "Yes".to_owned())),
        "{rows:?}"
    );
    assert!(rows.contains(&("NetworkManager".to_owned(), "No".to_owned())));
}
