# Software and Package Installer depth: what a user needs to rely on them

Benchmarked against the macOS App Store, GNOME Software and the macOS Installer. "Bande" is what
the apps do today; the ranked lists at the end are what is left. Delete this plan when they are
done.

## Software

| Area | User need (App Store, GNOME Software) | Bande Software |
|---|---|---|
| Start | Window at once, catalogue fills in | Have: catalogue, installed apps and icons load on worker threads; a spinner until the catalogue arrives |
| Places | Discover, Installed, Updates, categories | Have: segmented Discover/Installed/Updates (with the update count); Discover shows the nine categories and New & Updated (latest AppStream releases); category pages |
| Rows | Icon, name, summary, Get/Open | Have: two-column rows with the AppStream icon (JPEG XL, PNG or SVG, or the desktop entry's icon), Install/Open or the job's status |
| Search | Instant, ranked | Have: name, keywords, summary (bande-search), Show More pages of 40 |
| App page | Icon, name, developer, summary, screenshots, description | Have: Debian's 128 px icon, screenshots fetched once each, the current page first, and cached in `~/.cache/bande-software` under a hash of the address (unused for 30 days or past 200 MB, oldest first, they are pruned at start), with Previous/Next and captions |
| App page | What's New | Have: latest release, its date and notes |
| App page | Version, size, licence, source, developer, website | Have: available and installed version, download with the packages it pulls in (simulated install) or size on disk, category, licence, Debian suite and component, developer, website link |
| App page | Ratings and reviews | Missing; no review service (ODRS is the candidate) |
| App page | Age rating, permissions | Have: a Flatpak's sandbox permissions as sentences; missing: the age rating (OARS content rating is in AppStream) |
| Install | One click, password, progress, cancel, Open | Have: "You'll be asked for an administrator's password", "Waiting for your password…" while polkit asks, a cancelled prompt goes back quietly, progress with status and percent, Cancel when PackageKit allows it, Open when done |
| Install | Done while the window is closed | Have: a notification when it finishes or fails |
| Remove | Confirm, list what else goes | Have |
| Installed | Every installed app | Have: catalogue apps, apps that ship metainfo (the Bande apps) and apps with only a desktop entry, each with its package for removal |
| Updates | List, Update All, restarts | Hands off to Settings › Software Update (live, boot, offline and Flatpak classes, restarts, automatic updates at idle). Software lists the Debian and Flatpak updates, the apps among them and opens the pane |
| Errors | A sentence a person can act on | Have: no network, locked, no disk space, not in the package lists, unmet dependencies, system package, unsigned, no PackageKit; the PackageKit text under Details |
| Offline | Browse works; installs explain | Have: NetworkManager state shown as a line; screenshots reload when the network returns |
| Freshness | Catalogue follows apt | Have: reloaded on reopen when dpkg or the catalogue changed |
| Keyboard | Find, Back, places | Have: Ctrl+F, Alt+Left, Ctrl+1/2/3 |
| Sources | Where an app comes from, trust | Have: Debian suite and component. With Flatpak in use, one row per app (Debian first, Flatpak-only apps never in New & Updated), every Flatpak marked "Flatpak · remote" on rows, pages, Installed and Updates (Debian rows marked too there), and a source picker on the page when Debian and one or more remotes offer the app: Debian by default, each source's version and what it means (system package or sandboxed Flatpak); no per-source trust or "add a source" UI |
| Flatpak/Flathub | Apps Debian lacks | Dormant by default: Bande does not install `flatpak`. When the user has installed it with apt, Software (checked at launch) offers Settings… › Get apps from Flathub (off by default), which adds the remote with `--user` and reads its AppStream; per-user install with progress and Cancel, an X11-only app refused, remove with unused runtimes, the runtime download on the page; installs need no password |
| Settings | Automatic updates, notifications | Belong to Settings › Software Update, which has "Automatically keep this computer up to date" |

## Package Installer

| Step | User need (macOS Installer) | Bande Package Installer |
|---|---|---|
| Open a .deb | Name, icon, version, summary, size | Have: the app's name and icon from the package's desktop entry, package name, version, installed size, summary, Details |
| Dependencies | What else it installs or removes, how much | Have: "Also installs N packages · X to download" with the list, and what it removes (the button becomes Install and Remove) |
| Trust | Warn for packages outside the sources | Have: warns that the package is not from the software sources and its scripts run as root; when the sources have the same version, offers Install from Software Sources (signed) beside Install This File |
| Version | Upgrade, downgrade, already installed | Have: upgrade and downgrade notices, a downgrade caution, "Version X is installed" with Remove… |
| Install | Password, progress, cancel | Have: password hint, "Waiting for your password…", progress, Cancel; a refused or dismissed password says nothing was changed and what to do |
| Done | Success with Open, failure with a reason | Have: Open per app the package installs, restart notice; failures for locked, system package, no PackageKit, no network, disk space, dependencies, with the details and Try Again |
| Architecture | Refuse a foreign package clearly | Have |

## Remaining, ranked

Software:
1. A system-wide Flatpak installation as an option, and Flatpak apps a system remote offers.
2. Curated Discover (editor's picks per category, like GNOME Software's featured list) instead of
   only New & Updated.
3. OARS age rating and the licence as free/proprietary with a link.
4. Reviews from ODRS (opt-in, read-only first).
5. Move `Picture`/the installer's `Icon` into a toolkit image widget, and share the NetworkManager
   online follower with Calendar through a desktop library.

Package Installer:
1. Name the source a package adds (scan `etc/apt/sources.list.d` in the package) before
   installing.
2. Reinstall when the same version is installed.
