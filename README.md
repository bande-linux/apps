# Bande apps

Apps for the Bande desktop. They install from the same apt archive as the desktop
(`rolling main`), but live here so that changing one app rebuilds only that app, not the desktop,
and each is released on its own. `bande-desktop` recommends the default ones (listed in the
desktop's `packaging/bande-desktop/default-apps`) unversioned, so any of them can be removed or
replaced.

```
apt install bande-<name>
```

| App | Package | Notes |
|---|---|---|
| Terminal | `bande-terminal` | moved from the desktop repo (history kept) |
| Software | `bande-software` | moved from the desktop repo, with its AppStream catalog as a module |
| Package Installer | `bande-installer` | moved from the desktop repo; the desktop keeps `.deb` files mapped to it |
| Mail | `bande-mail` | |
| Calendar | `bande-calendar` | |

## Layout

| Path | What it is |
|---|---|
| `apps/<name>/` | one app: its own crate and `Cargo.lock`, packaged as `bande-<name>` |
| `apps/<name>/build-depends` | optional: Debian packages the build needs, one per line |
| `apps/<name>/smoke.sh` | optional: runs after the package is installed, and must exit 0 |
| `template/` | a starting point; never built or published |
| `rust-toolchain.toml` | the Rust toolchain for every app, the same as the desktop's |
| `ci/` | the build, the same in CI and locally |
| `ci/gate.sh [app...]` | the local merge gate: fmt, clippy `-D warnings` and tests, `--locked`, one shared `target/` |
| `ci/bump-desktop.sh [REV]` | pins every app to one `bande-linux/desktop` commit (default: main) and runs the gate |
| `ci/with-desktop.sh <desktop> <cargo args>` | runs cargo in every app against a desktop checkout instead of the pin |

## Adding an app

1. Copy `template/` to `apps/<name>/`.
2. Set `name = "bande-<name>"` in its `Cargo.toml`, fill in `[package.metadata.deb]` (keep
   `depends = "$auto"`), and run `cargo generate-lockfile`.
3. Check that it compiles, then merge to `main`; `ci/gate.sh` runs once on `main` for the batch.

## How it's built and published

CI runs only for releases. To release an app, set its `version` in `apps/<name>/Cargo.toml`,
merge to `main`, and tag that commit `<name>-vX.Y.Z` with the same version:

```
git tag mail-v0.1.0 && git push origin mail-v0.1.0
```

The tag builds that one app for arm64 and amd64 on GitHub's runners in `debian:testing`:
`cargo fmt --check`, clippy with `-D warnings`, the tests, then the `.deb`, installed with apt into
the container and checked with its `smoke.sh`. A tag whose commit isn't on `main`, or whose
version doesn't match `Cargo.toml`, fails. A green run uploads `bande-debs-arm64` and
`bande-debs-amd64`; the archive's scheduled publish picks them up within 30 minutes, taking only
the package named after the tagged app and never a name the desktop publishes.

A manual dispatch (`gh workflow run build.yml -f apps=<name>`) test-builds any app on any branch
with a `+git` version; it is never published.

## Licence

MIT OR Apache-2.0, unless an app says otherwise.
