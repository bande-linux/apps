#!/bin/bash
# release-all.sh: after a desktop bump that changes bande-app behaviour, raises the patch version
# of every app in its Cargo.toml and prints the commit and the tags that release them all. It
# changes no git state: run the printed commands when the owner asks for the releases.
set -euo pipefail
cd "$(dirname "$0")/.."
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
tags=()
for manifest in apps/*/Cargo.toml; do
    app=$(basename "$(dirname "$manifest")")
    version=$(sed -n 's/^version = "\([0-9]*\.[0-9]*\.[0-9]*\)"$/\1/p' "$manifest" | head -1)
    [ -n "$version" ] || { echo "release-all: no version in $manifest" >&2; exit 1; }
    next=${version%.*}.$((${version##*.} + 1))
    sed -i "0,/^version = \"$version\"$/s//version = \"$next\"/" "$manifest"
    tags+=("$app-v$next")
done
cargo metadata -q --format-version 1 > /dev/null
echo "Commit, merge to main, then tag that commit:"
echo "  git commit -am 'Release every app for the desktop bump'"
for tag in "${tags[@]}"; do
    echo "  git tag $tag && git push origin $tag"
done
