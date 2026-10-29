#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::path::Path;

use fixture::{Fixture, PAGE_SIZE, Task};
use unbound_monitor::{Snapshot, read_cpu_ticks};

fn process(snapshot: &Snapshot, pid: u32) -> &unbound_monitor::Process {
    snapshot
        .processes
        .iter()
        .find(|process| process.pid == pid)
        .expect("process in the snapshot")
}

#[test]
fn cpu_shares_split_user_system_and_idle_time_between_samples() {
    let fixture = Fixture::new();
    fixture.write(
        "proc/stat",
        "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\ncpu1 50 0 25 400 25 0 0 0 0 0\nintr 1\n",
    );
    let (before, cores) = read_cpu_ticks(fixture.path()).expect("first sample");
    assert_eq!(cores.len(), 2);
    fixture.write(
        "proc/stat",
        "cpu  180 20 80 900 50 10 10 0 0 0\ncpu0 130 20 55 400 25 10 10 0 0 0\ncpu1 50 0 25 500 25 0 0 0 0 0\n",
    );
    let (after, cores_after) = read_cpu_ticks(fixture.path()).expect("second sample");

    let share = after.share_since(before);
    assert_eq!((share.user, share.system, share.idle), (0.4, 0.2, 0.4));
    assert_eq!(cores_after[1].share_since(cores[1]).idle, 1.0);
}

#[test]
fn live_cpu_ticks_count_every_core() {
    let (total, cores) = read_cpu_ticks(Path::new("/")).expect("live /proc/stat");
    assert!(!cores.is_empty());
    assert!(total.total >= cores.iter().map(|core| core.total).max().unwrap_or(0));
}

#[test]
fn process_cpu_is_the_share_of_one_core_over_the_interval_as_in_top() {
    let fixture = Fixture::new();
    let busy = Task {
        pid: 100,
        cpu_ticks: 500,
        ..Task::default()
    };
    fixture.spawn(&busy);
    let mut sampler = fixture.sampler();
    let first = sampler.sample().expect("first sample");
    assert_eq!(process(&first, 100).cpu_percent, 0.0);

    fixture.set_cpu(2, 150);
    fixture.spawn(&Task {
        cpu_ticks: 800,
        ..busy
    });
    fixture.spawn(&Task {
        pid: 200,
        cpu_ticks: 50,
        start_ticks: 1100,
        ..Task::default()
    });
    let second = sampler.sample().expect("second sample");

    assert_eq!(second.seconds, 2.0);
    assert_eq!(process(&second, 100).cpu_percent, 150.0);
    assert_eq!(process(&second, 100).cpu_seconds, 8.0);
    assert_eq!(process(&second, 200).cpu_percent, 25.0);
    assert_eq!(second.cpu.user, 0.75);
}

#[test]
fn a_reused_pid_starts_its_cpu_count_afresh() {
    let fixture = Fixture::new();
    fixture.spawn(&Task {
        cpu_ticks: 5000,
        ..Task::default()
    });
    let mut sampler = fixture.sampler();
    sampler.sample().expect("first sample");

    fixture.set_cpu(1, 100);
    fixture.exit(100);
    fixture.spawn(&Task {
        cpu_ticks: 20,
        start_ticks: 9000,
        comm: "newcomer",
        ..Task::default()
    });
    let second = sampler.sample().expect("second sample");

    let newcomer = process(&second, 100);
    assert_eq!(newcomer.identity.name, "newcomer");
    assert_eq!(newcomer.cpu_percent, 20.0);
}

