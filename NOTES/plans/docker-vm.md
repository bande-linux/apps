# Containers and virtual machines: plan for a native Bande app

From the owner's note `NOTES/docker_vm.md` (the "Velox for Linux" brief), adapted to Bande: one
`bande-app` app in this workspace, not a GTK repo with its own CLI. The brief's North Star stays:
a pure client that adds nothing to the engine's or the guests' datapath, zero idle wake-ups, a
small footprint and two areas, each gated on its own. Delete this plan when its milestones are done.

Working name `velox` (package `bande-velox`, id `org.bande.Velox`, "Velox"); see decision 1.

## What the M2 mini gives us (checked read-only on 7.3-rc7+bande-apple-arm64-16k)

- KVM works as shipped: `CONFIG_KVM=y`, the `kvm` module is loaded, `/dev/kvm` exists, and the
  kernel boots at EL2. `VHOST_NET`, `VHOST_VSOCK`, `TUN`, `BRIDGE`, `VETH`, `NF_TABLES`/NAT,
  `OVERLAY_FS`, `USER_NS`, `MEMCG` and `UDMABUF` are all there. All of this comes from Debian's
  `arm64-16k` config, so the kernel repo needs no change for v1.
- `/dev/kvm` is `root:kvm 0660` with udev's `uaccess` tag, so the user logged in at the seat
  gets an ACL and needs no `kvm` group. Right now the ACL names `_greetd`, because the seat is
  at the greeter. Being in the `kvm` group matters only for SSH-only use.
- The CPU is 4 Blizzard (E) and 4 Avalanche (P) cores: no SMT, no SVE, an Apple PMU and AIC
  with a partial vGICv3. QEMU runs `-M virt,gic-version=3 -accel kvm -cpu host`. Guests run
  arm64 only at native speed; x86 guests would be TCG emulation, so they are out of scope.
- Installed: `qemu-system-arm` 11.1, `qemu-efi-aarch64` (AAVMF, with secboot/ms variants),
  `virtiofsd`, and `qemu-system-modules-opengl`, which ships `ui-dbus.so` (`-display dbus`).
  In testing but not installed: `passt`, `swtpm`, `podman` 5.8, `docker.io` 28.5,
  `docker-compose` 2.40, `podman-compose`, `libvirt` 12.8.
- The host uses 16K pages. Guests can use 4K pages, but some arm64 container images built with
  a 4K-page jemalloc crash on a 16K host. That is a real limit for native containers (see Risks).
- The compositor already has `keyboard_shortcuts_inhibit`, `pointer_constraints`,
  `relative_pointer`, `linux-dmabuf` and subsurfaces. The toolkit uses none of them yet.
- I could not boot a KVM guest from this agent: no seat ACL over SSH, and no `kvm` group.
  Milestone V0 boots one.

## Goals and scope (the brief's sections → v1)

| Brief | v1 | Later |
|---|---|---|
| §4 Containers: list grouped by Compose project, start/stop/restart/pause/kill/remove, logs (follow, search), inspect, stats, ports as links | yes | |
| §4 Exec terminal | Open in Terminal (hand-off, the brief allows it) | embedded, after the terminal-view lib (A6) |
| §4 Images (pull with progress, tag, remove, prune, history), volumes, networks, `/system/df`, engine info | yes | |
| §4 Compose up/down/restart by spawning `docker compose` | yes | |
| §4 Remote hosts over SSH, `<name>.velox.local` DNS | | later: DNS needs a resolved drop-in, i.e. root |
| §5.1 Cloud-image catalog + cloud-init, UEFI (AAVMF) | yes: Debian, Ubuntu, Fedora arm64 | |
| §5.1 ISO install, OS presets, Windows 11 ARM + swtpm + Secure Boot, import formats | | V5 / later |
| §5.2 Lifecycle, guest agent, stats | yes | autostart at login (enabled user unit) |
| §5.2 Graphical console embedded (D-Bus display) | V3, needs desktop D1 | clipboard, virgl |
| §5.2 Serial console | Open in Terminal (`ssh`) first | embedded via A6 |
| §5.2 virtiofs shares, USB hot-plug | V5 | PCI/VFIO, migration: out |
| §5.3 Snapshots (live internal, tree, revert) | V4 | disk-only with `fsfreeze` |
| §5.4 Full/linked clone | V4 | export/import (bundle, OVA, disk formats) |
| §5.5 passt user networking with port forwards | yes | NAT/bridged/isolated, `setup`, Docker FORWARD fix |
| §6 Two readiness state machines, live on events | yes | |
| CLI `velox`, `doctor`, the self-updater, `versions.env` | cut: Bande ships through apt, and the app is the UI | `doctor` text = the readiness screen |

