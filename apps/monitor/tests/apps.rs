#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::rc::Rc;

use fixture::{entry, process};
use unbound_monitor::{Catalog, Group, GroupKey, Process};

fn catalog() -> Catalog {
    Catalog::new(&[
        entry("org.unbound.Terminal", "Terminal", "unbound-term", false),
        entry("org.unbound.Files", "Files", "unbound-files %U", false),
        entry("htop", "htop", "htop", true),
        entry(
            "org.example.Editor",
            "Editor",
            "flatpak run org.example.Editor",
            false,
        ),
        entry("org.example.My-Tool", "My Tool", "/opt/tool/run", false),
        entry(
            "org.example.Script",
            "Script",
            "python3 /opt/script.py",
            false,
        ),
        entry("viewer-a", "Viewer A", "viewer --a", false),
        entry("viewer-b", "Viewer B", "viewer --b", false),
    ])
}

fn in_unit(mut process: Process, unit: &str) -> Process {
    Rc::make_mut(&mut process.identity).unit = Some(unit.to_owned());
    process
}

fn pids(groups: &[Group], processes: &[Process], key: &GroupKey) -> Vec<u32> {
    let group = groups
        .iter()
        .find(|group| &group.key == key)
        .unwrap_or_else(|| panic!("no group {key:?} in {groups:#?}"));
    group
        .members
        .iter()
        .map(|member| processes[*member].pid)
        .collect()
}

fn app(id: &str) -> GroupKey {
    GroupKey::App(id.to_owned())
}

fn alone(pid: u32) -> GroupKey {
    GroupKey::Process {
        pid,
        start_ticks: u64::from(pid),
    }
}

#[test]
fn terminal_children_stay_with_the_terminal_but_apps_started_from_it_stand_alone() {
    let processes = [
        process(500, 1, "/usr/bin/unbound-term"),
        process(501, 500, "/bin/bash"),
        process(502, 501, "cargo build"),
        process(503, 502, "rustc --edition=2024"),
        process(504, 501, "/usr/bin/unbound-files /tmp"),
        process(505, 504, "/usr/bin/helper"),
    ];
    let groups = catalog().group(&processes);

    assert_eq!(
        pids(&groups, &processes, &app("org.unbound.Terminal")),
        [500, 501, 502, 503]
    );
    assert_eq!(
        pids(&groups, &processes, &app("org.unbound.Files")),
        [504, 505]
    );
    assert_eq!(groups.len(), 2);
}

#[test]
fn systemd_app_scopes_name_the_app_even_when_the_program_does_not() {
    let processes = [
        in_unit(
            process(600, 1, "/app/bin/editor-bin"),
            "app-flatpak-org.example.Editor-81623.scope",
        ),
        in_unit(
            process(601, 1, "/opt/tool/tool-daemon"),
            "app-org.example.My\\x2dTool@4f2a.service",
        ),
        in_unit(
            process(602, 1, "/usr/bin/other"),
            "app-gnome-org.unknown.App-1.scope",
        ),
    ];
    let groups = catalog().group(&processes);

    assert_eq!(pids(&groups, &processes, &app("org.example.Editor")), [600]);
    assert_eq!(
        pids(&groups, &processes, &app("org.example.My-Tool")),
        [601]
    );
    assert_eq!(pids(&groups, &processes, &alone(602)), [602]);
}

#[test]
fn interpreters_terminal_entries_and_shared_programs_never_claim_a_process() {
    let processes = [
        process(700, 1, "/usr/bin/python3 /home/mikael/other.py"),
        process(701, 1, "/usr/bin/htop"),
        process(702, 1, "/usr/bin/viewer"),
    ];
    let groups = catalog().group(&processes);

    assert!(groups.iter().all(|group| group.app.is_none()));
    assert_eq!(groups.len(), 3);
}

#[test]
fn services_keep_their_same_named_workers_and_leave_other_children_alone() {
    let processes = [
        process(800, 1, "/usr/sbin/sshd"),
        process(801, 800, "/usr/sbin/sshd"),
        process(802, 801, "/bin/bash"),
        process(900, 1, "/usr/bin/unbound-dock"),
        process(901, 900, "/usr/bin/unbound-term"),
    ];
    let groups = catalog().group(&processes);

    assert_eq!(pids(&groups, &processes, &alone(800)), [800, 801]);
    assert_eq!(pids(&groups, &processes, &alone(802)), [802]);
    assert_eq!(pids(&groups, &processes, &alone(900)), [900]);
    assert_eq!(
        pids(&groups, &processes, &app("org.unbound.Terminal")),
        [901]
    );
}

#[test]
fn kernel_threads_share_one_group_and_the_oldest_root_leads_an_app() {
    let processes = [
        process(2, 0, "kthreadd"),
        process(30, 2, "kworker/0:1"),
        process(1200, 1, "/usr/bin/unbound-files"),
        process(1100, 1, "/usr/bin/unbound-files"),
        process(1300, 1200, "/usr/bin/thumbnailer"),
    ];
    let groups = catalog().group(&processes);

    assert_eq!(pids(&groups, &processes, &GroupKey::Kernel), [2, 30]);
    let files = groups
        .iter()
        .find(|group| group.key == app("org.unbound.Files"))
        .expect("files");
    assert_eq!(processes[files.leader].pid, 1100);
    assert_eq!(files.name, "Files");
    assert_eq!(files.members.len(), 3);
}
