#!/bin/bash
# gate.sh [app...]: the local merge gate for the named apps (default: every app, libs/ and the
# template), with CI's own scripts: ci/setup.sh check fails if a tool or package CI installs is
# missing here; each app gets ci/build-app.sh in the `gate` profile (its checks, clippy and tests
# alone, its .deb and ci/smoke.sh on it); libs/ and the template get ci/check-app.sh and
# ci/check.sh. Only what needs a clean root system or the other architecture stays CI-only (README).
# One workspace target dir (./target). On the build machines it runs as a job of the build queue
# (~/bande-linux/bin/build-run), which picks the machine; in CI there is no queue.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
if [ -z "${BUILD_RUN_JOB:-}" ] && [ -x "$HOME/bande-linux/bin/build-run" ]; then
    exec "$HOME/bande-linux/bin/build-run" "$root" ci/gate.sh "$@"
fi
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
packages=()
for dir in "${others[@]}"; do
    packages+=("bande-$(basename "$dir")")
done
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
for app in "${apps[@]}"; do
    echo "== $app"; ci/build-app.sh "$app" "$out" gate
done
for package in "${packages[@]}"; do
    echo "== $package"; ci/check.sh "$package"
done
echo "== gate passed: ${apps[*]} ${packages[*]}"
