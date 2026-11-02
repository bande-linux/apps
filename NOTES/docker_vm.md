# Agent brief: build Velox for Linux (Rust): native Docker manager + KVM/QEMU VM manager

> **Who reads this:** an autonomous coding agent starting a new repository.
> **What you're building:** a Linux desktop + CLI program written **100% in Rust**. It has two
> parts:
>
> 1. **A Docker manager for the host's native Docker Engine.** It does not run a VM for Docker.
>    Docker must already be installed from **Docker's official repository**. If it isn't, the
>    program says so ("Install Docker to continue") and never installs anything itself.
> 2. **A KVM/QEMU virtual-machine manager.** It covers the full set of common features:
>    create, console, snapshots, clones, export/import, shared folders, networking, UEFI/TPM,
>    and so on. It is a **parallel path** to the Docker part, with its own gate: if KVM/QEMU
>    isn't installed or usable, the VM area says "Install virtualization to continue" and shows
>    the install steps.
>
> The two parts are independent. Either one works while the other is gated.
>
> **Primary host:** Debian stable (13 "trixie"), amd64. arm64 and Ubuntu LTS come second.
> **How to use this brief:** read all of it once. Then run Phase 0, which turns this brief into
> the new repo's `CLAUDE.md` and ADRs. After that, work phase by phase and stop at every **exit
> gate**. Section 11 lists the decisions you must hand back to the human.

---

## 1. The reference implementation

*Velox* (`github.com/mikaelhug/Velox`, LGPL-2.1) is a lean Docker Desktop alternative for
macOS, written in Swift. Its **engine VM, guest kernel, `vinit`, port forwarder and Docker-socket
proxy do NOT apply here**: on Linux, Docker runs natively. What does carry over is its
**product design, GUI, Docker-API discipline and engineering conventions**. Clone it read-only
and study these:

| Read | Why |
|---|---|
| `CLAUDE.md` | The binding conventions. Keep the spirit of §8 (event-driven), §10 (one mechanism per job), §12 (remote hosts over SSH) and §13 (short GUI text), the release rules and the North Star. Ignore everything about the guest VM, VZNAT, the porthelper and workspaces. |
| `Sources/VeloxApp/**` | The GUI to match: the dashboards (containers, images, volumes, networks, compose grouping), logs, stats, settings, onboarding. Port the UX, not the Swift. |
| `Sources/VeloxCore/API/*` | `DockerClient`, `HTTPCodec`, `DockerEventHub`, models. How to talk to the Docker API over a raw unix socket with persistent `/events` streams and correct cancellation. |
| `Sources/VeloxCore/Proxy/DockerEventsWatcher.swift`, `NameDNSResponder.swift`, `NameRegistry.swift`, `SSHTunnel.swift` | The informer pattern, `<name>.velox.local` DNS, and remote hosts over SSH. |
| `Sources/VeloxCore/Support/*` | File locks, durable JSON manifests, the updater with signature verification, the remote-host store. |
| `git log` | The *why* behind every rule. Grep it before you re-derive a decision. |

---

## 2. North Star

Be **as lean, fast and small-footprint as possible**:

- the GUI's cold start, idle RAM and idle CPU (wakeups/s);
- the CLI's latency;
- the VM's boot time, disk IOPS, network throughput and snapshot/restore time;
- the installed size.

Peers to beat, or at least match:

| Area | Peers |
|---|---|
| Docker management | Docker Desktop for Linux (GUI footprint), Portainer, lazydocker (responsiveness) |
| VM management | virt-manager + libvirt, GNOME Boxes, Quickemu. Match libvirt's tuned defaults on VM performance; beat all three on footprint and UX. |

The overriding rule for the Docker side: **the program must add zero overhead to Docker
itself.** It is a *client*. It doesn't proxy the Docker socket, wrap the `docker` CLI, or sit
in any container's datapath. Containers run at native speed because nothing of ours is in
their path.

**Nothing ships on a performance claim that wasn't measured on this repo's benchmark harness.**

---

## 3. Binding architecture (the pillars)

