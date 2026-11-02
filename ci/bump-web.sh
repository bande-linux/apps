#!/bin/bash
# bump-web.sh [REV]: pin Browser to one bande-linux/web commit, by default the head of web main.
# REV must be on web main. Rewrites the `rev` of the web git dependencies in the workspace
# Cargo.toml, re-locks only what that needs, then runs ci/gate.sh on Browser.
# BUMP_NO_GATE=1 skips the gate.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
repo=https://github.com/bande-linux/web
rev=${1:-$(git ls-remote "$repo" refs/heads/main | cut -f1)}
[[ $rev =~ ^[0-9a-f]{7,40}$ ]] || { echo "bump-web: $rev is not a commit id" >&2; exit 1; }
status=$(curl -fsS "https://api.github.com/repos/bande-linux/web/compare/main...$rev" | jq -r .status)
case $status in
    identical | behind) ;;
    *) echo "bump-web: $rev is not on web main ($status)" >&2; exit 1 ;;
esac
rev=$(curl -fsS "https://api.github.com/repos/bande-linux/web/commits/$rev" | jq -r .sha)
sed -i -E "/github\.com\/bande-linux\/web\"/s/rev = \"[0-9a-f]+\"/rev = \"$rev\"/" Cargo.toml
cargo metadata -q --format-version 1 > /dev/null
pins=$(grep -o 'bande-linux/web", rev = "[0-9a-f]*"' Cargo.toml | sort -u | wc -l)
[ "$pins" -eq 1 ] || { echo "bump-web: the workspace pins more than one web rev" >&2; exit 1; }
if grep -l 'github.com/bande-linux/web' apps/*/Cargo.toml 2>/dev/null; then
    echo "bump-web: the manifests above pin the web repo themselves; use the workspace dependency" >&2
    exit 1
fi
[ -n "${BUMP_NO_GATE:-}" ] || ci/gate.sh browser
echo "Pin the web engine at ${rev:0:8}"
