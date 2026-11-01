#!/bin/bash
# build-app.sh <app> <out dir>: run as root in debian:testing with this repository checked out.
# Checks apps/<app> with ci/check-app.sh, builds it into bande-<app>_<version>_<arch>.deb, checks
# the package, installs it with apt into this container so its dependencies resolve from Debian,
# and runs ci/smoke.sh, then the app's own smoke.sh if it has one.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: build-app.sh <app> <out dir>} out=$(realpath -m "$2")
dir=apps/$app
arch=$(dpkg --print-architecture)
export DEBIAN_FRONTEND=noninteractive CARGO_TERM_COLOR=never

apt-get update -qq
common=(build-essential pkg-config rustup jq libxkbcommon-dev libfontconfig1 libwayland-client0
    dbus-daemon fonts-inter fonts-manrope fonts-jetbrains-mono desktop-file-utils appstream)
mapfile -t extra < <(grep -v '^#' "$dir/build-depends" 2>/dev/null | xargs -rn1)
apt-get install -y -qq --no-install-recommends "${common[@]}" "${extra[@]}" > /dev/null
export PATH="$HOME/.cargo/bin:$PATH"
channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
rustup toolchain install "$channel" --profile minimal --component clippy,rustfmt > /dev/null
rustup default "$channel" > /dev/null
cargo deb --version 2>/dev/null | grep -qw "$CARGO_DEB_VERSION" ||
    cargo install -q cargo-deb --version "$CARGO_DEB_VERSION" --locked

name=bande-$app
cargo metadata --no-deps --format-version 1 | jq -e --arg name "$name" --arg dir "$PWD/$dir/Cargo.toml" \
    'any(.packages[]; .name == $name and .manifest_path == $dir)' > /dev/null ||
    { echo "build-app: apps/$app must be the package $name" >&2; exit 1; }
ci/check-app.sh "$dir"
version=$(ci/version.sh "$app")
echo "build-app: $name $version for $arch"

cargo fmt --all --check
cargo clippy --locked --all-targets -q -p "$name" -- -D warnings
dbus-run-session --config-file=ci/test-bus.conf -- cargo test --locked -q -p "$name"
cargo build --release --locked -q -p "$name"
deb=$(realpath "$(cargo deb --no-build --locked -p "$name" --deb-version "$version")")
[ "$(dpkg-deb -f "$deb" Package)" = "$name" ] && [ "$(dpkg-deb -f "$deb" Architecture)" = "$arch" ] &&
    [ "$(dpkg-deb -f "$deb" Version)" = "$version" ] ||
    { echo "build-app: $deb has the wrong name, architecture or version" >&2; exit 1; }

apt-get install -y -qq "$deb" > /dev/null
ci/smoke.sh "$name"
[ ! -x "$dir/smoke.sh" ] || "$dir/smoke.sh"
mkdir -p "$out"
cp "$deb" "$out/"
echo "build-app: $(basename "$deb") passed"