1. **100% Rust, no daemon of our own.**
   - Our binaries: `velox` (CLI), `velox-gui` (GTK4 + libadwaita, via `gtk4-rs`), and
     `velox-vm` (the per-VM runner, §3.4).
   - We may spawn these **system binaries** from Debian packages as transient children:
     `qemu-system-*`, `qemu-img`, `virtiofsd`, `swtpm`, `passt`, `qemu-bridge-helper`, `ssh`.
   - We spawn the `docker` CLI **only** for "open in terminal" hand-offs, never for data.
   - Nothing resident runs when no VM is running and the GUI is closed.
2. **Docker is native, official, and never touched.**
   - We never install, upgrade, configure or restart Docker. That means no edits to
     `daemon.json`, no `systemctl` on `docker.service`, and no changes to the user's
     contexts.
   - We talk to the Docker Engine API over its unix socket. Resolve it the way the docker CLI
     does: `DOCKER_HOST`, then the current context in `~/.docker/config.json`, then rootless
     `$XDG_RUNTIME_DIR/docker.sock`, then `/var/run/docker.sock`.
   - Rootless Docker (Docker's official `docker-ce-rootless-extras`) is supported.
3. **Readiness is a first-class state machine with two parallel instances, Docker and
   Virtualization (§6).** If a part isn't usable, its area shows exactly one short message plus
   its fix: a docs link for Docker, copyable install steps for virtualization. Each state
   updates **live, on events**, when the user installs or starts something, with no restart of
   our program. Both instances use **one** readiness mechanism (§3.9) with two providers.
4. **VMs run on QEMU + KVM directly, controlled over QMP, supervised by systemd.** No libvirt
   in the default design (an ADR may revisit this, §11).
   - Each running VM is a **transient systemd user unit** created over D-Bus
     (`StartTransientUnit`, via `zbus`).
   - The unit runs `velox-vm <id>`. This small Rust runner spawns `qemu-system-*`, owns
     QEMU's QMP socket, and exposes **one** control socket at
     `$XDG_RUNTIME_DIR/velox/vms/<id>.sock`. GUI and CLI clients connect there and get both
     commands and pushed events. QEMU's QMP serves one client per socket, so the runner is
     what lets the GUI and the CLI watch the same VM at once.
   - What systemd gives us for free: supervision, journald logs, cgroup accounting and limits
     (`MemoryMax`, `CPUQuota`), and clean stop on logout. Autostart at boot uses
     `loginctl enable-linger` plus an enabled user unit.
   - VMs keep running when the GUI closes.
5. **No root at runtime.** The user needs:
   - membership in the `kvm` group for `/dev/kvm`;
   - for bridged/NAT tap networking, Debian's **`qemu-bridge-helper`** (setuid, allow-list in
     `/etc/qemu/bridge.conf`). This is the native-first way to attach a tap to a bridge
     without our own root helper.

   Missing prerequisites are stated in the UI with the exact one-line fix (§6.2). We never run
   `sudo` for the user, except for an optional `velox setup` that the user starts explicitly. It
   writes `bridge.conf`, the systemd-resolved drop-in for named access and, if chosen, the NAT
   bridge (§7.4), and it has a complete `velox setup --undo`.
6. **VM disks are durable.**
   - Disks are **qcow2** (needed for internal snapshots, linked clones and compressed
     export), attached as virtio-blk or virtio-scsi with `cache=none` + `aio=io_uring` (or
     `native`) + `discard=unmap`.
   - **Never** use `cache=unsafe` or anything that skips flush. Velox lost user data to
     exactly this kind of "optimization".
   - Test it: kill QEMU with SIGKILL mid-write, reboot, and run `fsck`; the result must be
     clean.
7. **All versions live in one place: `versions.env`.** That means the app version, the
   minimum QEMU / virtiofsd / swtpm / OVMF and Docker API versions we require, and pinned
   build images. Rust reads them through a **generated** `versions.rs`; never hand-edit it.
   The cloud-image catalog (§7.1) is a data file with checksums, verified at download.
