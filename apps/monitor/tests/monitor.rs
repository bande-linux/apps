#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixture;

use std::path::Path;

use fixture::{Fixture, PAGE_SIZE, Task};
use unbound_monitor::{
    Catalog, CoreKind, InterfaceKind, Monitor, Pressure, PressureLevel, Snapshot, Tab,
    read_cpu_ticks,
};

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
fn while_nobody_looks_only_the_system_is_sampled_and_processes_resume_without_a_spike() {
    let fixture = Fixture::new();
    let busy = Task {
        pid: 100,
        cpu_ticks: 500,
        ..Task::default()
    };
    fixture.spawn(&busy);
    let mut sampler = fixture.sampler();
    sampler.sample().expect("first sample");

    sampler.measure_processes(false);
    fixture.set_cpu(2, 150);
    fixture.spawn(&Task {
        cpu_ticks: 800,
        ..busy
    });
    let hidden = sampler.sample().expect("hidden sample");
    sampler.measure_processes(true);
    fixture.set_cpu(4, 300);
    fixture.spawn(&Task {
        cpu_ticks: 1100,
        ..busy
    });
    let shown = sampler.sample().expect("shown sample");
    fixture.set_cpu(6, 450);
    fixture.spawn(&Task {
        cpu_ticks: 1300,
        ..busy
    });
    let next = sampler.sample().expect("next sample");

    assert!(hidden.processes.is_empty());
    assert_eq!(hidden.cpu.user, 0.75);
    assert_eq!(process(&shown, 100).cpu_percent, 0.0);
    assert_eq!(process(&next, 100).cpu_percent, 100.0);
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
    sampler.measure_disk(true);
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
fn disk_counters_are_read_only_while_asked_for_and_rates_restart_from_zero() {
    let fixture = Fixture::new();
    fixture.spawn(&Task::default());
    fixture.set_io(100, 1_000_000, 0);
    let mut sampler = fixture.sampler();
    let cheap = sampler.sample().expect("first sample");
    assert_eq!(process(&cheap, 100).disk, None);

    fixture.set_cpu(2, 0);
    fixture.set_io(100, 5_000_000, 0);
    sampler.measure_disk(true);
    let measured = sampler.sample().expect("second sample");

    let rates = process(&measured, 100).disk.expect("readable io");
    assert_eq!((rates.read_per_second, rates.read_total), (0.0, 5_000_000));
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
    assert_eq!(memory.app, 4_200_000 * 1024);
    assert_eq!(memory.kernel, 400_000 * 1024);
    assert_eq!(
        memory.pressure,
        Pressure {
            some: 1.25,
            full: 0.0
        }
    );
    assert_eq!(
        (
            snapshot.disk.read_per_second,
            snapshot.disk.written_per_second
        ),
        (2_000_000.0, 4_000_000.0)
    );
    assert_eq!(
        (
            snapshot.disk_operations.reads_per_second,
            snapshot.disk_operations.writes_per_second
        ),
        (20.0, 10.0)
    );
    let names = snapshot
        .interfaces
        .iter()
        .map(|interface| (interface.name.as_str(), interface.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            ("wlan0", InterfaceKind::WiFi),
            ("eth0", InterfaceKind::Ethernet),
            ("wg0", InterfaceKind::Vpn)
        ]
    );
    assert_eq!(snapshot.interfaces[0].received_per_second, 4_000_000.0);
    assert_eq!(snapshot.interfaces[0].sent_per_second, 2_000_000.0);
}

#[test]
fn gpu_use_comes_from_drm_fdinfo_once_per_client_and_only_while_asked_for() {
    let fixture = Fixture::new();
    fixture.spawn(&Task::default());
    fixture.spawn(&Task {
        pid: 101,
        ..Task::default()
    });
    fixture.set_gpu(100, 7, 42, 1_000_000_000);
    fixture.set_gpu(100, 8, 42, 1_000_000_000);
    let mut sampler = fixture.sampler();

    let first = sampler.sample().expect("first sample");
    assert!(first.gpu);
    let used = process(&first, 100).gpu.expect("gpu use");
    assert_eq!((used.percent, used.seconds), (0.0, 1.0));
    assert_eq!(used.memory, 2 * 1024 * 1024);
    assert_eq!(process(&first, 101).gpu, None);

    fixture.set_cpu(2, 0);
    fixture.set_gpu(100, 7, 42, 2_000_000_000);
    fixture.set_gpu(100, 8, 42, 2_000_000_000);
    let idle = sampler.sample().expect("sample");
    assert_eq!(process(&idle, 100).gpu, None);
    assert!(idle.gpu);

    sampler.measure_gpu(true);
    sampler.sample().expect("sample");
    fixture.set_cpu(4, 0);
    fixture.set_gpu(100, 7, 42, 3_000_000_000);
    fixture.set_gpu(100, 8, 42, 3_000_000_000);
    let measured = sampler.sample().expect("sample");
    assert_eq!(process(&measured, 100).gpu.expect("gpu use").percent, 50.0);
}

#[test]
fn a_system_without_gpu_statistics_reports_none() {
    let fixture = Fixture::new();
    fixture.spawn(&Task::default());
    let snapshot = fixture.sampler().sample().expect("sample");
    assert!(!snapshot.gpu);
}

#[test]
fn memory_pressure_turns_amber_when_tasks_wait_and_red_when_all_of_them_stall() {
    let fixture = Fixture::new();
    let level = |some: f32, full: f32| {
        fixture.write(
            "proc/pressure/memory",
            &format!(
                "some avg10={some:.2} avg60=0.00 avg300=0.00 total=1\nfull avg10={full:.2} avg60=0.00 avg300=0.00 total=1\n"
            ),
        );
        let snapshot = fixture.sampler().sample().expect("sample");
        snapshot.memory.pressure.level()
    };
    assert_eq!(level(4.0, 0.0), PressureLevel::Normal);
    assert_eq!(level(12.0, 0.0), PressureLevel::Elevated);
    assert_eq!(level(12.0, 6.0), PressureLevel::Critical);
}

fn core_stat(cores: [(u64, u64); 4]) -> String {
    let (busy, idle) = cores
        .iter()
        .fold((0, 0), |(busy, idle), core| (busy + core.0, idle + core.1));
    let mut stat = format!("cpu  {busy} 0 0 {idle} 0 0 0 0 0 0\n");
    for (core, (busy, idle)) in cores.iter().enumerate() {
        stat.push_str(&format!("cpu{core} {busy} 0 0 {idle} 0 0 0 0 0 0\n"));
    }
    stat
}

#[test]
fn cores_of_two_capacities_group_into_performance_and_efficiency_with_their_own_shares() {
    let fixture = Fixture::new();
    for (core, capacity) in [1024, 1024, 512, 512].into_iter().enumerate() {
        fixture.write(
            &format!("sys/devices/system/cpu/cpu{core}/cpu_capacity"),
            &format!("{capacity}\n"),
        );
    }
    fixture.write("proc/stat", &core_stat([(0, 1000); 4]));
    let mut sampler = fixture.sampler();
    let mut monitor = Monitor::new(Catalog::new(&[]), 1000);
    monitor.update(sampler.sample().expect("first sample"));
    fixture.write(
        "proc/stat",
        &core_stat([(200, 1000), (100, 1100), (0, 1200), (50, 1150)]),
    );
    let snapshot = sampler.sample().expect("second sample");
    assert_eq!(snapshot.capacities, [1024, 1024, 512, 512]);
    monitor.update(snapshot);

    let shown = monitor
        .core_groups()
        .into_iter()
        .map(|group| (group.kind, group.cores))
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            (
                Some(CoreKind::Performance),
                vec![(0, vec![1.0]), (1, vec![0.5])]
            ),
            (
                Some(CoreKind::Efficiency),
                vec![(2, vec![0.0]), (3, vec![0.25])]
            ),
        ]
    );
}

