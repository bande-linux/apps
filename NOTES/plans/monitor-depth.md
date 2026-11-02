# Activity Monitor depth: what a user needs to rely on it

Benchmarked against macOS Activity Monitor. "Have" is what Activity Monitor does today; the
ranked list at the end is what is left. Delete this plan when the list is done.

## Tabs and columns

| macOS | User need | Bande |
|---|---|---|
| CPU: % CPU, CPU Time, Threads, Idle Wake Ups, PID, User | Find what burns the CPU and wakes the machine | Have, all default; % GPU and GPU Time choosable |
| Memory: Memory, Compressed, Threads, Ports, PID, User | Find what holds memory | Have Memory (footprint from `bande_status::memory::footprint`: PSS anon + swap PSS), Compressed (swap PSS), Resident, Threads, PID, User. No Ports column |
| Energy: Energy Impact, 12 hr Power, App Nap, Preventing Sleep | Find battery drains | Have Energy Impact (CPU + GPU + wake-ups, 30 s average), wake-ups, Preventing Sleep (logind inhibitors). No 12 hr average |
| Disk: Bytes Written/Read, Kind, PID | Find what hits the disk | Have rates and totals per process |
| Network: Sent/Rcvd Bytes and Packets per process | Find what uses the network | Have bytes per process for TCP (sock_diag `tcp_info` per socket, matched to `/proc/<pid>/fd`, only while the tab shows). A socket held by several processes (inherited across fork, passed over a Unix socket) counts once, for its earliest-started holder, the one that most likely made it, as nettop charges a socket to its owning process rather than to every holder. UDP/QUIC has no kernel byte counters and other users' sockets cannot be matched, so those show –. No packet counts |
| Column chooser (right-click a header) | Pick columns | Have, remembered per tab |
| GPU (window) | GPU use | Have a GPU tab when a DRM device reports per-client use |

## Footer panels

| macOS | Bande |
|---|---|
| CPU: System (red), User (blue), Idle, CPU Load graph, Threads, Processes | Have: same layout, coloured values, stacked graph (system red under user blue), per-core strips (drag the footer edge, click the graph, View → Show CPU Cores) |
| Memory: Memory Pressure graph, Physical, Used, Cached Files, Swap Used; App, Wired, Compressed | Have: same columns; the pressure graph is coloured green/yellow/red by PSI level, its height is memory used |
| Disk: reads/writes in and out per second, data read/written | Have; data read/s blue, written/s red in values and graph |
| Network: packets/data in/out | Data in/out only (no packet counts) |
| Energy: impact graph, battery charge 12 hr | Have both (battery graph when there is a battery) |

## View menu

| macOS | Bande |
|---|---|
| All / My / System / Selected / Applications in last 12 h / Active / Inactive / Windowed processes | Have All, All Hierarchically, My, System, Active, Apps (windowed) and Selected Processes. No Inactive or last-12-hours apps |
| All Processes, Hierarchically | Have (process tree by parent, kernel threads folded) |
| Update Frequency: 1 s / 2 s / 5 s | Have, remembered; the graph span follows (1, 2 or 5 min) |
| Columns, Dock Icon (CPU usage/history, memory, network, disk) | Columns have; dock icon is missing |
| Send Signal to Process | Have: View menu and the process context menu (HUP, INT, STOP, CONT, USR1, USR2), each but Continue after a confirmation, never pausing itself; TERM and KILL are Quit and Force Quit |
| Show/hide CPU, GPU history windows | Per-core strips in the footer instead |

## Process actions

| macOS | Bande |
|---|---|
| Quit / Force Quit with confirmation | Have; apps are asked over the session bus first, PID reuse is guarded by start time |
| Inspect: parent, user, CPU, threads, memory, open files and ports, statistics | Have Info (parent/children, command, unit, resources, statistics: page faults, context switches, read/write calls), Threads (with the kernel function each waits in), Open Files (with sockets and listening ports), Memory Map |
| Sample Process / spindump | Missing (Threads shows where each thread waits) |
| Search by name or PID, sort any column | Have |
| Sampling only while visible | Have: no process sampling while every window is hidden; the inspector samples its own process |

## Visual review (headless PNGs, light and dark)

Fixed: the window title was cut to "Activit…"/"Ac…" (toolbar now measures the title and fits tabs
and search, default size 960×640); PID and other columns cut their values or titles (widths now
fit the measured header); the memory footer mixed a usage bar into the readouts (now macOS's two
columns around the graph); graphs were a single accent line on a muddy well (now coloured series
with soft fills, framed well, caption above with span); footer height jumped between tabs (now
fixed four rows); lists use compact 24 pt rows (`Table::compact`), so about 20 processes show
instead of 15.

Cost: with Idle Wake Ups on the CPU tab (macOS's default) the release build uses about 1.6 % of
one core while visible at the 2 s interval, against 0.9 % without it (it reads every thread's
`status`); hidden, it samples nothing.

## Ranked remaining

1. Dock icon modes (CPU usage, CPU history, memory pressure): needs a desktop API for live dock
   icon content.
2. Sample Process in the inspector: user-space stacks need ptrace or perf (not permitted for other
   users' processes); a sampled call tree for the user's own processes would match macOS.
3. Ports column (socket count per process), packets in/out, and UDP traffic per process (needs
   eBPF or nftables accounting).
4. 12 hr average energy impact (needs a background sampler; only if it can stay asleep).