8. **Event-driven, never polling.**

   | Need | Event source |
   |---|---|
   | Containers | Docker `/events` over one persistent connection, then a **full reconcile** on every (re)connect: the informer rule |
   | VM state | QMP events (`STOP`, `RESUME`, `SHUTDOWN`, `JOB_STATUS_CHANGE`, `BLOCK_JOB_*`, …) relayed by the runner |
   | VM unit lifecycle | systemd D-Bus `PropertiesChanged` |
   | Docker / virtualization install and start detection | `inotify` on the socket directories, `/dev` (for `/dev/kvm`) and `/var/lib/dpkg/status` |
   | Child processes | `pidfd` |
   | Host suspend/resume | logind `PrepareForSleep`, then `guest-set-time` via the QEMU guest agent |
   | Host network changes | netlink |

   Stats: subscribe to Docker's streaming `/stats` only for **visible** containers. VM stats
   come from cgroup files and QMP, sampled only while a stats view is visible. The only timers
   allowed are genuine delays and visible-only UI ticks.
9. **One mechanism per job (Velox `CLAUDE.md` §10).** Write one HTTP/1.1 + Docker API client,
   one QMP client, one relay (for serial-console and agent sockets), one DNS responder, one
   job/progress model for every long operation (pull, export, import, clone, snapshot,
   convert), and one durable-manifest store. Extend these; never write a second.
10. **One GUI vocabulary.** Docker objects and VMs share the same list/detail/inspector
    patterns, the same job/progress UI, the same log viewer and the same terminal widget
    (`vte4`). GUI text is short and professional.

---

## 4. Part A: the Docker manager

The feature set, ported from Velox's GUI:

- **Containers.** List, grouped by Compose project (`com.docker.compose.project` label).
  Actions: start, stop, restart, pause/unpause, kill, remove. Plus live logs with follow and
  search, an exec terminal in an embedded `vte4`, inspect, per-container stats, published
  ports as clickable links, and `<name>.velox.local` links.
- **Images.** List, pull with progress, tag, remove, prune dangling/unused, and history/inspect.
- **Volumes and networks.** List, create, remove, prune, inspect, and show which containers
  use each one.
- **Disk usage** via `/system/df`, with prune actions. **Engine info** via `/info` and
  `/version`.
- **Compose projects.** Up/down/restart/logs for a project, by spawning `docker compose` with
  `--project-directory` from the labels. This is a hand-off, not a reimplementation.
- **Remote hosts over SSH** (Velox §12, kept almost word for word):
  - `ssh -L <local.sock>:<remote.sock>`, one `ssh` child per connected host.
  - `BatchMode=yes`, **no stored credentials ever**, never `StrictHostKeyChecking=accept-new`.
  - Every value that reaches a shell is shell-quoted.
  - A remote host is just another socket connector for the same client, stores and views.
- **Named access** `<name>.velox.local`. On Linux, container IPs on Docker bridges are
  already routable from the host, so this is **pure DNS**: a loopback DNS responder fed by
  the `/events` informer, plus a systemd-resolved routing domain `~velox.local` (installed by
  `velox setup`). The **same** responder also answers VM names (§7.4). Without resolved, the
  feature is off and its status says so.

**Out of scope:** installing or upgrading Docker; editing daemon config; Velox's workspaces
(they would require moving dockerd's data-root, which changes the user's Docker install);
Resource Saver; any engine VM.

---

## 5. Part B: the VM manager (KVM/QEMU)

The VM area is gated by the virtualization readiness check (§6.2) and is independent of
Docker's state.

### 5.1 Creating VMs
- **From cloud images.** A built-in catalog (Debian, Ubuntu, Fedora, Alpine, …) with
  checksums that are verified (signed `SHA256SUMS` where upstream signs them). Configured by
  **cloud-init** through a generated NoCloud seed (user, SSH key from `~/.ssh/*.pub`,
  hostname, packages). Target: boot to SSH in seconds.
- **From an ISO**, with OS presets (Linux, Windows 10/11, BSD) that pick a sane machine type,
  firmware, disk bus, NIC model and Hyper-V enlightenments.
- **Import** an existing image: qcow2, raw, vmdk, vdi, vhdx (converted with `qemu-img`), or
  OVA/OVF (§5.4), or our own bundle.
