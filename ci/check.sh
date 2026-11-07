#!/bin/bash
# check.sh <package>: cargo fmt --check, then clippy --all-targets -D warnings and the tests of one
# workspace package, --locked, on a private session bus with no display and Mesa's software Vulkan.
# One package at a time, so features resolve as in the package's own release build.
set -euo pipefail
cd "$(dirname "$0")/.."
package=${1:?usage: check.sh <package>}
unset WAYLAND_DISPLAY DISPLAY DBUS_SESSION_BUS_ADDRESS
export CARGO_TERM_COLOR=never VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
cargo fmt --all --check
ci/check-l10n.py apps template
cargo clippy --locked --all-targets -q -p "$package" -- -D warnings
dbus-run-session --config-file=ci/test-bus.conf -- cargo test --locked -q -p "$package"
echo "check: $package passed"
