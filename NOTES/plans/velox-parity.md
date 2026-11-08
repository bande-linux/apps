# Velox: parity with macOS Velox

The reference is macOS Velox (github.com/mikaelhug/Velox; ours). Bande Velox keeps Bande's title bar,
toolkit and theme. It matches the layout, density, information, live graphs and behaviour. Velox
on macOS runs its own engine in a VM; Bande Velox drives the computer's Docker Engine or Podman
(decision 4 in `docker-vm.md`). That difference decides the engine, workspaces and settings rows
below. Screenshots of each view sit next to the macOS ones in each stage's report.

## Gaps, in the order a user notices them

| # | Area | macOS Velox | Bande Velox now | Plan |
|---|---|---|---|---|
| 1 | Sidebar | Overview; Resources (Containers, Images, Volumes, Networks); Hosts (This Mac with running count, remote hosts, +); Workspaces (+); System (Engine Logs); engine pill at the bottom: dot, state, Start/Stop | Overview; "CONTAINERS" header over the four resources; Machines; engine name and version at the bottom, no control | Section headers Resources / Machines / Hosts / System; the engine pill with Start/Stop (rows 4 and 16) |
| 2 | Overview: engine card | Status icon disc, "Engine Running · up 1h 36m", "8 vCPU · 8 GB RAM · 64 GB disk", app and engine versions | One text line: engine, API, rootless, socket path | The card: state, engine uptime (from the systemd unit's start), CPUs and memory from `/info`, the size of the data folder's file system, Velox and engine versions |
| 3 | Overview: stat cards | Four cards with a tinted icon, title, 28 pt value, caption; Containers green, Images blue, Volumes orange, Disk purple | Five plain cards (with Machines), uppercase titles, no icons or colours, the Disk caption wraps | Icons and colours, title case, one-line captions; Machines stays as a fifth card |
| 4 | Overview: Disk Breakdown | A stacked bar (images, containers, volumes, build cache) with a legend | None | Add it from `/system/df`; refresh on container and image changes only |
| 5 | Overview: live graphs | CPU and Memory as line and fill; Disk I/O and Network mirrored around a centre line (blue up, red down); about 90 s of history; streams only while a stats view is shown | Same four graphs, plain lines, 60 s | Mirrored I/O and network, fills, 90 s; keep sampling only while shown |
| 6 | Overview: Reclaim Space | Toolbar sparkle button, sheet with toggles and "≈ size" per row | A text button in the header, a sheet with checkboxes | A toolbar button, sizes per row, a result line |
| 7 | Containers | Status capsules (green, yellow, orange, red), uptime that ticks, Ports and Image columns hidden by default, Ctrl+L logs, Ctrl+Delete delete, Copy → URL per port | Plain status text, all columns shown, logs and delete without shortcuts | Badges, column defaults, shortcuts, Copy URL entries |
| 8 | Logs window | Header with icon, name, "image · id"; footer "Streaming" / "Stream ended", reattaches after a restart | Filter, auto-scroll, wrap, copy, clear, count | Header, footer state, reattach |
| 9 | Images | Pull bar on top with progress, Arch badge, drop a `.tar` to load it | Pull from a menu, no badge, no drop | Pull bar, badge, drop to load (`/images/load`) |
| 10 | Volumes | Compose grouping, "dangling" and "orphaned" badges, inspector | Grouping and "dangling"; no "orphaned"; no inspector | "orphaned" project badge, inspector |
| 11 | Networks | Disclosure list: driver-coloured icon, "driver · subnet", container count, members with IPs | Table with the same fields | Keep the table (Bande style); colour the driver icons |
| 12 | Hosts | Remote engines over SSH (`ssh -L` to the remote socket), add sheet, connect/disconnect, rename, remove, per-host counts; every pane follows the selected host | None | Build it the same way: OpenSSH, the user's keys and config, nothing stored but user, host, port and socket |
| 13 | Workspaces | Several engine data disks, one active; new, duplicate, rename, move, delete; switching restarts the engine | None | Needs a decision: Docker's data root belongs to the system. Rootless Podman can run one user service per workspace with its own `--root`, without root. Proposed: workspaces for rootless Podman only; with Docker the section explains why it's missing |
| 14 | Command palette (Ctrl+K) | Search containers, images, volumes; inline start/stop/logs/shell/ports | None | Build it, with its Settings switch, off by default |
| 15 | Menu-bar panel | Engine state, running containers with ports and stop buttons, CPU/RAM line, Dashboard/Settings/Quit | None | A tray item through the desktop's StatusNotifierItem host, with its Settings switch, off by default |
| 16 | Engine lifecycle | Start/Stop/Resume, Resource Saver | Shows setup steps when no engine | Start/Stop for the user's Podman socket unit; for the system Docker service through systemd's own polkit action (needs the owner's yes, like the USB helper) |
| 17 | Engine Logs | Guest console, filter, auto-scroll, count | None | The engine unit's journal (system `docker.service` or the user's `podman.service`), with the same controls; explains when the journal isn't readable |
| 18 | Settings | Launch at login, crash notifications, updates, Docker context, about/diagnostics, CPU/memory/disk/swap, Resource Saver, file sharing | Crash notifications | Launch at login (XDG autostart), Copy Diagnostics. Engine resources, Resource Saver, file sharing, updates and contexts belong to macOS Velox's own engine and apt; left out |
| 19 | Notifications | "Container crashed", 137 noted as possibly OOM | Done; 137 has no note | Add the OOM wording |
| 20 | Onboarding | Three-step sheet | The not-ready page with setup steps | Keep Bande's page |

Not carried over: `.velox.local` names and links, VS Code, Rosetta, nested virtualization (macOS VM
features).

## Stages

1. Sidebar sections and engine pill; Overview (rows 1–6). Screenshots next to the macOS ones;
   idle CPU and memory measured.
2. Containers, logs, images, volumes, networks (rows 7–11).
3. Hosts (row 12), then workspaces once decided (row 13).
4. Palette, tray, engine lifecycle and logs, Settings (rows 14–19).
