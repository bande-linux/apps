#!/bin/bash
# setup.sh install|check <app>...: the toolchain, cargo-deb and Debian packages the build of the
# named apps needs, from one list: install (as root, in CI's debian:testing) installs them; check
# (the local gate) fails, naming what to install, if any is missing.
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:?usage: setup.sh install|check <app>...}
shift
cargo_deb=3.8.0
channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
packages=(build-essential pkg-config jq libxkbcommon-dev libfontconfig1 libwayland-client0 libvulkan1
    mesa-vulkan-drivers dbus-daemon fonts-inter fonts-manrope fonts-jetbrains-mono desktop-file-utils
    appstream python3)
for app in "$@"; do
    [ -f "apps/$app/Cargo.toml" ] || { echo "setup: no app apps/$app" >&2; exit 1; }
    mapfile -t -O ${#packages[@]} packages < <(grep -v '^#' "apps/$app/build-depends" 2>/dev/null | xargs -rn1)
done

case $mode in
install)
    export DEBIAN_FRONTEND=noninteractive PATH="$HOME/.cargo/bin:$PATH"
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends rustup "${packages[@]}" > /dev/null
    rustup toolchain install "$channel" --profile minimal --component clippy,rustfmt > /dev/null
    rustup default "$channel" > /dev/null
    cargo deb --version 2>/dev/null | grep -qw "$cargo_deb" ||
        cargo install -q cargo-deb --version "$cargo_deb" --locked
    ;;
check)
    [ ! -f ~/.cargo/env ] || . ~/.cargo/env
    missing=()
    for package in "${packages[@]}"; do
        dpkg-query -W -f='${db:Status-Abbrev}' "$package" 2>/dev/null | grep -q '^ii' || missing+=("$package")
    done
    [ ${#missing[@]} -eq 0 ] || { echo "setup: missing Debian packages: sudo apt-get install ${missing[*]}" >&2; exit 1; }
    command -v rustup > /dev/null || { echo "setup: no rustup" >&2; exit 1; }
    installed=$(rustup component list --toolchain "$channel" --installed 2>/dev/null) ||
        { echo "setup: no Rust $channel: rustup toolchain install $channel --component clippy,rustfmt" >&2; exit 1; }
    for component in clippy rustfmt; do
        grep -q "^$component" <<< "$installed" ||
            { echo "setup: Rust $channel lacks $component: rustup component add --toolchain $channel $component" >&2; exit 1; }
    done
    cargo deb --version 2>/dev/null | grep -qw "$cargo_deb" ||
        { echo "setup: not cargo-deb $cargo_deb: cargo install cargo-deb --version $cargo_deb --locked" >&2; exit 1; }
    echo "setup: every tool and package is installed"
    ;;
*)
    echo "usage: setup.sh install|check <app>..." >&2
    exit 2
    ;;
esac
