#!/bin/bash
# build-app.sh <app> <out dir> [profile]: the build of one app, the same in CI and in the local
# gate: ci/check-app.sh, the package version (ci/version.sh), ci/check.sh, then the binaries in
# the Cargo profile (default release; the gate's `gate` profile is release without its slow
# optimisations), bande-<app>_<version>_<arch>.deb with its name, architecture and version checked,
# ci/smoke.sh on it, and its language packs (bande-l10n-tool debs). Leaves the .debs in <out dir>.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: build-app.sh <app> <out dir> [profile]} out=$(realpath -m "${2:?usage: build-app.sh <app> <out dir> [profile]}")
profile=${3:-release}
dir=apps/$app name=bande-$app
arch=$(dpkg --print-architecture)
export CARGO_TERM_COLOR=never

cargo metadata --no-deps --format-version 1 | jq -e --arg name "$name" --arg dir "$PWD/$dir/Cargo.toml" \
    'any(.packages[]; .name == $name and .manifest_path == $dir)' > /dev/null ||
    { echo "build-app: apps/$app must be the package $name" >&2; exit 1; }
ci/check-app.sh "$dir"
version=$(ci/version.sh "$app")
echo "build-app: $name $version for $arch, $profile profile"
ci/check.sh "$name"
cargo build --profile "$profile" --locked -q -p "$name"
target=${CARGO_TARGET_DIR:-target}
rm -rf "$target/$profile/l10n"
ci/l10n-tool.sh pack "$target/$profile/l10n"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
deb=$(cargo deb --no-build --locked -q --profile "$profile" -p "$name" --deb-version "$version" -o "$work/")
[ "$(dpkg-deb -f "$deb" Package)" = "$name" ] && [ "$(dpkg-deb -f "$deb" Architecture)" = "$arch" ] &&
    [ "$(dpkg-deb -f "$deb" Version)" = "$version" ] ||
    { echo "build-app: $deb has the wrong name, architecture or version" >&2; exit 1; }
ci/smoke.sh "$deb"
mkdir -p "$out"
cp "$deb" "$out/"
ci/l10n-tool.sh debs "$target/$profile/l10n" "$out" "$version" "$name" ci/copyright
echo "build-app: $(basename "$deb") passed"