## Architecture

One app, `apps/velox`, with no daemon of its own and nothing resident when no VM runs and no
window is open. The rule from `docs/architecture/apps.md` applies: event fds are `Program`
sources, and requests run on a worker that wakes the main loop over a `UnixStream` pair (the
same pattern as Calendar's and Mail's `worker.rs`). There is no async runtime.

| Module | What it does |
|---|---|
| `engine/http.rs` | minimal HTTP/1.1 over `UnixStream`: request, content-length, incremental chunked decoding, hijack (`Upgrade: tcp`) and Docker's 8-byte stream framing. `httparse` is the only new dependency |
| `engine/client.rs` | typed Engine API calls (serde models): containers, images, volumes, networks, `system/df`, `info`, `version`, `/images/create` progress |
| `engine/socket.rs` | socket resolution: `DOCKER_HOST`, the current context in `~/.docker/config.json`, `$XDG_RUNTIME_DIR/docker.sock`, `$XDG_RUNTIME_DIR/podman/podman.sock`, `/run/docker.sock`, `/run/podman/podman.sock` |
| `engine/informer.rs` | one `/events` stream as a non-blocking source; a full re-list (reconcile) on every (re)connect; a store keyed by id |
| `vm/manifest.rs` | versioned JSON per VM, written durably (temp + fsync + rename, with `bande-fs` helpers where they fit); corrupt JSON fails loudly |
| `vm/qemu.rs` | argv builder: `virt,gic-version=3`, `-cpu host`, AAVMF code plus vars as **qcow2** (so internal snapshots cover NVRAM), virtio-blk `cache=none,aio=io_uring,discard=unmap`, virtio-rng, virtio-net over passt, the guest agent channel, `-sandbox on`, `-display dbus,p2p=yes` |
| `vm/unit.rs` | starts each VM as a transient systemd **user** unit (`StartTransientUnit` over the session bus, zbus): `MemoryMax`, `CPUQuota`, `NoNewPrivileges`; follows lifecycle through `PropertiesChanged`; the VM outlives the app |
| `vm/qmp.rs` | one QMP client: a non-blocking line-JSON source per running VM for events (`STOP`, `RESUME`, `SHUTDOWN`, `JOB_STATUS_CHANGE`, …) and commands. QEMU gets a second `-qmp` socket for scripts, so no runner process is needed |
| `vm/cloud.rs` | image catalog (data file: URL + SHA256/512), download with progress, `qemu-img create -b` overlay, NoCloud seed as a small vfat `CIDATA` image (`fatfs` crate, so no ISO tooling) |
| `readiness.rs` | one state machine, two providers (Engine, Virtualization). It re-evaluates on inotify (`bande-watch`) of the socket folders, `/dev` and `/var/lib/dpkg/status`. The first blocking state wins |
| `jobs.rs` | one job/progress/cancel model: pull, image download, snapshot, clone |
| `ui/…` | views (below) |

- **Live state without polling:** containers follow `/events`; VMs follow QMP events plus
  systemd unit state; readiness follows inotify. Stats come from `/containers/{id}/stats` streams
  and from cgroup reads of the VM unit, but only while the detail pane or Activity graph is
  visible. When no window is open, the app drops the `/events` stream (a socket-activated
  Podman then exits after its idle time) and QMP sources, and reconciles on Reopen.
- **Podman versus Docker:** one client for the Docker Engine API, which Podman's compatible API
  serves too (events, stats, exec hijack and `/images/create` all work). The app recommends
  rootless Podman: it is in Debian main, needs no root-equivalent group, is socket-activated
  (`podman.socket`, so nothing is resident while idle), and pulls in `passt`, which VMs reuse.
  Docker CE, Debian's `docker.io` and rootless Docker work through the same socket order. The
  brief's "docker-ce only" rule becomes decision 3.
- **Compose:** projects are grouped by the `com.docker.compose.project` label; podman-compose
  also sets it. Actions spawn `docker compose -p P --project-directory D -f …` with
  `DOCKER_HOST` set to the resolved socket, as an argv with no shell.
