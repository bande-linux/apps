#!/bin/bash
# build-packs.sh <out>: the language packs, bande-<app>-l10n-<lang>, of every released app, checked
# against the code's English and built from the translations alone, each at the version
# ci/version.sh gives the l10n-YYYYMMDD tag (.github/workflows/l10n.yml).
set -euo pipefail
cd "$(dirname "$0")/.."
out=$(realpath -m "${1:?usage: build-packs.sh <out>}")
target=${CARGO_TARGET_DIR:-target}
ci/l10n-tool.sh check ci/l10n-complete
rm -rf "$target/l10n"
ci/l10n-tool.sh pack "$target/l10n"
mkdir -p "$out"
for app in $(find apps -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' | xargs -rn1 basename | sort); do
    if [ -z "$(git tag --merged HEAD -l "$app-v[0-9]*.[0-9]*.[0-9]*")" ]; then
        echo "build-packs: $app has no release yet"
        continue
    fi
    ci/l10n-tool.sh debs "$target/l10n" "$out" "$(ci/version.sh "$app")" "bande-$app" ci/copyright
done
ls "$out"