- **Firmware:** UEFI via OVMF/AAVMF (default) or legacy BIOS (SeaBIOS). Secure Boot via the
  `ovmf` `*.secboot` + `*.ms` vars, and **TPM 2.0** via `swtpm`. Windows 11 must install with
  no manual flags.
- **Hardware settings:**
  - vCPUs, sockets/cores/threads, CPU model (`host` default);
  - RAM, plus the balloon with free-page reporting;
  - multiple disks (add, remove, resize grow-only with a guest hint) and CD-ROMs;
  - NICs and the virtio-rng;
  - display (virtio-gpu, plus virgl when available) and sound;
  - USB controller.

### 5.2 Running and using VMs
- **Lifecycle:** start, graceful shutdown (ACPI, or the guest agent when present), force off,
  reboot, pause/resume, save state/restore (suspend to disk via a live snapshot, §5.3), and
  autostart at login or at boot (linger).
- **Graphical console embedded in the GUI.** Evaluate QEMU's **D-Bus display** (`-display
  dbus`) with the Rust `qemu-display` / `rdw` crates against SPICE (`remote-viewer` fallback),
  and record the result as an ADR. Required: clipboard sharing, dynamic resize (spice-vdagent
  / virtio-gpu), and keyboard grab/release.
- **Serial console** in the embedded `vte4` terminal.
- **QEMU guest agent integration:** IP addresses, hostname, OS info, graceful shutdown,
  `guest-set-time` after host resume, and `fsfreeze` around disk snapshots.
- **Shared folders** via `virtiofsd` (this requires a `memfd` shared-memory backend). There's
  a one-click mount hint for Linux guests, and the virtio-win driver ISO for Windows.
- **USB device passthrough:** hot-plug a host USB device into a running VM, with a clear
  device-permission message (udev/uaccess) when it can't be opened. **PCI/VFIO passthrough**
  is advanced and post-1.0 unless the human says otherwise.
- **Per-VM stats:** CPU, RAM, disk I/O and network I/O from cgroups + QMP, visible-only.
- **Clipboard/SSH convenience:** `velox vm ssh <name>` and `ssh <name>.velox.local`.

### 5.3 Snapshots
- **Live snapshots including RAM and device state** (the "save everything" snapshot):
  QMP `snapshot-save` / `snapshot-load` / `snapshot-delete` jobs (QEMU ≥ 6.0), stored as
  qcow2 internal snapshots on **every** writable disk.
- **Disk-only snapshots of a running VM**, with the guest agent's `fsfreeze` so they're
  crash-consistent at the filesystem level.
- **Offline snapshots** for stopped VMs.
- A **snapshot tree** in the GUI (parent/child, name, description, timestamp, size) with
  revert, delete and "clone from snapshot".
- Rules: snapshot operations are QMP **jobs** that show progress in the shared job UI. Reverting
  asks for confirmation and states what is lost. Never delete a snapshot that a linked clone
  depends on (§5.4).
- An ADR records internal (qcow2-internal) snapshots against external overlay chains. The
  default is internal: simplest, single-file, and supported by `snapshot-save`. Revisit it if
  measurements show a problem.

### 5.4 Clone, export and import
- **Full clone:** `FICLONE` reflink when the filesystem supports it (btrfs, XFS), else
  `qemu-img convert` with sparseness preserved. **Never a hard link.**
- **Linked clone:** a qcow2 overlay with a backing file. The base becomes read-only and is
  tracked, so it can't be deleted while clones depend on it.
- Every clone regenerates the MAC, the VM UUID, the SMBIOS serial and the cloud-init
  `instance-id`, and offers to reset the machine-id/SSH host keys via cloud-init.
- **Export:**
  - (a) **Our bundle** (`.veloxvm`, a tar): the VM manifest, compressed qcow2 disks
    (`qemu-img convert -c`), optional snapshots, NVRAM vars and TPM state.
  - (b) **OVA/OVF** with streamOptimized VMDK, for VirtualBox and VMware.
  - (c) **A single disk image** as raw, qcow2, vmdk, vdi or vhdx.

  Export works on a stopped VM, or on a running VM from a snapshot.
- **Import:** the reverse of all three, with a hardware-mapping review step for OVF.
- All of these are cancellable jobs with progress. A cancelled or failed job leaves **no**
  half-written files: write to a temp name in the same directory, then `rename`.

### 5.5 VM networking
Pick per NIC:
- **User-mode (default, rootless):** `passt` (QEMU's modern user networking, Debian package
  `passt`), with port forwards configurable in the GUI. It's slower than tap and fine as a
  default.
- **NAT bridge:** a host bridge with DHCP and masquerade, created once by `velox setup`. Phase
  4 has an ADR choosing between systemd-networkd `DHCPServer=` + `IPMasquerade=`, a
  NetworkManager shared connection, or reusing libvirt's `virbr0` if it exists. Tap attaches
  through `qemu-bridge-helper`, with vhost-net when `/dev/vhost-net` is accessible.
- **Bridged to the LAN:** attach to an existing user-configured bridge, listed from netlink.
- **Isolated** (host-only) networks.

**Critical pitfall: native Docker on the same host breaks VM networking.** Docker sets the
iptables/nft `FORWARD` policy to DROP, which silently kills forwarded traffic for bridged and
NAT VMs. Separate nft tables can't override another table's drop. Detect it and fix it the
supported way (rules in Docker's `DOCKER-USER` chain, or a documented nft integration
installed by `velox setup`), and put it in the test matrix together with firewalld and ufw.

VM names resolve as `<vm>.velox.local` through the **same** DNS responder as containers, fed
by DHCP leases or the guest agent's IP report.

### 5.6 Storage layout
- Paths follow XDG:
  - VM directories: `$XDG_DATA_HOME/velox/vms/<id>/` (manifest, disks, NVRAM, TPM state,
    seed ISO).
  - Images cache: `$XDG_CACHE_HOME/velox/images/`.
  - Runtime sockets: `$XDG_RUNTIME_DIR/velox/`.
- Per-VM storage location can be moved (a folder picker), with the same "fail loudly if a VM
  that has booted before has missing disks" rule as Velox workspaces. Never silently create a
  blank disk.
- Manifests are versioned JSON, written durably (temp + fsync + rename) under a lock file.
  Corrupt JSON fails loudly; it is never replaced with defaults.
- **Never mutate a disk a running VM has open.** Guard on what the runner actually attached,
  not on the manifest. QEMU's own image locking is a second line of defense, not the first.

---

## 6. Readiness: "Install Docker / virtualization to continue"

The Docker area and the VM area each sit behind their **own** readiness state machine. Both
are evaluated on start and **re-evaluated on events** (§3.8). They are two providers on one
mechanism (`velox-readiness`), so they behave identically: same evaluation loop, same UI
component, same CLI behaviour.

Each blocking state shows **one short line**, plus at most one link, or a short block of
copyable steps.

Rules that apply to both:
- The program **never** runs these commands itself, never adds the user to a group, and never
  edits `/etc`. It shows the steps; the user runs them.
- **The two areas are independent.** Docker missing never blocks VMs; virtualization missing
  never blocks Docker. A remote Docker host (§4) is independent of local Docker readiness.
- **The first blocking state wins.** Evaluate in table order and show only that one, so the
  user always sees exactly the next thing to do.
- **Group changes need a new login session.** Being listed in `/etc/group` without the group
  in the process's `getgroups()` is its own state ("Log out and back in to finish"), never a
  confusing permission error.
- **The CLI mirrors the GUI.** `velox docker …` and `velox vm …` exit non-zero with the same
  line, followed by the same steps. `velox doctor` prints both state machines.
- **Package names and commands live in ONE data file**, keyed by distro and architecture
  (amd64: `qemu-system-x86`, `ovmf`; arm64: `qemu-system-arm`, `qemu-efi-aarch64`). The UI,
  the CLI, `velox doctor` and the `.deb`'s `Suggests:` all read it. In Phase 0, verify the
  exact package set on a clean Debian stable before you write it down.

### 6.1 Docker

| # | State | Detection | UI text | Fix shown |
|---|---|---|---|---|
| 1 | Not installed | no `docker-ce` in dpkg status and no reachable socket | **Install Docker to continue** | Link: `https://docs.docker.com/engine/install/debian/` |
| 2 | Not from Docker's repo | `docker.io` / `podman-docker` installed instead of `docker-ce` | **Install Docker Engine from Docker's repository** | Same link |
| 3 | Not running | package present; socket missing or refusing | **Docker is not running** | `sudo systemctl enable --now docker` |
| 4 | No permission | `EACCES` on the socket | **No access to Docker** | `sudo usermod -aG docker $USER`, then log out and in. Link: `https://docs.docker.com/engine/install/linux-postinstall/`. Brief note: the `docker` group is root-equivalent; rootless Docker is the alternative. |
| 5 | Re-login needed | in `/etc/group`, not in the session | **Log out and back in to finish** | — |
| 6 | Too old | `/version` API below the minimum in `versions.env` | **Update Docker to continue** | Same install link |
| — | Ready | `/_ping` OK and the API version is OK | the dashboards | — |

