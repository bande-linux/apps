# Velox: containers and virtual machines, a native Bande app

From the owner's note `NOTES/docker_vm.md` (the "Velox for Linux" brief), adapted to Bande: one
`bande-app` app in this workspace, not a GTK repo with its own CLI. The brief's North Star stays:
a pure client that adds nothing to the engine's or the guests' datapath, zero idle wake-ups, a
small footprint and two areas, each gated on its own. The GUI is the important part: it ports
Velox's UX (`github.com/mikaelhug/Velox`, `Sources/VeloxApp`), not its Swift. Delete this plan
when its milestones are done.

App `apps/velox`, package `bande-velox`, id `org.bande.Velox`, name "Velox".

## Owner decisions (approved 2026-11-02)

| # | Decision |
|---|---|
| 1 | Name: **Velox** (`bande-velox`, `org.bande.Velox`) |
| 2 | **One app**, two areas (Containers and Virtual Machines), each gated on its own |
| 3 | **Any working Docker Engine API**: Docker CE (preferred), Debian's `docker.io`, rootless Docker, Podman's compatible socket |
| 4 | **The app never installs or configures an engine** and never runs `sudo`. When nothing is found it shows copyable setup steps, Docker CE first, Podman as the alternative. The same applies to the QEMU stack |
| 5 | **Direct QEMU** + QMP + transient systemd user units: no libvirt, no runner, no daemon of our own |
| 6 | Images verified by **HTTPS + vendor checksums** now, signed SUMS (`gpgv`) at V5 |
| 7 | Tests follow the apps budget of **~80–100 lines per app** (apps `CLAUDE.md` 1ac2b62) |
| 8 | **Cut from v1:** CLI, remote SSH hosts, `.velox.local` DNS, bridged/NAT networking and `setup`, export/import, PCI passthrough, migration, x86 guests |

## What the M2 mini gives us (checked read-only on 7.3-rc7+bande-apple-arm64-16k)

- **KVM works as shipped.** `CONFIG_KVM=y`, the `kvm` module is loaded, `/dev/kvm` exists, and
  the kernel boots at EL2. The vhost, TUN, bridge, nftables/NAT, overlay, user-namespace and
  memory-cgroup options are on too. All of it comes from Debian's `arm64-16k` config, so the
  kernel repo needs no change.
- **`/dev/kvm` access:** it is `root:kvm 0660` with udev's `uaccess` tag, so the user at the
  seat gets an ACL and needs no `kvm` group. The group matters only for SSH-only use.
- **CPU:** 4 Blizzard (E) and 4 Avalanche (P) cores. QEMU runs with `-M virt,gic-version=3
  -accel kvm -cpu host`. Guests are arm64 only.
- **Installed:** QEMU 11.1, AAVMF (with secboot/ms variants), `virtiofsd`, and
  `qemu-system-modules-opengl`, which ships `ui-dbus.so`.
- **In testing, not installed:** `podman` 5.8, `docker.io` 28.5, `docker-compose` 2.40,
  `passt`, `swtpm`.
- **Host pages are 16K.** Some arm64 images built with a 4K-page jemalloc crash in native
  containers.

## Scope (the brief's sections → v1)

| Brief | v1 | Later |
|---|---|---|
| §4 Containers, Compose grouping, actions, logs, inspect, stats, port links | yes | |
| §4 Exec terminal | Open in Terminal (`docker exec -it`, the brief allows it) | embedded via A6 |
| §4 Images, volumes, networks, `/system/df`, engine info, volume tar export/import | yes | |
| §4 Compose up/down/restart (spawn `docker compose`) | yes | |
| §4 Remote hosts, `.velox.local` DNS | | decision 8 |
| §5.1 Cloud images + cloud-init, UEFI | yes: Debian, Ubuntu, Fedora arm64 | |
| §5.1 ISO with presets, Windows 11 ARM + swtpm + Secure Boot | V5 | import formats |
| §5.2 Lifecycle, guest agent, stats; graphical console | V1; console V3 | autostart, clipboard, virgl |
| §5.2 virtiofs shares, USB hot-plug | V5 | VFIO, migration: out |
| §5.3–5.4 Live snapshots with tree; full/linked clones | V4 | disk-only `fsfreeze`, export/import |
| §5.5 passt with port forwards | yes | NAT/bridged/isolated |
| §6 Two readiness state machines, live on events | yes | |
| Velox menu-bar panel, ⌘K palette, crash notifications | | A7 |

## Architecture

There is no async runtime. Event fds (engine `/events`, QMP, inotify) are `Program` sources on
the main loop. Requests run on a worker that wakes the loop over a `UnixStream` pair, as in
Calendar's and Mail's `worker.rs`.

