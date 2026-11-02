#!/bin/bash
# gate.sh [app...]: the local merge gate for the named apps (default: every app, libs/ and the
# template), with CI's own scripts: ci/setup.sh check fails if a tool or package CI installs is
# missing here; each app gets ci/build-app.sh in the `gate` profile (its checks, clippy and tests
# alone, its .deb and ci/smoke.sh on it); libs/ and the template get ci/check-app.sh and
# ci/check.sh. Only what needs a clean root system or the other architecture stays CI-only (README).
# One workspace target dir (./target). It takes ~/bande-linux/kbuild.lock (one heavy build at a
# time) and runs under ~/bande-linux/bin/gentle; while ~/bande-linux/kbuild.reserve exists it
# waits, so a queued long build isn't starved.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
if [ $# -eq 0 ]; then
    mapfile -t apps < <(find apps -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' | xargs -rn1 basename | sort)
    others=(template libs/*/)
else
    apps=("$@")
    others=()
fi
ci/setup.sh check "${apps[@]}"
[ ${#others[@]} -eq 0 ] || ci/check-app.sh template
free=$(df --output=avail -BG "$root" | tail -1 | tr -dc 0-9)
need=12; [ -d target ] && need=4
[ "$free" -ge $need ] || { echo "gate: only ${free}G free, refusing to build" >&2; exit 3; }
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target}
gentle=$HOME/bande-linux/bin/gentle
[ -x "$gentle" ] || gentle=env
lock=$HOME/bande-linux/kbuild.lock
while [ -e "$HOME/bande-linux/kbuild.reserve" ]; do
    echo "gate: waiting, the build lock is reserved by $(cat "$HOME/bande-linux/kbuild.reserve" 2>/dev/null)" >&2
    sleep 60
done
packages=()
for dir in "${others[@]}"; do
    packages+=("bande-$(basename "$dir")")
done
exec flock "$lock" bash -c '
set -euo pipefail
gentle=$1 root=$2 apps=$3; shift 3
cd "$root"
out=$(mktemp -d)
trap "rm -rf \"$out\"" EXIT
for app in $apps; do
    echo "== $app"; "$gentle" ci/build-app.sh "$app" "$out" gate
done
for package in "$@"; do
    echo "== $package"; "$gentle" ci/check.sh "$package"
done
echo "== gate passed: $apps $*"
' gate "$gentle" "$root" "${apps[*]}" "${packages[@]}"