### 6.2 Virtualization (KVM/QEMU)

| # | State | Detection | UI text | Fix shown |
|---|---|---|---|---|
| 1 | No hardware support | no `vmx`/`svm` in `/proc/cpuinfo` (amd64), or no KVM on arm64, and no `/dev/kvm` | **Enable virtualization in your firmware** | One line: turn on Intel VT-x / AMD-V (SVM) in BIOS/UEFI. If the host is itself a VM, enable nested virtualization on its hypervisor. |
| 2 | KVM module not loaded | CPU flags present, `/dev/kvm` missing | **KVM is not loaded** | `sudo modprobe kvm_intel` or `sudo modprobe kvm_amd`, picked from the CPU vendor |
| 3 | QEMU not installed | `qemu-system-<arch>` or `qemu-img` missing | **Install virtualization to continue** | The steps block below |
| 4 | No permission | `EACCES` opening `/dev/kvm` | **No access to KVM** | `sudo usermod -aG kvm $USER`, then log out and in |
| 5 | Re-login needed | in `/etc/group`, not in the session | **Log out and back in to finish** | — |
| 6 | Too old | QEMU below the minimum in `versions.env` | **Update QEMU to continue** | The steps block below |
| — | Ready | `/dev/kvm` opens, `KVM_GET_API_VERSION` is OK, QEMU version is OK | the VM area | — |