#[test]
fn names_users_and_kernel_threads_come_from_proc() {
    let fixture = Fixture::new();
    fixture.spawn(&Task {
        pid: 2,
        ppid: 0,
        comm: "kthreadd",
        uid: 0,
        ..Task::default()
    });
    fixture.spawn(&Task {
        pid: 40,
        ppid: 2,
        comm: "kworker/0:1",
        uid: 0,
        ..Task::default()
    });
    fixture.spawn(&Task {
        pid: 300,
        comm: "unbound-composi",
        command: &["/usr/bin/unbound-compositor", "--session"],
        ..Task::default()
    });
    fixture.spawn(&Task {
        pid: 301,
        comm: "Web Content",
        command: &["/usr/lib/firefox/firefox", "-contentproc"],
        uid: 4242,
        ..Task::default()
    });
    let snapshot = fixture.sampler().sample().expect("sample");

    assert!(process(&snapshot, 2).identity.kernel_thread);
    assert!(process(&snapshot, 40).identity.kernel_thread);
    assert!(!process(&snapshot, 300).identity.kernel_thread);
    assert_eq!(process(&snapshot, 40).identity.user, "root");
    assert_eq!(process(&snapshot, 300).identity.name, "unbound-compositor");
    assert_eq!(process(&snapshot, 300).identity.user, "mikael");
    assert_eq!(process(&snapshot, 301).identity.name, "Web Content");
    assert_eq!(process(&snapshot, 301).identity.user, "4242");
    assert_eq!(
        process(&snapshot, 301).identity.unit.as_deref(),
        Some("session-1.scope")
    );
}

#[test]
fn memory_is_proportional_only_while_asked_for_and_resident_otherwise() {
    let fixture = Fixture::new();
    fixture.spawn(&Task::default());
    fixture.set_proportional(100, 300);
    let mut sampler = fixture.sampler();

    let cheap = sampler.sample().expect("sample");
    assert_eq!(process(&cheap, 100).proportional, None);
    assert_eq!(process(&cheap, 100).resident, 256 * PAGE_SIZE);

    sampler.measure_proportional_memory(true);
    let detailed = sampler.sample().expect("sample");
    assert_eq!(process(&detailed, 100).proportional, Some(300 * 1024));
}

#[test]
fn disk_rates_count_bytes_since_the_last_sample() {
    let fixture = Fixture::new();
    fixture.spawn(&Task::default());
    fixture.set_io(100, 1_000_000, 0);
    let mut sampler = fixture.sampler();
    let first = sampler.sample().expect("first sample");
    let rates = process(&first, 100).disk.expect("readable io");
    assert_eq!((rates.read_per_second, rates.read_total), (0.0, 1_000_000));

    fixture.set_cpu(2, 0);
    fixture.set_io(100, 5_000_000, 2_000_000);
    fixture.spawn(&Task {
        pid: 101,
        ..Task::default()
    });
    let second = sampler.sample().expect("second sample");

    let rates = process(&second, 100).disk.expect("readable io");
    assert_eq!(
        (rates.read_per_second, rates.written_per_second),
        (2_000_000.0, 1_000_000.0)
    );
    assert_eq!(process(&second, 101).disk, None);
}

#[test]
fn system_totals_cover_memory_physical_disks_and_interfaces_without_loopback() {
    let fixture = Fixture::new();
    let mut sampler = fixture.sampler();
    sampler.sample().expect("first sample");
    fixture.set_cpu(4, 0);
    fixture.set_traffic(8);
    let snapshot = sampler.sample().expect("second sample");

    let memory = snapshot.memory;
    assert_eq!(memory.total, 16_000_000 * 1024);
    assert_eq!(memory.used, 6_000_000 * 1024);
    assert_eq!(memory.cached, 3_500_000 * 1024);
    assert_eq!(memory.swap_used, 1_000_000 * 1024);
    assert_eq!(memory.compressed, 1_000_000);
    assert_eq!(memory.pressure, 1.25);
    assert_eq!(
        (
            snapshot.disk.read_per_second,
            snapshot.disk.written_per_second
        ),
        (2_000_000.0, 4_000_000.0)
    );
    let names = snapshot
        .interfaces
        .iter()
        .map(|interface| interface.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["wlan0"]);
    assert_eq!(snapshot.interfaces[0].received_per_second, 4_000_000.0);
    assert_eq!(snapshot.interfaces[0].sent_per_second, 2_000_000.0);
}
