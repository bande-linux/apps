#!/bin/bash
# with-desktop.sh <desktop checkout> <cargo args...>: runs `cargo <args>` in every app with each
# bande-linux/desktop crate it uses patched to that checkout, to try a desktop branch on the apps
# before it merges, e.g. `ci/with-desktop.sh ~/bande-linux/wt/desktop-x check --all-targets`.
# Each Cargo.lock is put back afterwards. Same lock, gentle and target dir as ci/gate.sh.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
desktop=$(realpath "${1:?usage: with-desktop.sh <desktop checkout> <cargo args...>}")
shift
[ $# -gt 0 ] || { echo "with-desktop: name the cargo command" >&2; exit 2; }
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
source=https://github.com/bande-linux/desktop
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_TERM_COLOR=never
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target}
gentle=$HOME/bande-linux/bin/gentle
[ -x "$gentle" ] || gentle=env
crates=$(cargo metadata -q --no-deps --format-version 1 --manifest-path "$desktop/Cargo.toml" |
    jq -r '.packages[] | "\(.name) \(.manifest_path | rtrimstr("/Cargo.toml"))"')
scratch=$(mktemp -d)
trap 'for lock in "$scratch"/*.lock; do [ -e "$lock" ] && cp "$lock" "$root/apps/$(basename "$lock" .lock)/Cargo.lock"; done; rm -rf "$scratch"' EXIT
for manifest in "$root"/apps/*/Cargo.toml; do
    dir=$(dirname "$manifest") app=$(basename "$(dirname "$manifest")")
    grep -q "$source" "$manifest" || continue
    cp "$dir/Cargo.lock" "$scratch/$app.lock"
    patches=()
    for name in $(awk -v source="git+$source" '/^name = / { name = $3 } /^source = / && index($3, "\"" source) == 1 { gsub(/"/, "", name); print name }' "$dir/Cargo.lock"); do
        path=$(awk -v name="$name" '$1 == name { print $2 }' <<< "$crates")
        [ -n "$path" ] || { echo "with-desktop: $desktop has no crate $name" >&2; exit 1; }
        patches+=(--config "patch.\"$source\".$name.path=\"$path\"")
    done
    echo "== $app: cargo $*"
    (cd "$dir" && flock "$HOME/bande-linux/kbuild.lock" "$gentle" cargo "${patches[@]}" "$@")
done