The **"Install virtualization to continue"** steps block, as shown on Debian amd64 (the
contents come from the data file):

```bash
sudo apt update
sudo apt install qemu-system-x86 qemu-utils ovmf swtpm swtpm-tools virtiofsd passt
sudo usermod -aG kvm $USER
```

Then: **"Log out and back in."** Below the steps, add a link to Debian's KVM page
(`https://wiki.debian.org/KVM`) for anything unusual.

**Optional components degrade a single feature; they never gate the whole area.** Once the
VM area is Ready, a missing optional piece disables only its own feature, with an inline hint
naming the package:

| Missing | Feature disabled | Inline hint |
|---|---|---|
| `ovmf` / `qemu-efi-aarch64` | UEFI, Secure Boot | **UEFI needs ovmf** |
| `swtpm`, `swtpm-tools` | TPM 2.0 (and so Windows 11) | **TPM needs swtpm** |
| `virtiofsd` | Shared folders | **Sharing needs virtiofsd** |
| `passt` | User-mode networking | **Networking needs passt** (bridged NICs still work) |
| `qemu-bridge-helper` allow-list | NAT and bridged NICs | **Run `velox setup` for bridged networking** |
| display module | Embedded graphical console | Serial console only, plus the package name (determined in Phase 0) |

The minimum required set behind state 3 is `qemu-system-<arch>` + `qemu-utils`. Everything
else is optional.

---

## 7. Mechanism choices to settle in Phase 0 (record each as an ADR)

1. The GUI toolkit (default: GTK4 + libadwaita) and the embedded terminal (`vte4`).
2. The I/O model. The lean default is blocking threads + one epoll loop for streams; adopt
   `tokio` only if the ADR shows it pays off. GTK owns the main loop, so define how
   background events reach it (`glib::MainContext` channels).
