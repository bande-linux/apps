#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use fixture::{entry, process};
use unbound_monitor::{
    Catalog, Column, GroupKey, Interface, Monitor, Process, Rates, Row, RowKey, Snapshot, Tab,
    format_bytes, format_cpu_time, format_rate,
};

fn catalog() -> Catalog {
    Catalog::new(&[
        entry("org.unbound.Terminal", "Terminal", "unbound-term", false),
        entry("org.unbound.Files", "Files", "unbound-files", false),
    ])
}

fn busy(mut process: Process, cpu_percent: f32) -> Process {
    process.cpu_percent = cpu_percent;
    process
}

fn snapshot(processes: Vec<Process>) -> Snapshot {
    Snapshot {
        seconds: 2.0,
        processes,
        ..Snapshot::default()
    }
}

fn desk(terminal: f32, cargo: f32, files: f32) -> Snapshot {
    snapshot(vec![
        busy(process(500, 1, "/usr/bin/unbound-term"), terminal),
        busy(process(501, 500, "/bin/bash"), 0.5),
        busy(process(502, 501, "/usr/bin/cargo"), cargo),
        busy(process(600, 1, "/usr/bin/unbound-files"), files),
        busy(process(700, 1, "/usr/sbin/sshd"), 0.0),
    ])
}

fn names(monitor: &Monitor) -> Vec<&str> {
    monitor.rows().iter().map(|row| row.name.as_str()).collect()
}

fn child_names(row: &Row) -> Vec<&str> {
    row.children.iter().map(|row| row.name.as_str()).collect()
}

fn terminal() -> RowKey {
    RowKey::Group(GroupKey::App("org.unbound.Terminal".to_owned()))
}

fn cargo() -> RowKey {
    RowKey::Process {
        pid: 502,
        start_ticks: 502,
    }
}

#[test]
fn apps_sort_by_their_summed_cpu_and_selection_follows_a_row_that_moves() {
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(desk(1.0, 2.0, 10.0));
    assert_eq!(names(&monitor), ["Files", "Terminal", "sshd"]);
    assert_eq!(monitor.rows()[1].cells[1], "3.5");
    assert_eq!(monitor.rows()[0].children, []);

    monitor.set_selection(vec![terminal()]);
    monitor.update(desk(1.0, 40.0, 10.0));
    assert_eq!(names(&monitor), ["Terminal", "Files", "sshd"]);
    assert_eq!(monitor.selection(), [terminal()]);

    monitor.set_sort(Column::Name, false);
    assert_eq!(names(&monitor), ["Files", "sshd", "Terminal"]);
    monitor.set_sort(Column::Name, true);
    assert_eq!(names(&monitor), ["Terminal", "sshd", "Files"]);
}

#[test]
fn an_apps_processes_sort_under_it_and_a_selected_one_stays_selected() {
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(desk(1.0, 2.0, 10.0));
    assert_eq!(
        child_names(&monitor.rows()[1]),
        ["cargo", "unbound-term", "bash"]
    );
    monitor.set_selection(vec![cargo()]);
    monitor.update(desk(1.0, 0.0, 10.0));
    assert_eq!(
        child_names(&monitor.rows()[1]),
        ["unbound-term", "bash", "cargo"]
    );
    assert_eq!(monitor.selection(), [cargo()]);
}

#[test]
fn a_selected_process_that_exits_leaves_nothing_selected() {
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(desk(1.0, 2.0, 10.0));
    monitor.set_selection(vec![RowKey::Group(GroupKey::App(
        "org.unbound.Files".to_owned(),
    ))]);
    let mut without_files = desk(1.0, 2.0, 10.0);
    without_files.processes.retain(|process| process.pid != 600);
    monitor.update(without_files);
    assert_eq!(monitor.selection(), []);
}

#[test]
fn search_by_process_name_or_pid_opens_the_app_on_the_matching_processes() {
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(desk(1.0, 2.0, 10.0));
    monitor.set_search("CARGO");
    assert_eq!(names(&monitor), ["Terminal"]);
    assert_eq!(child_names(&monitor.rows()[0]), ["cargo"]);
    assert!(monitor.rows()[0].revealed);
    monitor.set_search("600");
    assert_eq!(names(&monitor), ["Files"]);
    monitor.set_search("term");
    assert_eq!(names(&monitor), ["Terminal"]);
    assert_eq!(monitor.rows()[0].children.len(), 3);
    assert!(!monitor.rows()[0].revealed);
}

#[test]
fn quitting_an_app_signals_its_roots_and_force_quitting_signals_every_process() {
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(desk(1.0, 2.0, 10.0));
    monitor.set_selection(vec![terminal()]);
    let target = monitor.stop_target().expect("the terminal");
    assert_eq!(target.name, "Terminal");
    assert!(target.roots.is_empty());
    assert_eq!(
        target.apps,
        [("org.unbound.Terminal".to_owned(), vec![(500, 500)])]
    );
    assert_eq!(target.processes, [(500, 500), (501, 501), (502, 502)]);

    monitor.set_selection(vec![cargo(), terminal()]);
    let target = monitor.stop_target().expect("cargo and the terminal");
    assert_eq!((target.name.as_str(), target.count), ("cargo", 2));
    assert_eq!(target.roots, [(502, 502)]);
    assert_eq!(target.apps.len(), 1);
    assert_eq!(target.processes, [(502, 502), (500, 500), (501, 501)]);
}

#[test]
fn disk_cells_show_a_dash_where_a_process_cannot_be_read() {
    let mut readable = process(800, 1, "/usr/bin/rsync");
    readable.disk = Some(Rates {
        read_per_second: 0.0,
        written_per_second: 2_500_000.0,
        read_total: 0,
        written_total: 7_000_000_000,
    });
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(snapshot(vec![readable, process(801, 1, "/usr/sbin/cron")]));
    monitor.set_tab(Tab::Disk);
    let cells = monitor
        .rows()
        .iter()
        .map(|row| row.cells.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        cells[0][..5],
        ["rsync", "0 bytes/s", "2.5 MB/s", "0 bytes", "7 GB"]
    );
    assert_eq!(cells[1][..3], ["cron", "\u{2013}", "\u{2013}"]);
}

#[test]
fn the_network_tab_lists_interfaces_by_traffic() {
    let interface = |name: &str, received: f64| Interface {
        name: name.to_owned(),
        received_per_second: received,
        sent_per_second: 0.0,
        received_total: 0,
        sent_total: 0,
    };
    let mut monitor = Monitor::new(catalog(), 1000);
    monitor.update(Snapshot {
        interfaces: vec![interface("eth0", 10.0), interface("wlan0", 2_000.0)],
        ..desk(1.0, 2.0, 10.0)
    });
    monitor.set_tab(Tab::Network);
    assert_eq!(names(&monitor), ["wlan0", "eth0"]);
    assert_eq!(monitor.rows()[0].cells[1], "2 KB/s");
    assert_eq!(monitor.stop_target(), None);
}

#[test]
fn values_read_like_activity_monitor() {
    assert_eq!(format_cpu_time(83.456), "1:23.46");
    assert_eq!(format_cpu_time(3_725.0), "1:02:05");
    assert_eq!(format_bytes(999), "999 bytes");
    assert_eq!(format_bytes(1_234_567), "1.2 MB");
    assert_eq!(format_bytes(16_000_000_000), "16 GB");
    assert_eq!(format_rate(1.0), "1 byte/s");
}
