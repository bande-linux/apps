#!/bin/bash
# version.sh <app>: the package version of apps/<app>. Built from its release tag <app>-vX.Y.Z
# (GITHUB_REF_TYPE=tag) it is X.Y.Z, which must equal the app's Cargo version; an unpublished build is
# <Cargo version>+git<date>.<count>.<sha>, never published. From a translation-only tag
# l10n-YYYYMMDD[.N] it is the app's last release before it, X.Y.Z+l10nYYYYMMDD[.N].
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: version.sh <app>}
declared=$(cargo metadata --no-deps --format-version 1 | jq -r --arg name "bande-$app" '.packages[] | select(.name == $name) | .version')
[ -n "$declared" ] || { echo "version: no package bande-$app" >&2; exit 1; }
if [ "${GITHUB_REF_TYPE:-}" = tag ] && [[ ${GITHUB_REF_NAME:-} =~ ^l10n-([0-9]{8})(\.[0-9]+)?$ ]]; then
    stamp=${BASH_REMATCH[1]}${BASH_REMATCH[2]}
    last=$(git describe --tags --abbrev=0 --match "$app-v[0-9]*.[0-9]*.[0-9]*" HEAD 2>/dev/null) ||
        { echo "version: $GITHUB_REF_NAME has no $app-vX.Y.Z release before it" >&2; exit 1; }
    echo "${last#"$app"-v}+l10n$stamp"
    exit 0
fi
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