3. Direct QEMU + per-VM runner + systemd units (the default) against **libvirt**.
   - libvirt brings mature snapshots, networks and virt-manager interop.
   - It costs a resident C daemon, XML, and indirection.
   - The default is direct QEMU, but the agent must write the comparison honestly.
4. The VM graphical console: QEMU D-Bus display against SPICE.
5. The NAT-bridge provider (§5.5).
6. Snapshot model: internal against external (§5.3).
7. Distribution: a `.deb` from our own signed apt repo (updates via apt) against GitHub
   release tarballs with Velox's signed self-updater. The two must not both update the same
   install.

The `.deb` treats both parts symmetrically. Neither Docker nor the QEMU stack is a `Depends`
or a `Recommends`, because both are **detected, not required**, and the readiness screens
(§6) are how a user finds out what to install. The QEMU stack appears only under `Suggests:`,
generated from the same data file as §6. Docker is never declared at all, since it must come
from Docker's repository.

---

## 8. Repository layout (starting point; change it with an ADR)

```
Cargo.toml                 # workspace
versions.env               # the ONLY version source
crates/
  velox-docker/            # HTTP/1.1 codec, Docker API client, /events + /stats streams, socket resolution
  velox-readiness/         # ONE readiness mechanism, two providers: Docker + virtualization (§6); package data file
  velox-qmp/               # QMP client + event stream
  velox-vm/                # VM model, manifests, QEMU argv builder, runner binary, snapshots, clone, export/import
  velox-net/               # netlink, bridges, DNS responder (containers + VMs), resolved integration
  velox-ssh/               # remote-host tunnels
  velox-jobs/              # the one job/progress/cancel model
  velox-store/             # durable JSON manifests + file locks
  velox-cli/               # `velox`
  velox-gui/               # GTK4/libadwaita app
docs/adr/                  # one file per decision, numbered
docs/bench/                # benchmark harness + reports
packaging/                 # .deb, systemd unit templates, setup/undo assets
```

Every new dependency needs a one-line justification. Footprint is part of the North Star.

---

## 9. Phased plan with exit gates

At each gate, **stop, write a short report** (what was built, what was measured, open risks)
**and wait for the human's go.** Commit in small, reviewable steps. Never tag a release.

### Phase 0: recon, conventions, spikes
- Study §1. Write the repo's `CLAUDE.md` by adapting Velox's conventions to this brief. Write
  the ADRs in §7.
- Spikes:
  - (a) Talk to a native Docker socket from Rust and stream `/events`.
  - (b) Boot a Debian cloud image under hand-written `qemu-system-x86_64 -accel kvm` +
    cloud-init, then `snapshot-save`/`snapshot-load` over QMP.
  - (c) Embed a QEMU console in a GTK4 window.
- **Exit:** CLAUDE.md, ADRs, spike results, and the risk list.

### Phase 1: readiness (both parts) + Docker core (CLI first)
- `velox-readiness` with both providers and the package data file, plus `velox doctor`.
- **Every** state in §6.1 and §6.2 must be reproducible in tests, using throwaway Debian VMs:
  - Docker: no Docker, `docker.io` instead of `docker-ce`, Docker stopped, no group
    membership, re-login pending, rootless Docker.
  - Virtualization: no `/dev/kvm` (nested virtualization off), module unloaded, no QEMU, no
    `kvm` group, re-login pending, each optional package missing.
- `velox-docker` and `velox docker ps|images|logs|…` style read commands.
- **Exit:** every readiness state is shown correctly and turns Ready live, with no restart,
  after the user runs the shown steps. The two areas never block each other. The informer
  survives a `systemctl restart docker`.

### Phase 2: VM core (CLI first)
- The runner, systemd transient units, QMP, cloud-image catalog + cloud-init, ISO install,
  UEFI/SeaBIOS, lifecycle, serial console, guest agent, passt networking, stats.
- Each optional-component degradation from §6.2 is wired to its feature.
- **Exit:** create→boot→SSH time measured; the SIGKILL durability test passes; VMs survive the
  GUI/CLI exiting; autostart works.

### Phase 3: snapshots, clones, export/import
- Everything in §5.3–5.4, built on the shared job model.
- **Exit:** a round-trip test matrix, covering live snapshot→revert, linked clone→delete-base
  refusal, export bundle→import on a second machine, and OVA→VirtualBox import (manual check
  documented), plus timings.