| Module | What it does |
|---|---|
| `engine/http.rs` | minimal HTTP/1.1 over `UnixStream`: content-length, incremental chunked decoding, hijack, Docker's 8-byte stream framing. New dependency: `httparse` |
| `engine/socket.rs` | socket order: `DOCKER_HOST`, the context in `~/.docker/config.json`, `$XDG_RUNTIME_DIR/docker.sock`, `$XDG_RUNTIME_DIR/podman/podman.sock`, `/run/docker.sock`, `/run/podman/podman.sock` |
| `engine/client.rs` | typed Engine API (serde): containers, images, volumes, networks, `system/df`, `info`, `version`, pull progress, stats, logs |
| `engine/informer.rs` | one `/events` stream; full re-list on every (re)connect; a store keyed by id that the views read |
| `vm/*` below | **built as the crate `bande-vm` in `libs/vm`** (V0, 2026-11-02): `Library`/`Machine`/`Manifest`, `Host` (argv), `Units`, `Qmp`, `ImageStore` + `catalog()`, `CloudInit`; the app's `vm/` keeps only the jobs and views |
| `vm/manifest.rs` | versioned JSON per VM, durable (temp + fsync + rename); corrupt JSON fails loudly |
| `vm/qemu.rs` | argv builder: `virt,gic-version=3`, `-cpu host`, AAVMF code + vars as **qcow2**, virtio-blk `cache=none,aio=io_uring,discard=unmap`, virtio-rng, passt NIC, guest-agent channel, `-sandbox on`, `-display dbus,p2p=yes` (V3; `-display none` until then) |
| `vm/unit.rs` | a transient systemd **user** unit per VM (`StartTransientUnit` over zbus) with `MemoryMax`, `CPUQuota`, `NoNewPrivileges`; lifecycle from `PropertiesChanged`; VMs outlive the app. passt runs in a second aux unit (`velox-vm-<id>-network.service`, `Type=forking`, `StopWhenUnneeded`) that the VM unit `Requires`/`After` |
| `vm/qmp.rs` | one QMP client: events (`STOP`, `RESUME`, `SHUTDOWN`, `JOB_STATUS_CHANGE`) and commands; a second `-qmp` socket for scripts |
| `vm/cloud.rs` | catalog data file (URL + SHA256/512), download job, `qemu-img` overlay, NoCloud `CIDATA` vfat seed (`fatfs`). Base images live in `$XDG_DATA_HOME/velox/images` (not the cache: overlays depend on them), named by checksum prefix |
| `readiness.rs` | one state machine, two providers; re-evaluated on inotify (`bande-watch`) of the socket folders, `/dev` and `/var/lib/dpkg/status`; first blocking state wins; the setup steps live in one table here |
| `jobs.rs` | one job/progress/cancel model: pull, download, create, snapshot, clone |
| `ui/` | `sidebar`, `overview`, `containers`, `images`, `volumes`, `networks`, `machines`, `new_machine`, `machine_window` (console, info, snapshots, settings), `logs_window`, `inspector`, `not_ready`, `settings` |

- **Live state:** containers follow `/events`; VMs follow QMP plus unit state; readiness
  follows inotify. Stats are streamed only for rows on screen and the open inspector or VM
  window.
- **No window open:** the app drops `/events` and QMP (so a socket-activated Podman exits) and
  reconciles on Reopen.
- **Podman:** the same Engine API client. Compose groups by `com.docker.compose.project`, which
  podman-compose sets too. Compose actions spawn `docker compose -p P --project-directory D -f …`
  as an argv, with `DOCKER_HOST` set.
- **Why not libvirt:** a resident or socket-activated daemon, XML, a second daemon for D-Bus,
  session mode still limited, and the console still comes from QEMU. crosvm and
  cloud-hypervisor lack Debian packages or arm64 UEFI, snapshots and display.
- **Console (`libs/vm/src/display.rs`, `machines/console.rs`):** QEMU's D-Bus display over a p2p
  connection (QMP `getfd` + `add_client`), our own zbus bindings. QEMU is the auth server on both
  the display and the listener socket. Frames arrive as the guest's memfd (`Listener.Unix.Map`) plus
  damage, shown with `ExternalSurface::show_shm`; when QEMU only has bytes (`Scanout`, `Update`), we
  copy them into a memfd of our own. Resize goes through `SetUIInfo`; input goes through USB
  (`usb-kbd`, `usb-tablet` on `qemu-xhci`, which the firmware understands too), so the pointer stays
  free and a click captures only the keyboard. The guest's hardware cursor becomes the Wayland cursor
  (`Cursor::Image`). The session closes when the tab or window goes away, so nothing runs for a
  hidden console. dmabuf waits for guest 3D (`gl=on`).
- **Clipboard:** `qemu-vdagent` on the virtio port `com.redhat.spice.0`; needs `spice-vdagent` in a
  guest X session. Guest copies are read at once; host copies are read when the window gets them,
  and anything marked as a password stays home. `spice-vdagent` wants grab serials counted from 0,
  and QEMU calls `Register` on us when the guest agent restarts, which starts them over.
- **Snapshots:** internal qcow2 with vmstate in `disk`, covering `disk` and `vars`; the tree (name,
  date, parent, memory) lives in the manifest. A running machine saves memory; a stopped one saves
  disks with `qemu-img`. Reverting to a memory snapshot while stopped starts QEMU with `-S`, then
  `snapshot-load` and `cont`: `-loadvm` picks `vars` for the vmstate and fails.
- **Clones:** a full clone is `qemu-img convert` keeping the base image. A linked clone freezes the
  source's disk as `images/linked-<id>.qcow2` and puts a new overlay on it for both machines, so
  either can be deleted in any order; the frozen layer goes when nothing uses it. That replaces the
  plan's "refuse to delete the base". A source with snapshots only clones in full. Both get a new
  MAC, UUID and seed (`instance-id`, hostname, `machine-id`).

