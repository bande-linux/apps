#!/bin/bash
# build-app.sh <app> <out dir>: run as root in debian:testing with this repository checked out.
# Builds apps/<app> into bande-<app>_<version>_<arch>.deb, checks the package, installs it with
# apt into this container so its dependencies resolve from Debian, and runs the app's smoke.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: build-app.sh <app> <out dir>} out=$(realpath -m "$2")
dir=apps/$app
arch=$(dpkg --print-architecture)
export DEBIAN_FRONTEND=noninteractive CARGO_TERM_COLOR=never

apt-get update -qq
mapfile -t extra < <(grep -v '^#' "$dir/build-depends" 2>/dev/null | xargs -rn1)
apt-get install -y -qq --no-install-recommends build-essential pkg-config rustup jq "${extra[@]}" > /dev/null
export PATH="$HOME/.cargo/bin:$PATH"
channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
rustup toolchain install "$channel" --profile minimal --component clippy,rustfmt > /dev/null
rustup default "$channel" > /dev/null
cargo deb --version 2>/dev/null | grep -qw "$CARGO_DEB_VERSION" ||
    cargo install -q cargo-deb --version "$CARGO_DEB_VERSION" --locked

name=$(cargo metadata --no-deps --format-version 1 --manifest-path "$dir/Cargo.toml" | jq -r '.packages[0].name')
[ "$name" = "bande-$app" ] || { echo "build-app: apps/$app must be the package bande-$app, not $name" >&2; exit 1; }
version=$(ci/version.sh "$app")
echo "build-app: $name $version for $arch"

(cd "$dir" && cargo fmt --check && cargo clippy --locked --all-targets -q -- -D warnings &&
    cargo test --locked -q && cargo build --release --locked -q)
deb=$(cd "$dir" && realpath "$(cargo deb --no-build --locked --deb-version "$version")")
[ "$(dpkg-deb -f "$deb" Package)" = "$name" ] && [ "$(dpkg-deb -f "$deb" Architecture)" = "$arch" ] &&
    [ "$(dpkg-deb -f "$deb" Version)" = "$version" ] ||
    { echo "build-app: $deb has the wrong name, architecture or version" >&2; exit 1; }

apt-get install -y -qq "$deb" > /dev/null
[ ! -x "$dir/smoke.sh" ] || "$dir/smoke.sh"
mkdir -p "$out"
cp "$deb" "$out/"
echo "build-app: $(basename "$deb") passed"
