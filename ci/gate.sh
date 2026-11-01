#!/bin/bash
# gate.sh [app...]: the local merge gate for the named apps (default: every app and the template):
# ci/check-app.sh, cargo fmt --check, clippy --all-targets -D warnings and the tests, all --locked,
# on a private session bus. One workspace target dir (./target), no incremental, no debug info. It takes
# ~/bande-linux/kbuild.lock (one heavy build at a time) and runs under ~/bande-linux/bin/gentle;
# while ~/bande-linux/kbuild.reserve exists it waits, so a queued long build isn't starved.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
if [ $# -eq 0 ]; then
    mapfile -t dirs < <(find apps -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' | sort)
    dirs+=(template)
else
    dirs=()
    for app in "$@"; do
        [ -f "apps/$app/Cargo.toml" ] || { echo "gate: no app apps/$app" >&2; exit 1; }
        dirs+=("apps/$app")
    done
fi
packages=()
for dir in "${dirs[@]}"; do
    packages+=(-p "bande-$(basename "$dir")")
done
ci/check-app.sh "${dirs[@]}"
free=$(df --output=avail -BG "$root" | tail -1 | tr -dc 0-9)
need=12; [ -d target ] && need=4
[ "$free" -ge $need ] || { echo "gate: only ${free}G free, refusing to build" >&2; exit 3; }
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_TERM_COLOR=never
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target}
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
unset WAYLAND_DISPLAY DISPLAY DBUS_SESSION_BUS_ADDRESS
gentle=$HOME/bande-linux/bin/gentle
[ -x "$gentle" ] || gentle=env
lock=$HOME/bande-linux/kbuild.lock
while [ -e "$HOME/bande-linux/kbuild.reserve" ]; do
    echo "gate: waiting, the build lock is reserved by $(cat "$HOME/bande-linux/kbuild.reserve" 2>/dev/null)" >&2
    sleep 60
done
exec flock "$lock" bash -c '
set -euo pipefail
gentle=$1 root=$2; shift 2
cd "$root"
echo "== fmt"; cargo fmt --all --check
echo "== clippy"; "$gentle" cargo clippy --locked --all-targets -q "$@" -- -D warnings
echo "== test"; "$gentle" dbus-run-session --config-file="$root/ci/test-bus.conf" -- cargo test --locked -q "$@"
echo "== gate passed: $*"
' gate "$gentle" "$root" "${packages[@]}"