## GUI

### What Velox has (studied from `Sources/VeloxApp`)

- **Shell:** one dashboard window (`NavigationSplitView`, 920×560).
  - Sidebar: Overview; Resources (Containers, Images, Volumes, Networks); Hosts; Workspaces;
    System (Engine Logs).
  - An engine status pill at the sidebar foot: state dot, label, Start/Stop/Resume.
  - The window title names the pane and the host.
- **Overview:** an engine strip (state, uptime, versions, **Reclaim Space**). Stat cards:
  Containers running, Images, Volumes, Disk. Live cards: CPU and Memory sparklines summed over
  running containers, Disk I/O and Network mirrored sparklines, streamed only while shown.
  - Reclaim Space sheet: toggles for stopped containers, unused images, build cache and unused
    volumes, each with its reclaimable size from `/system/df`, then "Reclaimed N".
- **Containers:** a table with **Name** (state dot, Compose project as a disclosure row with
  "2 of 3 running" or a shared pending action), **Image** (secondary, middle-truncated),
  **Status** (uptime badge "Up 3 hours" / "Exited (1) 5 min ago", or a pending badge
  "Starting…/Stopping…/Restarting…" with an orange dot), **Ports** (clickable `:8080` links,
  marked when the host port is blocked) and **CPU / MEM** (live, for visible rows).
  - Columns can be customised.
  - Toolbar: Start, Stop, Restart, Delete (selected), inspector toggle, "Filter containers".
  - Context menu: Start or Stop/Restart/Pause–Resume, View Logs, Open in Terminal, Copy ▸
    (Name, Container ID, As docker run Command, IP, each port URL), Delete…. Several selected:
    Start/Stop/Restart/Delete N Containers. A project: Start/Stop/Restart Project.
  - Keys: ⌘L logs, ⌘I inspector, ⌘⌫ delete.
  - Inspector: Container (image, state, created, restarts with a crash-loop hint), Process
    (command, workdir), Environment (count), Network (IP, port links), Mounts, Labels.
  - Empty state: "No Containers / Run one with `docker run`."
- **Images:** Repository, Tag, Arch capsule (arm64/amd64/multi), Created, Size.
  - Toolbar: an inline pull field "Pull image (e.g. nginx:latest)" with progress, Prune Unused
    Images, Remove Selected, filter.
  - Context menu: Run… (sheet: name, ports "8080:80", "Remove when it exits (--rm)"), Copy
    Reference, Copy Image ID, Remove.
  - Confirmations say what is kept: "an image still referenced by a container can't be removed".