- **Why not libvirt:** it adds a resident (or socket-activated) daemon, XML, and an extra
  `libvirt-dbus` daemon for D-Bus access. Session mode (`qemu:///session`) still can't do
  bridged networking without helpers, and the D-Bus display console still has to come from QEMU.
  Direct QEMU with systemd units gives supervision, cgroup limits, journald logs and clean stop on
  logout. libvirt's value is interop with virt-manager, which the owner doesn't use here.
  **crosvm/cloud-hypervisor:** not in Debian (crosvm) or weak on arm64 UEFI, snapshots and
  display. Neither is a fit.
- **Display:** QEMU's D-Bus display (`org.qemu.Display1`, p2p through QMP `add_client`). The
  `qemu-display` crate is zbus-based like ours: use it if it is on zbus 5, else write the few
  interfaces we use ourselves. Scanouts arrive as shared memory (`ScanoutMap` +
  `UpdateMap`) or as dmabuf when `gl=on`. Resize goes through `SetUIInfo`, so virtio-gpu
  hot-resizes with no guest agent. Input comes from the Keyboard/Mouse interfaces with a
  virtio-tablet, so normal use needs no pointer grab. VNC/SPICE were rejected: no Rust SPICE
  client; VNC copies every frame and has poor clipboard support.

## UI (macOS-faithful, Bande style)

- **Main window:** a sidebar source list with Containers, Images, Volumes and Networks, then
  **Virtual Machines** with each VM listed by name and state dot (as in UTM). Content is a
  `bande-widgets` `Table` with disclosure rows for Compose projects, plus a detail pane with
  Info, Logs and Stats. The toolbar has Start/Stop/Restart (or Pull… / New VM…), a search field
  and the ⋮ menu button. Lists don't flicker: events update rows in place.
- **Gated areas:** a sidebar section that isn't ready shows one centred line plus one fix, as
  in §6: "Install a container engine to continue" with an Install… button (PackageKit, polkit,
  reusing Software's/Installer's `bande-packages` flow) and copyable steps; "Log out and back in
  to finish"; "No access to Docker" with the group note. The other area keeps working.
- **New Virtual Machine:** a sheet assistant: OS (Debian/Ubuntu/Fedora cloud) → size (CPU,
  memory, disk; defaults 4 P-cores, 4 GB, 32 GB) → name and the user's `~/.ssh/*.pub` key →
  Create. A job row shows download, overlay and first boot.
- **VM console:** its own window per VM (as in UTM). Ctrl+Alt releases the keyboard. When the
  keyboard is captured, a shortcuts-inhibit request lets Super reach the guest only after
  consent. A closed console window keeps the VM running.
- **Menus:** File (New Virtual Machine… Ctrl+N, Pull Image… Shift+Ctrl+N), Container
  (Start, Stop, Restart, Pause, Kill, Remove…, Open in Terminal, Show Logs), Machine (Start,
  Pause, Shut Down, Force Stop, Take Snapshot…, Clone…, Show Console), View (sections,
  Show Stats). Removing anything asks first and names what is lost.
- **Settings window** (`bande-app-settings`): engine socket (Automatic or a path), VM storage
  folder (`$XDG_DATA_HOME/bande/velox/vms`), default VM resources.
- **Lifecycle:** single instance, keeps running windowless, no background mode (the engine
  and the systemd units run without us), so `runs_in_background` stays false.

## Security

- The app needs no root at runtime. Installs go through PackageKit with polkit (the user
  authenticates in `bande-polkit`), and only Debian packages are installed. Docker CE is never
  installed for the user, only linked to.
- The `docker` group is root-equivalent. The app says so where it shows the fix and recommends
  rootless Podman.
- Each QEMU runs in the user's own unit with `-sandbox on` (seccomp), `NoNewPrivileges`,
  `MemoryMax`, and only the VM folder writable where user units allow it. Bridge helpers are
  setuid, so they aren't in v1.
- Downloads are verified against the vendor's SHA256/512SUMS over HTTPS before use. Signature
  checking with `gpgv` comes later; see decision 6.
- Every spawned process (`docker compose`, Terminal, `ssh`, `qemu-*`) gets an argv, never a
  shell string. Container names and labels are data, never interpolated into commands.
- Never write to a disk that a running VM has open: guard on the unit's actual attach list.
  QEMU's image locking is a second line of defence. Never `cache=unsafe`.

