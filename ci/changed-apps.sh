#!/bin/bash
# changed-apps.sh <before> <after> [names]: the apps to build, as a JSON array of directory names
# under apps/. With names ("all" or a space-separated list) it builds those. Otherwise it builds
# the apps whose directory changed between the two commits, or every app when the shared build
# (ci/, rust-toolchain.toml, the workflow) changed or <before> is unknown.
set -euo pipefail
cd "$(dirname "$0")/.."
before=$1 after=$2 names=${3:-}

all() { find apps -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' | xargs -r -n1 basename | sort; }

if [ -n "$names" ]; then
    if [ "$names" = all ]; then
        all
    else
        for name in $names; do
            [ -f "apps/$name/Cargo.toml" ] || { echo "changed-apps: no app apps/$name" >&2; exit 1; }
            echo "$name"
        done
    fi
elif [ -z "$before" ] || [ "$before" = 0000000000000000000000000000000000000000 ] ||
    ! git cat-file -e "$before^{commit}" 2>/dev/null ||
    git diff --name-only "$before" "$after" | grep -qE '^(ci/|rust-toolchain\.toml$|\.github/workflows/build\.yml$)'; then
    all
else
    git diff --name-only "$before" "$after" -- apps | cut -d/ -f2 | sort -u |
        while read -r name; do [ ! -f "apps/$name/Cargo.toml" ] || echo "$name"; done
fi | jq -R . | jq -cs .