- **Volumes:** grouped by Compose project like containers (badges "dangling", and "no
  containers" on a project). Columns: Name, Size, Created.
  - Toolbar: Prune Unused Volumes, Remove Selected, Import Volume (tar), inspector toggle.
  - Context menu: Copy Name, Copy Mountpoint, Export as tar…, Remove.
  - Inspector: Name, Driver, Size, Created, Mountpoint.
- **Networks:** a list of disclosures: name, driver · subnets, attached count; expanded, each
  container with its IP, or "No attached containers".
- **Logs:** a **separate window per container**. ANSI colours, stderr not tinted, tail 1000 then
  follow, reattach with `since`. Toolbar: Filter, auto-scroll toggle, wrap toggle, Copy all,
  Clear. Footer: "N of M lines".
- **Settings** (720×560, tabs General / Resources / File Sharing): launch at login, notify on
  crash, updates, Docker context, About + Copy Diagnostics; engine CPU/memory/swap/disk,
  Resource Saver.
- **Onboarding:** a sheet (Welcome → Checks → Finish).
- **Other:** a menu-bar panel (running containers with ports and Stop, CPU/MEM, Dashboard ⌘D,
  Settings ⌘,, Quit), a ⌘K palette (find a container/image/volume and act), and §13's rule:
  very short, professional text.

### What carries over, and what changes for Bande

- **Kept:** sidebar + table + inspector, Compose disclosure rows, the pending badges, port
  links, a logs window per container, Reclaim Space, the Run… sheet, the confirmation wording,
  short text, and streaming stats only while visible.
- **Dropped (Velox-only):** engine Start/Stop, Resource Saver, Workspaces, Engine Logs (on
  Linux they are the engine's journal), Docker contexts, the updater, and engine resources in
  Settings.
- **Changed:**
  - The sidebar status pill becomes a read-only engine pill: "Docker 28.5", "Podman 5.8 ·
    rootless", or "No engine".
  - The onboarding sheet becomes each area's not-ready view.
  - ⌘ becomes Ctrl; the menu-bar panel and palette wait for A7.
  - Every action is also in the menus (⋮ and the menu bar), so Menu Search finds it.
- **Bande frame:**
  - Single instance; closing the window leaves Velox running windowless (in the Taskbar, its
    menus working) until Quit.
  - No background mode in v1: containers and VMs run without the app.
  - Velox's own Settings window comes from `bande-app-settings`.
- **Widgets:** `Table` (sorting, column chooser, disclosure rows, as in Activity Monitor),
  `Form` for inspectors and settings, `SearchField`, `Segmented`, `Sheet`, `Alert`,
  `ContextMenu`, `Progress`, `Stepper`, `Slider`, `PopUpButton`, `TextView` (read-only, for
  logs).
  - The sidebar is the toolkit's `Table::sidebar()`, the graph its `Graph`, the console its
    `SurfaceView`.

### Main window

```
┌ ● ● ●  Containers                        [▶][■][↻][🗑]  [◧]  [🔍 Filter containers ]  ⋮ ┐
│ Overview          │ Name                 Image           Status           Ports    CPU  MEM│
│ CONTAINERS        │ ▾ ▣ shop             2 of 3 running                                      │
│   Containers   12 │    ● shop-web-1      shop-web        Up 3 hours       :8080 :8443  2% 41M│
│   Images        9 │    ● shop-db-1       postgres:17     Up 3 hours       —        1% 88M│
│   Volumes       4 │    ○ shop-worker-1   shop-worker     Exited (1) 5 min —        —    —│
│   Networks      3 │ ● redis              redis:8         Stopping…        :6379    —    —│
│ VIRTUAL MACHINES  │                                                                    │
│   Machines      2 │                                                                    │
│ ───────────────── │                                                                    │
│ ● Podman 5.8 · rootless                                                                │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

- The sidebar shows counts (running/total for Containers and Machines).
- The inspector toggle (◧, Ctrl+I) opens a ~280 pt right pane with the selection's `Form`. Its
  sections are as in Velox, plus **Stats**: CPU, memory, network and block I/O graphs (D2), for
  that container only, only while shown.
- Double-click or Return on a container opens its Logs window.

### Overview

```
│ Overview                                                         [Reclaim Space…] │
│ ● Docker 28.5 · API 1.51 · rootless       ● 2 machines running                    │
│ ┌Containers┐ ┌Images──┐ ┌Volumes─┐ ┌Disk────┐ ┌Machines┐                           │
│ │ 5 of 12  │ │ 9 · 3GB│ │ 4 · 1GB│ │ 6.2 GB │ │ 2 of 3 │                           │
│ ┌CPU ▁▂▄▃▂ 14%────────┐ ┌Memory ▃▃▄▄ 1.2 GB──┐ ┌Disk I/O┐ ┌Network┐                 │
```

Cards click through to their pane. Live cards sum containers and VMs, from engine stats and
the VM units' cgroups, only while Overview is shown.

### Images, Volumes, Networks

- **Images:**
  - Toolbar: [Pull field + Pull], Prune Unused, Remove, filter.
  - Columns: Repository, Tag, Arch, Created, Size, and **In Use** (a dot when a container
    references it, which Velox lacks; from the store).
  - Pull progress shows inline in the toolbar and as a job row.
  - Run… is a sheet as in Velox.
- **Volumes:**
  - As in Velox. Tar export/import goes through the archive API on a never-started helper
    container (an empty imported image, removed after), so no image is pulled.
  - The inspector adds **Used By** (containers), as the brief asks.
- **Networks:** a `Table` with disclosure rows (Name, Driver, Subnet, Containers). Expanded
  rows show the container and its IP. Create… (name, driver bridge, subnet) and Remove for
  user networks, as the brief asks.

### Virtual Machines (designed to match the container side)

**List** (sidebar Machines): the same table, toolbar and inspector pattern.

```
┌ ● ● ●  Machines                         [+][▶][⏸][■][🗑]  [◧]  [🔍 Filter machines ]  ⋮ ┐
│ …sidebar… │ Name             OS              Status             IP / SSH        CPU  MEM│
│           │ ● debian-dev     Debian 13       Running · 2 hours  localhost:2222   3% 1.1G│
│           │ ⏸ fedora-test    Fedora 43       Paused                              —    — │
│           │ ○ ubuntu         Ubuntu 26.04    Stopped                             —    — │
│           │ ◌ alpine-ci      Debian 13       Downloading… 43 %                          │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

- **States and dots:**
  - Running (green), Paused (yellow), Stopped (hollow).
  - Pending badges "Starting…/Shutting Down…/Saving…/Reverting…" (orange), like containers.
  - "Downloading… N %" / "Creating…" job rows.
- **Context menu:** Start/Resume, Pause, Shut Down, Force Stop, Open Console, Open in Terminal
  (`ssh -p`), Take Snapshot…, Clone…, Copy ▸ (Name, IP, SSH Command), Delete…. Several
  selected: Start/Shut Down N Machines.
- **Inspector:** Machine (OS, firmware, CPUs, memory, created), Disks (size on disk / virtual),
  Network (forwards, IP from the guest agent), Snapshots (count, latest), Stats graphs.
- **Empty state:** "No Virtual Machines" with [New Virtual Machine…].

**New Virtual Machine** (a sheet, File ▸ New Virtual Machine… Ctrl+N, three steps like Velox's
onboarding):

```
┌ New Virtual Machine ───────────────────────────────────────────┐
│ [ Cloud Image | ISO ]                                           │
│  ● Debian 13 "trixie"           arm64   350 MB                  │
│  ○ Ubuntu 26.04 LTS             arm64   600 MB                  │
│  ○ Fedora Cloud 43              arm64   500 MB                  │
│                                        [Cancel]  [Continue]     │
├ step 2: CPUs [4 ⇅]  Memory ──●── 4 GB  Disk ──●── 32 GB         │
├ step 3: Name [debian-dev]  User [mikael]  SSH key [id_ed25519 ▾]│
│           Forward SSH to host port [2222]       [Back] [Create] │
└─────────────────────────────────────────────────────────────────┘
```

- ISO (V5): a file picker plus a preset popup (Linux, Windows 11); no step 3.
- Create closes the sheet, selects the new row, shows its job, and starts the VM when it is
  ready.

**Machine window** (one per VM, like Logs windows; listed in the Window menu; opened by
double-click or Open Console):

```
┌ ● ● ●  debian-dev — Running         [ Console | Info | Snapshots | Settings ]  [⏸][■]  ⋮ ┐
│                                                                                       │
│                         guest display (D1 surface, resizes the guest)                 │
│                                                                                       │
│ Click to capture · Ctrl+Alt releases                                                  │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

- **Console:** the guest display. When the VM is stopped, a centred [Start]. An image with no display
  driver shows QEMU's "Display output is not active" (why the catalog has Debian's `generic` image,
  not `genericcloud`, whose kernel has no DRM or USB input).
- **Info:** the inspector's form at full size, with larger graphs.
- **Snapshots:** a tree table (Name, Date, Size, ◆ current) with [Take Snapshot…] [Revert…]
  [Delete…] [Clone…].
  - Revert asks "Revert to “before-upgrade”? Changes since it are lost."
  - Delete is refused while a linked clone depends on it, saying so.
- **Settings:** a `Form`: Name; CPUs; Memory; Disks (list, Grow…); Network (forward table: Host
  ↔ Guest port, [+][−]); Display (on/off).
  - Shared Folders and USB come at V5.
  - Rows that need the VM stopped are disabled with "Shut down to change".
- **Closing the window** never stops the VM.

### Not-ready states (replace onboarding; one per area, the other area unaffected)

Velox's §6 rule holds: the first blocking state wins, with one short line, then copyable steps
in a read-only monospaced `TextView` with [Copy], at most one link, and live re-evaluation.

```
│              Install a container engine to continue                        │
│  Docker Engine (recommended)                                  [Copy]       │
│  ┌──────────────────────────────────────────────────────────────┐          │
│  │ (Docker's apt repo for Debian, then)                         │          │
│  │ sudo apt install docker-ce docker-ce-cli containerd.io \     │          │
│  │   docker-buildx-plugin docker-compose-plugin                 │          │
│  │ sudo usermod -aG docker $USER                                │          │
│  └──────────────────────────────────────────────────────────────┘          │
│  docs.docker.com/engine/install/debian                                     │
│  ▸ Or Podman (rootless, from Debian)                                       │
```

- **Engine states:**
  - Not installed → the block above.
  - Not running → `sudo systemctl enable --now docker`, or `systemctl --user enable --now
    podman.socket`.
  - No access → the group step plus "The docker group is root-equivalent."
  - Re-login → "Log out and back in to finish".
  - Too old → "Update Docker to continue".
- **Virtualization states:**
  - "Install virtualization to continue" → `sudo apt install qemu-system-arm qemu-utils
    qemu-efi-aarch64 qemu-system-modules-opengl passt`.
  - "No access to KVM" (an SSH session without the seat ACL) → the `kvm` group step.
  - Re-login.
- **Inside a ready area,** a missing optional piece greys out only its control, with a one-line
  hint, e.g. "Snapshots need qemu-utils".

### Menus and keys (from ⋮ and the menu bar; `bande-app` supplies App, Edit, Window)

| Menu | Items |
|---|---|
| Velox | About Velox, Settings… Ctrl+, , Hide, Quit Ctrl+Q |
| File | New Virtual Machine… Ctrl+N, Pull Image…, Run Image…, Close Window Ctrl+W |
| View | Overview Ctrl+1, Containers Ctrl+2, Images Ctrl+3, Volumes Ctrl+4, Networks Ctrl+5, Machines Ctrl+6, Show/Hide Inspector Ctrl+I, Find Ctrl+F, Columns ▸ |
| Container | Start, Stop, Restart, Pause/Resume, Show Logs Ctrl+L, Open in Terminal, Copy ▸, Delete… Delete; for a project: Start/Stop/Restart/Down Project |
| Image | Run…, Remove…, Prune Unused…; Volume/Network items appear in the same menu slot for their pane |
| Machine | Start/Resume Ctrl+R, Pause, Shut Down, Force Stop, Open Console Ctrl+O, Take Snapshot… Shift+Ctrl+S, Clone…, Delete… |
| Window | Velox, then each Logs and Machine window |

There is also Reclaim Space… (File). Disabled items stay visible, as on macOS.

### Settings window (`bande-app-settings`)

- **General:**
  - Engine: Automatic (shows what was found and its socket), or a socket path.
  - "Open Logs in a new window" / reuse.
- **Machines:**
  - Storage folder (`$XDG_DATA_HOME/bande/velox/vms`, Change…; it never silently recreates
    missing disks).
  - Defaults for new machines (CPUs, memory, disk).
  - "Pin to performance cores" (default on if V0 says so).
- **About:** Velox's About comes from `bande-app`. Copy Diagnostics (versions, engine,
  readiness) goes in Help.

### GUI performance rules

- Rows update in place from events, with no flicker.
- Stats run only for visible rows, the open inspector and VM windows.
- Logs keep 10,000 lines per window; filtering runs on the kept lines.
- The console draws only on damage.
- Windowless: no streams.

## Security

- **No root at runtime, and nothing installed for the user** (decision 4). The `docker` group
  is root-equivalent; the not-ready view says so and offers Podman rootless.
- **QEMU sandboxing:** each QEMU runs in the user's own unit with the full `-sandbox on` set (passt outside it, V0),
  `NoNewPrivileges` and `MemoryMax`. No setuid bridge helper in v1.
- **Downloads** are checked against vendor checksums over HTTPS (decision 6).
- **Spawned processes** (`docker compose`, Terminal, `ssh`, `qemu-*`) get argv only, never a
  shell; names and labels are data.
- **Disks:** never mutate a disk a running VM has attached, and never `cache=unsafe`.

## Performance targets (measured on the mini with `bande-background` and the bench scripts)

- **Idle:** 0 wake-ups/s, both with a static window and windowless. Windowless private memory
  ≤ 4 MB, as Terminal's 3.4 MB.
- **Responsiveness:** first list of 100 containers ≤ 150 ms; 1,000 rows scroll smoothly.
- **Console:** 0 % CPU when still; 1080p at 60 fps ≤ 5 % of a P-core with shared memory, about
  0 with dmabuf.
- **VMs:** Debian genericcloud boots to SSH ≤ 8 s; virtio-blk IOPS within 10 % of libvirt's
  defaults; snapshot save/load of 4 GB ≤ 5 s.
- **Containers:** Velox adds nothing; it is never in their path.

## Packaging and tests

- **Package:** `ci/new-app.sh velox Velox "Velox"`.
  - Theme icon in the desktop's `data/icons/src/apps.py`, plus the README table.
  - `ci/check-app.sh` and `ci/smoke.sh`, and `apps/velox/smoke.sh` (`--version`).
- **Dependencies:**
  - `Depends: $auto, fonts-…` only.
  - `Suggests: docker-compose, podman, qemu-system-arm, qemu-utils, qemu-efi-aarch64,
    qemu-system-modules-opengl, passt` (plus swtpm and virtiofsd at V5). Docker CE is never
    declared.
- **New crates:** `httparse`, `fatfs`, and `qemu-display` (or its interfaces) at V3.
- **Tests (~80–100 lines):**
  - One e2e test: start headless with no engine socket and no KVM, and check that the
    accessibility tree shows both not-ready lines and their Copy buttons.
  - One parser test for chunked decoding plus stream framing.

## V0 findings (2026-11-02, mini, QEMU 11.1.2, passt 2026-10-02, Debian 13 genericcloud)

- **KVM over SSH (solved since):** `/dev/kvm` is `root:kvm 0660` plus a uaccess ACL for the
  seated session, and the seat had only the greeter, so an SSH session gets `EACCES`. The
  spike ran on **TCG** (`-accel tcg,thread=multi,tb-size=256 -cpu max,pauth-impdef=on`).
  KVM needs the owner's seated session, or `kvm` group membership for SSH-only use (owner's
  call). `Accel::detect()` picks KVM when `/dev/kvm` opens read-write.
- **TCG times (4 vCPU, 2 GiB):** QEMU initialised (unit active + QMP greeting) in 0.08–0.2 s;
  the first boot with cloud-init took 69 s to "Cloud-init finished" and SSH; a warm boot took
  36–72 s to multi-user (load-dependent). The ≤ 8 s target needs a KVM measurement.
- **Snapshots (internal, qcow2 disk + qcow2 vars):** `snapshot-save` with 0.9 GB of dirty RAM
  took 0.35–0.9 s, `snapshot-load` 0.4–2.2 s (TCG), and `snapshot-delete` ~15 ms. RAM state
  really reverts: a file written after the save was gone after the load, and SSH reconnects
  through passt. The save brackets itself with `STOP`/`RESUME`. vmstate goes to `disk`;
  devices are `disk` + `vars`. **Delete frees clusters inside the qcow2 but does not shrink
  the host file** (`du` stays at the high-water mark; the next save reuses the space).
  Reclaiming needs an offline `qemu-img convert`: V4 shows the size and offers Compact….
- **`-sandbox on` breaks QEMU's `-netdev passt`:** the seccomp filter is inherited by the
  passt that QEMU spawns, which dies with SIGSYS. passt therefore runs as its own daemon
  (`--socket … --one-off --pid …`, which forks when ready) and QEMU joins it with
  `-netdev stream,addr.type=unix`. That allows the full
  `-sandbox on,obsolete=deny,elevateprivileges=deny,spawn=deny,resourcecontrol=deny`.
  Port forwards bind `127.0.0.1` only (`--tcp-ports 127.0.0.1/H:G`).
- **`-daemonize` is incompatible** with `spawn=deny` and `elevateprivileges=deny` (QEMU exits
  1 without a message), so the unit is `Type=exec`. Readiness is event-driven: inotify on the
  runtime folder for the QMP socket, plus a pidfd on `MainPID`, then the QMP greeting, which
  QEMU sends only once init is done; EOF means QEMU failed. A socket file alone is not
  enough, because chardevs are created before the block devices are opened.
- **`-mon` is deprecated in QEMU 11:** use `-object monitor-qmp,chardev=…`.
- **systemd** emits every `ActiveState` change twice, plus an `inactive` when a transient
  unit loads; `UnitEvents` drops repeats. `system_powerdown` ends the unit cleanly (passt
  exits with `--one-off`), and so does `systemctl --user stop` (QEMU exits on SIGTERM).
- **PMU (from the kernel source, `arch/arm64/kvm/pmu-emul.c`; not measured):** the M2 PMU is
  IMPDEF, and KVM spoofs a minimal PMUv3 through `ARM64_WORKAROUND_PMUV3_IMPDEF_TRAPS`, so
  `-cpu host` keeps `pmu=on`. QEMU sets no PMU type, so KVM takes the PMU of the core where the
  vCPU was initialised and lets vCPUs run on both clusters. Guest counters just stop on the
  other cluster, which is harmless.
- **Core pinning:** none by default. Pinning to the P-cores (CPUs 4–7) would only keep guest
  perf counters steady and would starve the desktop. Revisit if a KVM run shows guest timer
  trouble.
- **highmem:** left at QEMU's default. The guest under TCG saw 4 vCPUs (`CPU part 0x051`, `max`)
  and booted UEFI from qcow2 vars. Under KVM, QEMU clamps the high regions to the host IPA
  size by itself, which is unconfirmed until a KVM run.
- **First KVM run (2026-11-02, bande-vm's own argv, QEMU run through `sg kvm` because the
  lingering user manager predates the owner's `kvm` group):**
  - Time to SSH: **21.9 s** on the first boot with cloud-init, **16.5 s** warm (4 vCPU, 4 GiB,
    Debian 13). The guest's kernel + userspace take 7.1 s (`systemd-analyze`), so ~9 s go to
    QEMU, AAVMF and GRUB before the kernel. The ≤ 8 s target is not met; the next step is to
    time the firmware and try a shorter UEFI path (boot order, GRUB timeout, direct kernel).
  - Guest `dmesg`: PMU `armv8_pmuv3_0` with 2 counters (KVM's spoofed PMUv3); `arch_timer`
    virtual timer at 24 MHz; GICv3 with ITS, a redistributor per vCPU; PSCI 1.3; vCPUs report
    MIDR `0x611f0320` (Blizzard, the E-core where they were initialised). One
    `hrtimer: interrupt took 12 ms` at boot, nothing else.
  - E/P migration: with 4 busy guest vCPUs, the vCPU threads were moved between CPUs 0–3 and
    4–7 every 0.5 s for 60 s (116 moves): no new guest messages, no fault, and the guest clock
    stayed within SSH latency of the host's. No pinning needed.
  - highmem: QEMU drops the high PCIe window for the M2's IPA size by itself; the guest has only
    the 32-bit window `0x10000000–0x3efeffff` and RAM above 4 GiB (`0x47ff0000–0x13fffffff`).
    Fine for virtio; large BARs (virtio-gpu blobs at V3) will need a check.
- **passt and AppArmor:** Debian's `passt` profile only lets it write under `/tmp`, `/var/tmp`
  and `$HOME`, so a socket in `$XDG_RUNTIME_DIR` fails with `EACCES` (the spike used an
  unconfined copy). bande-vm keeps passt's socket and pid file in a private
  `/tmp/velox-<uid>/<id>` (0700, owner checked).
- **KVM in the user manager:** the VM's QEMU runs under `user@<uid>.service`, so it has the
  manager's groups, not the app's. Readiness checks the manager's groups (and the seat ACL) and
  shows "Log out and back in to finish" when only the session or `/etc/group` has `kvm`; with
  lingering on, only a restart refreshes the manager.

## Milestones (each mergeable on its own; desktop items merge there first and arrive with a bump)

| # | Repo | What | Done when |
|---|---|---|---|
| V5 | apps | Left of V5: USB passthrough (`usb-host` through QMP `device_add`), and a Windows 11 ARM install checked end to end | a Windows 11 ARM ISO installs with no manual flags |
| A7 | apps | Velox extras: Ctrl+K palette, crash notifications (`bande-notify`), optional status item via the tray with the background mode (as Calendar's) | each off by default and measured at 0 wake-ups while idle |

Done and merged: A1–A4, V0, V1, D1 (`SurfaceView`), D2 (`Graph`), V3 and V4. D4 needs no widget of
its own: `Table::sidebar()` is the shared source list, and Velox, Settings and the file chooser use it.

Also done: A6 (`libs/terminal-view`: Terminal's emulator and renderer with a `TerminalView` widget;
Velox opens a machine's serial console and a container shell with it) and most of V5: an ISO path
in the New Virtual Machine sheet with Linux and Windows 11 presets, shared folders over virtiofs,
swtpm and Secure Boot firmware for Windows, and gpgv-checked vendor checksums.
- **ISO machines:** blank qcow2 disk first in the boot order, the ISO as a removable USB disk second,
  so the firmware installs from it and later boots the disk; Settings ▸ Eject drops it. No seed.
- **Windows 11 preset:** `AAVMF_CODE.ms.fd` with Microsoft keys enrolled in the vars, `ramfb` for
  the picture (Windows has no virtio-gpu driver), NVMe for the disk, `tpm-tis-device` on swtpm. No
  network driver during setup (virtio-win is a separate download). Not run: no Windows ISO and no
  swtpm on the mini.
- **Helpers:** swtpm and virtiofsd run in their own transient units, started before QEMU and
  waited for by their socket; they exit with QEMU. A machine with shared folders cannot take
  snapshots (vhost-user-fs does not migrate).
- **Checksums:** Ubuntu's `SHA256SUMS.gpg` and Fedora's clearsigned `CHECKSUM` are checked with
  `gpgv` against the vendor keys in `libs/vm/data`. Debian publishes no signature beside
  `latest/SHA512SUMS`, so Debian stays HTTPS-only.

Measured on the mini (KVM, Debian 13 generic, 4 vCPU, 4 GiB):
- **Boot to SSH: 8.4 s** from the unit's start (firmware 0.7 s, GRUB and kernel load 1.6 s, Linux to
  login 5.8 s). AAVMF waited 5 s for a boot key until `-boot menu=on,splash-time=0`.
- **Snapshots:** saving 2.3 GB of dirty memory took 4.6 s, loading it 8.8 s; Suspend took 8.7 s and
  resuming to SSH 10.2 s with 2.7 GB in use.
- **3D:** glmark2-es2 (off-screen, DRM) scores 899 with virgl on the M2 GPU against 127 with the guest's
  llvmpipe, about 7×. Showing the 3D picture needs a compositor that imports the host GPU's dmabufs;
  the headless lab (lavapipe) cannot, so Velox says so instead of being dropped by the compositor.
- Not measured: 1080p at 60 fps and a release build's cost (a debug build used about 1.7 % of a core
  with a still guest and the main window open).
- **Guest resize:** the guest is offered the window's size as its preferred mode (`SetUIInfo`);
  GNOME and KDE apply it, Xfce needs `xrandr --auto`. QEMU's own vdagent sends no monitor
  configuration, so `spice-vdagent` cannot apply it either.

D3 (desktop): the theme icon, with A1; the app ships the same SVG for hicolor. **Kernel: none.** If V0 hits a guest timer, PMU or vGIC
fault, it becomes a held topic in `kernel/patches/<series>/held/kvm/`.

## Audit against macOS Velox and VMware Workstation Pro (2026-11-07)

| Feature | State | Why |
|---|---|---|
| macOS Velox: dashboard, containers, images, volumes, networks, logs, inspector, Compose, Reclaim Space | done (A1–A4) | |
| macOS Velox: menu-bar panel, ⌘K palette, crash notifications | not done (A7) | each needs a Settings switch, and Velox has no Settings window yet |
| Built-in monitor | done | per container (inspector) and per machine (Info tab): CPU, memory, disk and network graphs, sampled every 2 s only while shown; network comes from the guest agent |
| NAT networking with port forwards | done | passt, unprivileged |
| Bridged, host-only, network editor | left out | bridges need root or a setuid helper (decision 4: Velox never configures the system); passt cannot isolate yet |
| Shared folders | done | virtiofs; disables snapshots (vhost-user-fs does not migrate) |
| USB passthrough | not done | `usb-host` needs write access to `/dev/bus/usb`, which seat ACLs give only for some devices; untestable here without taking a device from the owner's session |
| Clipboard | done | qemu-vdagent + spice-vdagent, text only |
| Drag and drop of files | left out | QEMU's vdagent has no file transfer; shared folders cover it |
| Several guest displays | left out | needs a window per `Console_N`; later |
| Suspend to disk and resume | done | an internal snapshot `velox-suspended`, loaded and deleted on the next start |
| Snapshots and clones (full, linked) | done (V4) | |
| Templates | covered by clones | a stopped machine cloned linked is a template |
| Encryption | left out | qcow2 LUKS needs a secret store per machine; the disk sits in the user's home, which LUKS on the system disk already covers |
| TPM and Secure Boot | done (Windows preset) | swtpm; not run without swtpm here |
| OVF import and export | left out | OVA/VMDK carry VMware/VirtualBox hardware; a disk-only import through `qemu-img convert` is the useful part, later |
| Autostart at login | left out | VMs already outlive the app; autostart needs persistent user units rather than transient ones |
| Guest tools | done for cloud images | `qemu-guest-agent` and `spice-vdagent` through cloud-init |
| 3D graphics | done, off by default | `virtio-gpu-gl-pci` with virgl on the host render node, dmabuf scanouts; falls back to the plain display when QEMU will not start with it |

## Risks

- **16K host pages:** some arm64 images crash in native containers. Name the cause in the error.
  A later fix is a 4K "container VM" (as Asahi's muvm).
- **P/E core migration** may confuse guests; pin to P-cores if V0 shows a problem.
- **Partial vGIC and Apple PMU** may need non-default QEMU options; V0 settles them.
- **Internal snapshots with UEFI** need the vars as qcow2; internal snapshots grow files, so
  delete must discard.
- **`qemu-display` maturity and zbus skew:** fall back to our own bindings.
- **Podman compat gaps** (`/system/df`, event types): test A1–A4 on Podman and Docker CE.
- **Seat ACL:** `/dev/kvm` access follows the active seat session, though running VMs keep
  their fd. VMs started over SSH need the `kvm` group.
- **The GUI is large:** four widgets the desktop lacks (source list, graph, external surface,
  and the inspector split, done in-app) gate the polish. A1–A3 use in-app versions where needed
  and switch when D2/D4 land.