## Performance targets (measured with `bande-background` and the bench scripts on the mini)

- Idle: **0 wake-ups/s**, both with a window open on a static list and windowless. Windowless
  private memory ≤ 4 MB, like Terminal's 3.4 MB.
- Cold start to a drawn list of 100 containers ≤ 150 ms. 1,000 containers stay smooth.
- Stats only while visible. A static console window costs 0 % CPU (frames only on damage). A
  1080p console at 60 fps ≤ 5 % of one P-core with shared memory, ~0 with dmabuf.
- VM: Debian genericcloud arm64 boots to SSH ≤ 8 s. virtio-blk randread IOPS within 10 % of
  libvirt's defaults. Snapshot save/load of a 4 GB guest ≤ 5 s.
- The app adds 0 to containers: no proxy, nothing in their path.

## Packaging

- `bande-velox` from `ci/new-app.sh velox Velox "Velox"`: `.desktop`, metainfo, theme icon in
  the desktop's `data/icons/src/apps.py`, README table. `ci/check-app.sh` and `ci/smoke.sh` as
  for every app. `apps/velox/smoke.sh` checks `--version` and that the readiness text renders
  with no engine and no `/dev/kvm` access.
- `Depends: $auto, fonts-…` only. `Suggests: podman, docker-compose, qemu-system-arm,
  qemu-utils, qemu-efi-aarch64, qemu-system-modules-opengl, passt` (swtpm and virtiofsd when
  those milestones land). The readiness screens and Install… are how a user finds what's
  missing. The package data (names per arch) lives in one table in `readiness.rs`.
- New workspace dependencies: `httparse` and `fatfs`, plus `qemu-display` or its trimmed zbus
  interfaces at V3. Each gets a one-line justification in the merge.
- Tests (the repo is at 482 of ~500 lines): one end-to-end test (about 40 lines) where the app
  starts headless with no engine socket and no KVM, and the accessibility tree shows both
  gated lines; and one parser test for chunked + stream framing (genuinely tricky). The budget needs ~60 lines freed or raised;
  see decision 7.

## Milestones (each one mergeable to apps `main` on its own; desktop items merge there first)

| # | Repo | What | Done when |
|---|---|---|---|
| A1 | apps | **Vertical slice:** the app from the template; `engine/http` + socket resolution + informer; Containers list grouped by Compose project with Start/Stop; engine readiness | `podman run`/`docker rm` in a terminal show up in the list live; `systemctl --user restart podman.socket` (or `docker`) reconnects and reconciles; 0 wake-ups/s idle measured; gated line shown with no engine |
| A2 | apps | Container actions, detail pane (Info, Logs with follow/search, Stats while visible), ports as links, Open in Terminal (`docker exec -it`) | logs follow without a timer; closing the pane ends the stats stream (checked with `ss`) |
| A3 | apps | Images (pull with job progress, tag, remove, prune, history), Volumes, Networks with "used by", Disk usage, engine info, Install… via PackageKit | `alpine` pulls with progress; prune frees what `/system/df` predicted; Install… on a clean machine ends Ready without restart |
| A4 | apps | Compose up/down/restart/logs via `docker compose` | a sample project goes up and down from the app, and grouping follows |
| V0 | apps | Spike (branch, merged as notes only): hand-written QEMU argv on the mini; Debian cloud image + seed; QMP `snapshot-save/load`; check `pmu`, `highmem`, P/E core pinning (`AllowedCPUs=4-7`) | the guest boots to SSH with times recorded; findings go into `vm/qemu.rs` defaults |
| V1 | apps | Virtualization readiness; VM core: catalog + download + seed, transient user unit, QMP events, Start/Shut Down/Force Stop/Pause, passt with an SSH port forward, guest-agent IP, Open in Terminal (`ssh`) | create→SSH ≤ 8 s; the VM survives Quit; state follows `systemctl --user stop`; SIGKILL during a write then `fsck` is clean |
| D1 | desktop | Toolkit external surface: a widget backed by a `wl_subsurface` that the app fills with memfd shm buffers + damage (dmabuf import next); client-side `keyboard_shortcuts_inhibit` and pointer lock/relative pointer in `bande-platform` | a test app shows a moving buffer at 0 CPU while still; Super reaches a focused, inhibited surface |
| D2 | desktop | Move Activity Monitor's graph painting (`paint_graph`, `Graph`) into `bande-widgets`; Monitor uses it | Monitor looks the same; Velox stats panes use the widget |
| V3 | apps | Embedded graphical console over D-Bus display (p2p), resize through `SetUIInfo`, keyboard capture/release, clipboard via `qemu-vdagent` | a Debian desktop guest is usable in a console window; a still guest costs 0 % CPU |
| V4 | apps | Snapshots (live `snapshot-save`/`load`/`delete` as jobs, tree with revert/delete) and clones (full via `FICLONE` else `qemu-img convert`, linked overlay with base guard; new MAC/UUID/instance-id) | revert restores RAM state; deleting a linked clone's base is refused |
| A6 | apps | `libs/terminal-view`: Terminal's emulator + renderer + keys taken over a byte stream, not only a pty; embedded exec and serial console use it | Terminal unchanged; exec and serial consoles work inside Velox |
| V5 | apps | ISO install with presets, virtiofs shares, USB hot-plug, swtpm + Secure Boot (Windows 11 ARM) | a Windows 11 ARM ISO installs with no manual flags |