### Phase 4: networking, sharing, devices
- NAT bridge (ADR), bridged and isolated networks, `qemu-bridge-helper` integration,
  `velox setup` / `--undo`, named access for containers + VMs, virtiofs shares, USB hot-plug,
  Secure Boot + swtpm (Windows 11 install test).
- **Exit:** a coexistence matrix (Docker installed / firewalld / ufw / NetworkManager /
  systemd-networkd) all green, and VM network throughput against virt-manager defaults.

### Phase 5: GUI
- Docker dashboards (Velox parity), VM list/detail/console/snapshot tree/jobs, settings,
  remote hosts.
- **Exit:** a screenshot tour, GUI idle RAM/CPU with the window open, closed and in the tray,
  and accessibility labels on every control.

### Phase 6: packaging, updates, CI, scorecard
- `.deb` + the chosen update channel, CI (GitHub Linux runners expose `/dev/kvm` behind a udev
  rule; verify it in Phase 0) running real VM boot/snapshot tests and Docker-readiness tests,
  and a benchmark harness against the §2 peers.
- **Exit:** install/upgrade/uninstall on clean Debian stable + Ubuntu LTS, and a scorecard with
  no unexplained regression. Hand it to the human for the release decision.

---

## 10. Working rules for you, the agent

- **Releasing is opt-in.** Never bump the version, write a `release:` commit or push a `v*`
  tag unless the human explicitly says "release" or "ship".
- **Leave the dev machine's system alone.** No `sudo`, no Docker install or reconfiguration,
  no `/etc` edits, no group changes, no firewall changes. Test those paths in throwaway VMs or
  CI.
- Measure every performance decision. Repeat runs, report the median and spread, and don't
  call a delta smaller than the spread a win or a regression.
- Before writing a "simpler version of X for this one case", extend X (§3.9). Before adding a
  timer, find the event (§3.8). Before adding a daemon, don't.
- Keep `CLAUDE.md` current. When a decision changes, update the binding text in the same
  commit.
- Report outcomes faithfully. A skipped test is reported as skipped.

---

## 11. Decisions reserved for the human (ask, don't assume)

1. **Product and binary name.** `velox` is used here as a placeholder.
2. **License.** Velox is LGPL-2.1; this is a new codebase, so confirm the license.
3. *(Decided: Docker and VMs are parallel paths, each gated independently, §6.)*
4. **Accepting non-official Docker.** The default is to require `docker-ce` from Docker's repo,
   and Debian's `docker.io` shows "Install Docker Engine from Docker's repository". Confirm
   this, or accept any working Engine API.
5. **libvirt against direct QEMU**, after the Phase 0 ADR.
6. **Distribution channel:** an apt repo or GitHub releases with the self-updater.
7. **PCI/VFIO passthrough and VM migration:** in or out of 1.0.
8. **Target matrix:** Debian stable only, or also Ubuntu LTS, plus whether arm64 is a 1.0
   requirement.

---

## 12. Definition of done for 1.0

- On a clean Debian stable with **neither Docker nor QEMU**, the app opens:
  - the Docker area says **"Install Docker to continue"** and links Docker's Debian install
    guide;
  - the VM area says **"Install virtualization to continue"** and shows the apt + `kvm` group
    steps.

  After the user follows either one (and logs back in where a group changed), that area
  unlocks **without restarting the app**, and the other area is unaffected throughout. The
  same holds for the reverse orders: Docker only, and QEMU only.
- The Docker dashboards reach Velox parity, remote hosts work over SSH, and `<name>.velox.local`
  resolves containers and VMs.
- VMs: create from cloud image or ISO (incl. Windows 11 with TPM + Secure Boot), embedded
  console, live snapshots with revert, full and linked clones, export/import (bundle, OVA,
  disk formats), shared folders, USB hot-plug, passt/NAT/bridged/isolated networking,
  autostart.
- The durability kill test, the network coexistence matrix and the CI VM tests are green.
- The scorecard meets §2, and every number in the README comes from the current report.
