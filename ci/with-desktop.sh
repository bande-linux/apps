#!/bin/bash
# with-desktop.sh <desktop checkout> <cargo args...>: runs `cargo <args>` on the workspace with each
# bande-linux/desktop crate it uses patched to that checkout, to try a desktop branch on the apps
# before it merges, e.g. `ci/with-desktop.sh ~/bande-linux/wt/desktop-x check --workspace
# --all-targets`. Cargo.lock is put back afterwards. Same lock, gentle and target dir as ci/gate.sh.
# Cargo keeps a locked version over a [patch] of another version and only warns, so the desktop
# crates leave the lock for the run, and it fails if any of them still comes from the git pin.
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
saved=$(mktemp)
cp "$root/Cargo.lock" "$saved"
trap 'cp "$saved" "$root/Cargo.lock"; rm -f "$saved"' EXIT
patches=()
for name in $(awk -v source="git+$source" '/^name = / { name = $3 } /^source = / && index($3, "\"" source) == 1 { gsub(/"/, "", name); print name }' "$root/Cargo.lock"); do
    path=$(awk -v name="$name" '$1 == name { print $2 }' <<< "$crates")
    [ -n "$path" ] || { echo "with-desktop: $desktop has no crate $name" >&2; exit 1; }
    patches+=(--config "patch.\"$source\".$name.path=\"$path\"")
done
cd "$root"
awk -v RS= -v ORS='\n\n' -v pinned="source = \"git+$source" 'index($0, pinned) == 0' "$saved" > Cargo.lock
resolved=$(mktemp)
trap 'cp "$saved" "$root/Cargo.lock"; rm -f "$saved" "$resolved"' EXIT
if ! report=$(cargo "${patches[@]}" metadata --format-version 1 2>&1 > "$resolved"); then
    echo "$report" >&2
    echo "with-desktop: cargo cannot resolve the apps against $desktop" >&2
    exit 1
fi
if grep -F 'was not used in the crate graph' <<< "$report" >&2; then
    echo "with-desktop: cargo did not use $desktop" >&2
    exit 1
fi
still_pinned=$(jq -r --arg source "git+$source" '.packages[] | select(.source // "" | startswith($source)) | .name' "$resolved")
if [ -n "$still_pinned" ]; then
    echo "with-desktop: still from the git pin, not $desktop:" $still_pinned >&2
    exit 1
fi
echo "== cargo $*"
flock "$HOME/bande-linux/kbuild.lock" "$gentle" cargo "${patches[@]}" "$@"