D3 (desktop): the theme icon, with A1. The kernel needs no milestone: KVM, vhost and netfilter
are already in Debian's config. If V0 finds a guest timer, PMU or vGIC fault on the M2, that
becomes a patch topic in `kernel/patches/<series>/held/kvm/`.

## Risks

- **16K host pages:** some arm64 images (old jemalloc, Chromium-based tools) crash under native
  containers. The mitigation is to say so in the error and, later, offer a "container VM" (a 4K
  guest running Podman, as Asahi's muvm does). It's a v2 decision.
- **P/E cores:** vCPUs that migrate between Blizzard and Avalanche may confuse guests that read
  MIDR or timing. Pin VMs to P-cores by default if V0 shows a problem.
- **Partial vGIC and Apple PMU:** QEMU options that differ from generic arm64 (`pmu=off`,
  `highmem`). V0 pins them down.
- **Internal snapshots with UEFI:** raw AAVMF vars break `snapshot-save`; vars kept as qcow2
  avoid it. qcow2 internal snapshots grow the file, so delete must reclaim with `discard`.
- **`qemu-display` crate maturity and zbus version skew:** if it doesn't fit, write the 4–5
  interfaces ourselves with zbus.
- **Podman compat gaps:** `/system/df` fields and some event types differ. Test A1–A4 on both
  Podman and Docker CE.
- **Compose with Podman:** `docker compose` against the Podman socket is fine; podman-compose
  differs. Support the former.
- **Seat ACL:** `/dev/kvm` access ends when the seat goes to another session; running VMs keep
  their fd. VMs started over SSH need the `kvm` group.

## Owner decisions (each with a recommendation)

1. **Name:** keep "Velox" (`bande-velox`), your brand, which avoids Docker's trademark; or a
   generic Bande name such as "Containers". *Recommend Velox.*
2. **One app or two** (a container manager and a VM manager): *recommend one app*, as the brief
   says, with two sidebar groups each gated on its own.
3. **Which engines:** the brief requires docker-ce from Docker's repo. Docker publishes no
   Debian *testing* suite, and Bande is built on testing. *Recommend accepting any working
   Engine API* (Podman, `docker.io`, docker-ce, rootless), with rootless Podman as the one the
   app installs.
4. **Installing for the user:** the brief says to show steps and never install. Bande's Software
   installs through PackageKit and polkit. *Recommend an Install… button for Debian packages
   only* (Podman, QEMU stack) plus copyable steps; never Docker CE, never group changes.
5. **libvirt or direct QEMU:** *recommend direct QEMU + QMP + transient systemd user units*, no
   runner and no daemon (see Architecture).
6. **Image trust:** HTTPS + vendor checksums in v1 and signed SUMS via `gpgv` later, or
   signatures from day one. *Recommend checksums now, signatures in V5.*
7. **Test budget:** the apps repo is at 482/~500 lines. *Recommend raising the cap to ~560*
   for this app's one e2e test and one framing parser test, rather than cutting other apps'
   tests.
8. **Cut from v1:** CLI, remote SSH hosts, `.velox.local` DNS, bridged/NAT networking and
   `setup`, export/import, PCI passthrough, migration, x86 guests. *Recommend all of these
   cut*, revisited after V5.
