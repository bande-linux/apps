#!/bin/bash
# changed-apps.sh <ref type> <ref name> [names]: the apps to build, as a JSON array of directory
# names under apps/. A tag <app>-vX.Y.Z builds that app, and only from a commit on main; a
# dispatch builds the apps it names, or "all".
set -euo pipefail
cd "$(dirname "$0")/.."
ref_type=$1 ref_name=$2 names=${3:-}

if [ "$ref_type" = tag ]; then
    [[ $ref_name =~ ^(.+)-v([0-9]+\.[0-9]+\.[0-9]+)$ ]] || { echo "changed-apps: tag $ref_name is not <app>-vX.Y.Z" >&2; exit 1; }
    names=${BASH_REMATCH[1]}
    git fetch -q origin main
    git merge-base --is-ancestor HEAD origin/main || { echo "changed-apps: tag $ref_name is not on main" >&2; exit 1; }
fi
if [ "$names" = all ]; then
    find apps -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' | xargs -r -n1 basename | sort
else
    for name in $names; do
        [ -f "apps/$name/Cargo.toml" ] || { echo "changed-apps: no app apps/$name" >&2; exit 1; }
        echo "$name"
    done
fi | jq -R . | jq -cs .