fn set_switches(fixture: &Fixture, pid: u32, switches: u64) {
    fixture.write(
        &format!("proc/{pid}/task/{pid}/status"),
        &format!(
            "Name:\ttask\nvoluntary_ctxt_switches:\t{switches}\nnonvoluntary_ctxt_switches:\t3\n"
        ),
    );
}

#[test]
fn an_app_that_wakes_often_uses_more_energy_than_an_idle_one_and_only_on_a_laptop() {
    let fixture = Fixture::new();
    for (pid, comm) in [(100, "chatty"), (101, "quiet")] {
        fixture.spawn(&Task {
            pid,
            comm,
            start_ticks: u64::from(pid),
            ..Task::default()
        });
        set_switches(&fixture, pid, 1000);
    }
    let mut desktop = fixture.sampler();
    desktop.sample().expect("first sample");
    fixture.set_cpu(2, 0);
    set_switches(&fixture, 100, 2200);
    let cheap = desktop.sample().expect("second sample");
    assert!(
        cheap
            .processes
            .iter()
            .all(|process| process.wakeups.is_none())
    );
    let mut monitor = Monitor::new(Catalog::new(&[]), 1000);
    monitor.update(cheap);
    assert!(!monitor.tabs().contains(&Tab::Energy));

    fixture.write("sys/class/power_supply/BAT0/type", "Battery\n");
    fixture.set_cpu(0, 0);
    set_switches(&fixture, 100, 1000);
    let mut laptop = fixture.sampler();
    laptop.measure_wakeups(true);
    laptop.sample().expect("first sample");
    fixture.set_cpu(2, 0);
    set_switches(&fixture, 100, 2200);
    let measured = laptop.sample().expect("second sample");
    assert_eq!(
        measured
            .processes
            .iter()
            .map(|process| (process.pid, process.wakeups))
            .collect::<Vec<_>>(),
        [(100, Some(600.0)), (101, Some(0.0))]
    );
    let mut monitor = Monitor::new(Catalog::new(&[]), 1000);
    monitor.update(measured);
    assert!(monitor.tabs().contains(&Tab::Energy));
    monitor.set_tab(Tab::Energy);
    assert!(monitor.measures_wakeups());
    let rows = monitor
        .rows()
        .iter()
        .map(|row| (row.name.clone(), row.cells[1].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        [
            ("chatty".to_owned(), "60.0".to_owned()),
            ("quiet".to_owned(), "0.0".to_owned())
        ]
    );
}
