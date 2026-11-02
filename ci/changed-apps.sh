#!/bin/bash
# changed-apps.sh <release tag> [names]: the apps to build, as a JSON array of directory names
# under apps/. A release tag <app>-vX.Y.Z (pushed, or named by a dry-run dispatch) builds that app,
# and only from a commit on main; with an empty tag it builds the apps named, or "all".
set -euo pipefail
cd "$(dirname "$0")/.."
tag=$1 names=${2:-}

if [ -n "$tag" ]; then
    [[ $tag =~ ^(.+)-v([0-9]+\.[0-9]+\.[0-9]+)$ ]] || { echo "changed-apps: tag $tag is not <app>-vX.Y.Z" >&2; exit 1; }
    names=${BASH_REMATCH[1]}
    git fetch -q origin main
    git merge-base --is-ancestor HEAD origin/main || { echo "changed-apps: $tag is not on main" >&2; exit 1; }
fi
if [ "$names" = all ]; then
    find apps -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' | xargs -r -n1 basename | sort
else
    for name in $names; do
        [ -f "apps/$name/Cargo.toml" ] || { echo "changed-apps: no app apps/$name" >&2; exit 1; }
        echo "$name"
    done
fi | jq -R . | jq -cs .
