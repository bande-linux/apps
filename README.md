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
| Preview | `bande-preview` (images; the default image viewer) |
| Clock | `bande-clock` (alarms, timers, a stopwatch and a world clock; rings with its window closed) |

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
| `ci/gate.sh [app...]` | the local merge gate: `ci/check-app.sh`, fmt, clippy `-D warnings` and tests, `--locked`, one `target/` (default: every app, the libs and the template) |
| `ci/check-app.sh <dir>...` | static checks that an app is a `bande-app` app packaged like the others (ids, `.desktop`, metainfo, fonts); fails on any problem |
| `ci/smoke.sh <package>` | checks an installed package: `--version`, its files, `desktop-file-validate`, `appstreamcli validate` |
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

CI runs only for releases. To release an app, set its `version` in `apps/<name>/Cargo.toml`,
merge to `main`, and tag that commit `<name>-vX.Y.Z` with the same version:

```
git tag mail-v0.1.0 && git push origin mail-v0.1.0
```

The tag builds that one app for arm64 and amd64 on GitHub's runners in `debian:testing`:
`cargo fmt --check`, clippy with `-D warnings`, the tests, then the `.deb`, installed with apt into
the container and checked with `ci/smoke.sh` and the app's own `smoke.sh`, if any. A tag whose commit isn't on `main`, or whose
version doesn't match `Cargo.toml`, fails. A green run uploads `bande-debs-arm64` and
`bande-debs-amd64`; the archive's scheduled publish picks them up within 30 minutes, taking only
the package named after the tagged app and never a name the desktop publishes.

A manual dispatch (`gh workflow run build.yml -f apps=<name>`) test-builds any app on any branch
with a `+git` version; it is never published.

## Licence

MIT OR Apache-2.0, unless an app says otherwise.
