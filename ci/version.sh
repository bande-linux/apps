#!/bin/bash
# version.sh <app>: the package version of apps/<app>, <Cargo version>+git<date>.<count>.<sha>,
# from the commits that touch the app or the shared build. Any change that rebuilds an app also
# makes its version newer, and rebuilding the same commit gives the same version.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: version.sh <app>}
paths=("apps/$app" ci rust-toolchain.toml .github/workflows/build.yml)
declared=$(cargo metadata --no-deps --format-version 1 --manifest-path "apps/$app/Cargo.toml" | jq -r '.packages[0].version')
count=$(git rev-list --count HEAD -- "${paths[@]}")
date=$(TZ=UTC git log -1 --format=%cd --date=format-local:%Y%m%d -- "${paths[@]}")
sha=$(git log -1 --format=%h --abbrev=7 -- "${paths[@]}")
echo "$declared+git$date.$count.$sha"
