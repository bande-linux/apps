#!/bin/bash
# version.sh <app>: the package version of apps/<app>. Built from its release tag <app>-vX.Y.Z
# (GITHUB_REF_TYPE=tag) it is X.Y.Z, which must equal the app's Cargo version; a test build is
# <Cargo version>+git<date>.<count>.<sha>, never published.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: version.sh <app>}
declared=$(cargo metadata --no-deps --format-version 1 | jq -r --arg name "bande-$app" '.packages[] | select(.name == $name) | .version')
[ -n "$declared" ] || { echo "version: no package bande-$app" >&2; exit 1; }
if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
    tagged=${GITHUB_REF_NAME#"$app"-v}
    [ "$tagged" = "$declared" ] || { echo "version: tag $GITHUB_REF_NAME but apps/$app/Cargo.toml says $declared" >&2; exit 1; }
    echo "$declared"
    exit 0
fi
paths=("apps/$app" ci Cargo.toml Cargo.lock rust-toolchain.toml .github/workflows/build.yml)
count=$(git rev-list --count HEAD -- "${paths[@]}")
date=$(TZ=UTC git log -1 --format=%cd --date=format-local:%Y%m%d -- "${paths[@]}")
sha=$(git log -1 --format=%h --abbrev=7 -- "${paths[@]}")
echo "$declared+git$date.$count.$sha"
