# Bande apps

Apps for the Bande desktop. They install from the same apt archive as the desktop
(`rolling main`), but live here so that changing one app rebuilds only that app, not the desktop,
and each is released on its own. `bande-desktop` recommends the default ones (listed in the
desktop's `packaging/bande-desktop/default-apps`) unversioned, so any of them can be removed or
replaced.

```
apt install bande-<name>
```

| App | Package |
|---|---|
| Terminal | `bande-terminal` |
| Software | `bande-software` (its AppStream catalog is a module) |
| Package Installer | `bande-installer` (the desktop maps `.deb` files to it) |
| Activity Monitor | `bande-monitor` |
| Mail | `bande-mail` |
| Calendar | `bande-calendar` |
| Browser | `bande-browser` (the engine is `bande-linux/web`, pinned by `ci/bump-web.sh`) |
| Velox | `bande-velox` (containers of Docker Engine or Podman, and virtual machines on QEMU) |
| Calculator | `bande-calculator` (Basic, Scientific and Programmer, paper tape, RPN) |
| Preview | `bande-preview` (images and PDF documents, the default viewer of both; PDFs render in a child process) |
| Clock | `bande-clock` (alarms, timers, a stopwatch and a world clock; rings with its window closed) |
| TextEdit | `bande-textedit` (plain and rich text, Markdown, HTML, Word and OpenDocument; the desktop maps `text/plain` to it) |
| Disk Utility | `bande-disks` (disks, volumes and disk images through UDisks2; replaces gnome-disk-utility) |
| Archive Utility | `bande-archive` (the desktop maps zip, tar, gzip, bzip2, xz, zstd and 7-Zip files to it, and Files' Compress) |

Every app is a `bande-app` app: like on macOS, it keeps running after its last window closes and
reopens a window when activated, until Quit (Ctrl+Q). Without a window its menus stay full:
items that act on a window's content are disabled, the rest work and open the window they need,
and the Window menu names the main window (Message Viewer, Calendar, Activity Monitor, Calculator).

## Layout

| Path | What it is |
|---|---|
| `Cargo.toml`, `Cargo.lock` | one Cargo workspace: every app, the shared libs, the template, one lock, one desktop pin, shared lints and the release profile (the desktop's `dist`: fat LTO, one codegen unit, `panic = "abort"`) |
| `apps/<name>/` | one app on `bande-app`, packaged as `bande-<name>` |
| `apps/<name>/build-depends` | optional: Debian packages the build needs beyond what every app needs, one per line |
| `apps/<name>/smoke.sh` | optional: extra checks after `ci/smoke.sh`, once the package is installed; must exit 0 |
| `libs/<name>/` | code shared by apps, the crate `bande-<name>` (`bande-keyring`: passwords in the Secret Service; `bande-vm`: QEMU virtual machines in systemd user units) |
| `template/` | the starting point for a new app: a real, gated `bande-app` app, never published |
| `rust-toolchain.toml` | the Rust toolchain for every app, the same as the desktop's |
| `ci/` | the build, the same in CI and locally |
| `ci/gate.sh [app...]` | the local merge gate: CI's own `ci/build-app.sh` on each app in the `gate` profile, `ci/check.sh` on the libs and the template, one `target/` (default: every app, the libs and the template) |
| `ci/setup.sh install\|check <app>...` | the toolchain, `cargo-deb` and Debian packages a build needs: CI installs them, the gate fails if one is missing |
| `ci/build-app.sh <app> <out> [profile]` | one app's build, the same in CI and the gate: `ci/check-app.sh`, `ci/version.sh`, `ci/check.sh`, the binaries, the `.deb` and `ci/smoke.sh` on it |
| `ci/check.sh <package>` | fmt, clippy `--all-targets -D warnings` and the tests of one package, `--locked`, on a private session bus with software Vulkan |
| `ci/check-app.sh <dir>...` | static checks that an app is a `bande-app` app packaged like the others (ids, `.desktop`, metainfo, fonts); fails on any problem |
| `ci/smoke.sh [--install] <deb>` | checks a package unpacked: `desktop-file-validate`, `appstreamcli validate`, `--version`, apt resolves its dependencies; `--install` (CI) then installs it and checks its files and the app's own `smoke.sh` |
| `ci/dry-run.sh <app>` | dispatches the release build of `<app>` on `main` without publishing it and waits for it |
| `ci/new-app.sh <dir> <Id> "<Name>"` | a new app in `apps/<dir>` from the template |
| `ci/release-all.sh` | raises every app's patch version and prints the tags that release them, for a bump that changes `bande-app` behaviour |
| `ci/bump-desktop.sh [REV]` | pins every app to one `bande-linux/desktop` commit (default: main) and runs the gate |
| `ci/bump-web.sh [REV]` | pins Browser to one `bande-linux/web` commit (default: main) and runs its gate |
| `ci/with-desktop.sh <desktop> <cargo args>` | runs cargo on the workspace against a desktop checkout instead of the pin |

## Adding an app

```
ci/new-app.sh mail-merge MailMerge "Mail Merge"
```

makes `apps/mail-merge/` (package `bande-mail-merge`, app id `org.bande.MailMerge`) from the
template and prints the rest: the theme icon in the desktop's `data/icons/src/apps.py`, the
table above, and `default-apps` for a default app. Check that it compiles
(`cargo check --locked -p bande-mail-merge`), then merge to `main`; `ci/gate.sh` runs once on
`main` for the batch.

## How it's built and published

CI runs only for releases. To release an app:

1. Set its `version` in `apps/<name>/Cargo.toml`, run `ci/gate.sh <name>` and merge to `main`.
2. Dry-run the release: `ci/dry-run.sh <name>` dispatches `build.yml` on `main` with
   `tag=<name>-vX.Y.Z`, which builds and checks that commit exactly as the tag will (same version,
   same checks, both architectures), and waits for it. Dispatch runs are never published.
3. When it passes, tag the commit it printed and push the tag:

```
git tag mail-v0.1.0 <commit> && git push origin mail-v0.1.0
```

The tag builds that one app for arm64 and amd64 on GitHub's runners in `debian:testing`:
`ci/setup.sh install`, `ci/build-app.sh` (release profile) and `ci/smoke.sh --install`. A tag whose
commit isn't on `main`, or whose version doesn't match `Cargo.toml`, fails. A green run uploads
`bande-debs-arm64` and `bande-debs-amd64`; the archive's scheduled publish picks them up within 30
minutes, taking only the package named after the tagged app and never a name the desktop publishes.
The tag run rebuilds rather than promoting the dry run's packages: the archive takes a package only
from the green push run of the tag that points at the commit it built, and a promotion path could
only be tested by pushing a real tag.

A manual dispatch without a tag (`gh workflow run build.yml -f apps=<name>`) test-builds any app on
any branch with a `+git` version; it is never published either.

`ci/gate.sh` runs the same scripts as CI, so a release fails on nothing the gate skipped, except:

- The release profile (fat LTO, one codegen unit): the gate packages the `gate` profile, the same
  without its slow optimisations, so a gate stays within minutes; the dry run builds the release one.
- amd64: the mini is arm64; cross-checking needs every C library for amd64. The dry run covers it.
- Installing into a clean root system (`ci/smoke.sh --install`): apt resolving the package's
  dependencies from Debian alone, the installed files and the app's own `smoke.sh` need root in a
  fresh `debian:testing`; locally `apt-get -s` resolves them against this system only.
- The release tag itself (`<app>-vX.Y.Z` on a commit of `main`, matching `Cargo.toml`): checked by
  the dry run.

## Licence

MIT OR Apache-2.0, unless an app says otherwise.
