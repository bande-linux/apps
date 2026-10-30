#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tempfile::TempDir;
use unbound_monitor::{Identity, Process, Sampler};
use unbound_toolkit::{CpuRenderer, Scene};
use unbound_xdg::DesktopEntry;

pub const TICKS_PER_SECOND: u64 = 100;
pub const PAGE_SIZE: u64 = 4096;

pub struct Fixture {
    pub root: TempDir,
}

pub struct Task<'a> {
    pub pid: u32,
    pub ppid: u32,
    pub comm: &'a str,
    pub command: &'a [&'a str],
    pub uid: u32,
    pub cpu_ticks: u64,
    pub start_ticks: u64,
    pub resident_pages: u64,
    pub unit: &'a str,
}

impl Default for Task<'_> {
    fn default() -> Self {
        Self {
            pid: 100,
            ppid: 1,
            comm: "task",
            command: &[],
            uid: 1000,
            cpu_ticks: 0,
            start_ticks: 1000,
            resident_pages: 256,
            unit: "session-1.scope",
        }
    }
}

impl Fixture {
    pub fn new() -> Self {
        let fixture = Self {
            root: tempfile::tempdir().expect("temporary folder"),
        };
        fixture.write(
            "etc/passwd",
            "root:x:0:0:root:/root:/bin/sh\nmikael:x:1000:1000:Mikael:/home/mikael:/bin/bash\n",
        );
        fixture.write(
            "proc/meminfo",
            "MemTotal:       16000000 kB\nMemFree:         2000000 kB\nMemAvailable:   10000000 kB\nBuffers:          100000 kB\nCached:          3000000 kB\nSwapCached:            0 kB\nSReclaimable:     400000 kB\nSwapTotal:       8000000 kB\nSwapFree:        7000000 kB\n",
        );
        fixture.write(
            "proc/pressure/memory",
            "some avg10=1.25 avg60=0.50 avg300=0.10 total=100\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
        );
        fixture.write(
            "sys/block/zram0/mm_stat",
            "4000000 1000000 1100000 0 1100000 0 0 0 0\n",
        );
        fixture.write("sys/block/nvme0n1/device/model", "disk\n");
        fixture.write("sys/block/loop0/size", "0\n");
        fixture.set_cpu(0, 0);
        fixture.set_traffic(0);
        fixture
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn sampler(&self) -> Sampler {
        Sampler::new(self.path(), TICKS_PER_SECOND, PAGE_SIZE)
    }

    pub fn write(&self, relative: &str, text: &str) {
        let path = self.path().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        fs::write(path, text).expect("write");
    }

    pub fn set_cpu(&self, elapsed_seconds: u64, busy_ticks_per_core: u64) {
        let idle = 10_000 + elapsed_seconds * TICKS_PER_SECOND - busy_ticks_per_core;
        let mut stat = format!(
            "cpu  {} 0 0 {} 0 0 0 0 0 0\n",
            2 * busy_ticks_per_core,
            2 * idle
        );
        for core in 0..2 {
            stat.push_str(&format!(
                "cpu{core} {busy_ticks_per_core} 0 0 {idle} 0 0 0 0 0 0\n"
            ));
        }
        self.write("proc/stat", &stat);
    }

    pub fn set_traffic(&self, megabytes: u64) {
        let bytes = megabytes * 1_000_000;
        self.write(
            "proc/net/dev",
            &format!(
                "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n    lo: {bytes} 1 0 0 0 0 0 0 {bytes} 1 0 0 0 0 0 0\n  wlan0: {} 1 0 0 0 0 0 0 {} 1 0 0 0 0 0 0\n",
                2 * bytes,
                bytes
            ),
        );
        let sectors = bytes / 512;
        self.write(
            "proc/diskstats",
            &format!(
                " 259 0 nvme0n1 1 0 {sectors} 0 1 0 {} 0 0 0 0 0 0 0 0 0 0\n 259 1 nvme0n1p1 1 0 {sectors} 0 1 0 {sectors} 0 0 0 0 0 0 0 0 0 0\n 7 0 loop0 1 0 {sectors} 0 1 0 {sectors} 0 0 0 0 0 0 0 0 0 0\n",
                2 * sectors
            ),
        );
    }

    pub fn spawn(&self, task: &Task) {
        let folder = format!("proc/{}", task.pid);
        self.write(
            &format!("{folder}/stat"),
            &format!(
                "{} ({}) S {} {} {} 0 -1 4194304 0 0 0 0 {} 0 0 0 20 0 3 0 {} 1000000 {} 0\n",
                task.pid,
                task.comm,
                task.ppid,
                task.pid,
                task.pid,
                task.cpu_ticks,
                task.start_ticks,
                task.resident_pages
            ),
        );
        let mut cmdline = task.command.join("\0");
        if !cmdline.is_empty() {
            cmdline.push('\0');
        }
        self.write(&format!("{folder}/cmdline"), &cmdline);
        self.write(
            &format!("{folder}/status"),
            &format!(
                "Name:\t{}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n",
                task.comm,
                uid = task.uid
            ),
        );
        self.write(
            &format!("{folder}/cgroup"),
            &format!(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/{}\n",
                task.unit
            ),
        );
    }

    pub fn set_io(&self, pid: u32, read: u64, written: u64) {
        self.write(
            &format!("proc/{pid}/io"),
            &format!("rchar: 0\nwchar: 0\nread_bytes: {read}\nwrite_bytes: {written}\ncancelled_write_bytes: 0\n"),
        );
    }

    pub fn set_proportional(&self, pid: u32, kilobytes: u64) {
        self.write(
            &format!("proc/{pid}/smaps_rollup"),
            &format!(
                "00400000-ffff ---p 00000000 00:00 0 [rollup]\nRss: 9000 kB\nPss: {kilobytes} kB\n"
            ),
        );
    }

    pub fn set_gpu(&self, pid: u32, fd: u32, client: u64, engine_ns: u64) {
        let fds = self.path().join(format!("proc/{pid}/fd"));
        fs::create_dir_all(&fds).expect("fd folder");
        let link = fds.join(fd.to_string());
        if fs::symlink_metadata(&link).is_err() {
            std::os::unix::fs::symlink("/dev/dri/renderD128", link).expect("link");
        }
        self.write(
            &format!("proc/{pid}/fdinfo/{fd}"),
            &format!(
                "pos:\t0\ndrm-driver:\tasahi\ndrm-client-id:\t{client}\ndrm-engine-render:\t{engine_ns} ns\ndrm-engine-compute:\t0 ns\ndrm-resident-memory:\t2048 KiB\n"
            ),
        );
    }

    pub fn exit(&self, pid: u32) {
        fs::remove_dir_all(self.path().join(format!("proc/{pid}"))).expect("remove process");
    }
}

pub fn process(pid: u32, ppid: u32, command: &str) -> Process {
    let program = command.split(' ').next().unwrap_or_default();
    Process {
        pid,
        ppid,
        start_ticks: u64::from(pid),
        identity: Rc::new(Identity {
            name: Path::new(program)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            command: command.split(' ').map(str::to_owned).collect(),
            executable: Some(PathBuf::from(program)),
            uid: 1000,
            user: "mikael".to_owned(),
            unit: Some("session-1.scope".to_owned()),
            kernel_thread: ppid == 2 || pid == 2,
        }),
        cpu_percent: 0.0,
        cpu_seconds: 0.0,
        threads: 1,
        proportional: None,
        resident: 0,
        disk: None,
        gpu: None,
    }
}

pub fn entry(id: &str, name: &str, exec: &str, terminal: bool) -> DesktopEntry {
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nTerminal={terminal}\n"
    );
    DesktopEntry::parse(
        &format!("{id}.desktop"),
        Path::new("/usr/share/applications")
            .join(format!("{id}.desktop"))
            .as_path(),
        &text,
        None,
        &[],
    )
    .expect("valid entry")
}

pub fn check_golden(name: &str, scene: &Scene) {
    let frame = CpuRenderer::new().render(scene).expect("frame");
    unbound_golden::assert_golden!(
        name,
        &unbound_golden::from_rgba(frame.width, frame.height, frame.rgba)
    );
}
