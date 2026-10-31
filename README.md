# Bande apps

Optional apps for the Bande desktop. They install from the same apt archive as the desktop
(`rolling main`), but live here so that changing one app rebuilds only that app, not the desktop.

```
apt install bande-<name>
```

## Layout

| Path | What it is |
|---|---|
| `apps/<name>/` | one app: its own crate and `Cargo.lock`, packaged as `bande-<name>` |
| `apps/<name>/build-depends` | optional: Debian packages the build needs, one per line |
| `apps/<name>/smoke.sh` | optional: runs after the package is installed, and must exit 0 |
| `template/` | a starting point; never built or published |
| `rust-toolchain.toml` | the Rust toolchain for every app, the same as the desktop's |
| `ci/` | the build, the same in CI and locally |

## Adding an app

1. Copy `template/` to `apps/<name>/`.
2. Set `name = "bande-<name>"` in its `Cargo.toml`, fill in `[package.metadata.deb]` (keep
   `depends = "$auto"`), and run `cargo generate-lockfile`.
3. Push to `main`.

## How it's built and published

On every push to `main`, `.github/workflows/build.yml` builds only the apps whose directory
changed, or every app when `ci/`, `rust-toolchain.toml` or the workflow changed. Each app is
built for arm64 and amd64 on GitHub's runners in `debian:testing`: `cargo fmt --check`, clippy
with `-D warnings`, the tests, then the `.deb`. That `.deb` is installed with apt into the
container and its `smoke.sh` is run. A dispatch builds the apps you name, or `all`.

An app's version is `<Cargo version>+git<date>.<count>.<sha>`, counted over the commits that
touch the app or the shared build, so any change that rebuilds an app makes it newer.

A green run uploads `bande-debs-arm64` and `bande-debs-amd64`. The archive's scheduled publish
picks them up within 30 minutes. It only takes packages named after an app in this
repository, and never a name the desktop publishes.

## Licence

MIT OR Apache-2.0, unless an app says otherwise.
