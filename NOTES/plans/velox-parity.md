# Velox: parity with macOS Velox

The reference is macOS Velox (github.com/mikaelhug/Velox; ours). Bande Velox keeps Bande's title bar,
toolkit and theme. It matches the layout, density, information, live graphs and behaviour. Velox
on macOS runs its own engine in a VM; Bande Velox drives the computer's Docker Engine or Podman
(decision 4 in `docker-vm.md`). That difference decides the engine, workspaces and settings rows
below. Screenshots of each view sit next to the macOS ones in each stage's report.

## Gaps, in the order a user notices them

| # | Area | macOS Velox | Bande Velox now | Plan |
|---|---|---|---|---|
| 1 | Sidebar | — | Section headings (the toolkit's sidebar headings), + beside Hosts and Workspaces; the engine pill's Start/Stop waits on row 16 | — |
| 2 | Overview: engine card | — | Done | — |
| 3 | Overview: stat cards | — | Done | — |
| 4 | Overview: Disk Breakdown | — | Done | — |
| 5 | Overview: live graphs | — | Done; one redraw per stats round, nothing while hidden | — |
| 6 | Overview: Reclaim Space | — | Done | — |
| 7 | Containers | — | Done: status capsules (`Cell::badge`) | — |
| 8 | Logs window | — | Footer shows Streaming / Stream ended; reattach was already there | — |
| 9 | Images | — | Pull bar and arch tint were already there; dropping image archives (`.tar`, `.tar.gz`, `.tgz`, `.tar.zst`) loads them | — |
| 10 | Volumes | — | Done: "dangling" and "orphaned" badges, inspector (Ctrl+I) | — |
| 11 | Networks | — | Icons coloured by driver | — |
| 12 | Hosts | — | Done: remote engines over OpenSSH (`ssh -L` to the remote socket, the user's keys and config, nothing stored but user, host, port and socket); add, remove, copy the SSH command; a connecting/unreachable page with Try Again | Rename done; an edit sheet for the address |
| 13 | Workspaces | — | Done: the user's Docker contexts; switching one runs the equivalent of `docker context use` and every pane follows; New Workspace makes a context for this computer or a host | Rename and delete done |
| 14 | Command palette (Ctrl+K) | Search containers, images, volumes; inline start/stop/logs/shell/ports | None | Build it, with its Settings switch, off by default |
| 15 | Menu-bar panel | Engine state, running containers with ports and stop buttons, CPU/RAM line, Dashboard/Settings/Quit | None | A tray item through the desktop's StatusNotifierItem host, with its Settings switch, off by default |
| 16 | Engine lifecycle | Start/Stop/Resume, Resource Saver | Shows setup steps when no engine | Owner (2026-11-08): use the user's own rights first: no prompt for rootless Podman or for a member of the docker group; start and stop the unit when systemd allows it. Only when that is denied, ask through systemd's polkit prompt |
| 17 | Engine Logs | — | Done: System → Engine Logs follows the engine unit's journal (`journalctl [--user] -u docker/podman.service`) in the logs window; hidden for remote engines | — |
| 18 | Settings | Launch at login, crash notifications, updates, Docker context, about/diagnostics, CPU/memory/disk/swap, Resource Saver, file sharing | Crash notifications | Launch at login (XDG autostart), Copy Diagnostics. Engine resources, Resource Saver, file sharing, updates and contexts belong to macOS Velox's own engine and apt; left out |
| 19 | Notifications | — | Done, 137 named as possibly out of memory | — |
| 20 | Onboarding | Three-step sheet | The not-ready page with setup steps | Keep Bande's page |
| 21 | Packaging | — | No list of the programs and files the apps run or read | Port the desktop's `ci/runtime-deps` and its check to the apps repo, as its own task |

Not carried over: `.velox.local` names and links, VS Code, Rosetta, nested virtualization (macOS VM
features).

## Stages

1. Sidebar sections and engine pill; Overview (rows 1–6). Screenshots next to the macOS ones;
   idle CPU and memory measured.
2. Containers, logs, images, volumes, networks (rows 7–11).
3. Hosts (row 12) and workspaces as Docker contexts (row 13).
4. Palette, tray, engine lifecycle and logs, Settings (rows 14–19).
5. The runtime-deps check (row 21).
