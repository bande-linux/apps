#!/bin/bash
# bump-desktop.sh [REV]: pin every app (and the template) to one bande-linux/desktop commit,
# by default the head of desktop main. REV must be on desktop main. Rewrites the `rev` of the
# desktop git dependencies in the workspace Cargo.toml, re-locks only what that needs, then runs
# ci/gate.sh on every app.
# BUMP_NO_GATE=1 skips the gate.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
repo=https://github.com/bande-linux/desktop
rev=${1:-$(git ls-remote "$repo" refs/heads/main | cut -f1)}
[[ $rev =~ ^[0-9a-f]{7,40}$ ]] || { echo "bump-desktop: $rev is not a commit id" >&2; exit 1; }
status=$(curl -fsS "https://api.github.com/repos/bande-linux/desktop/compare/main...$rev" | jq -r .status)
case $status in
    identical | behind) ;;
    *) echo "bump-desktop: $rev is not on desktop main ($status)" >&2; exit 1 ;;
esac
rev=$(curl -fsS "https://api.github.com/repos/bande-linux/desktop/commits/$rev" | jq -r .sha)
sed -i -E "/github\.com\/bande-linux\/desktop/s/rev = \"[0-9a-f]+\"/rev = \"$rev\"/" Cargo.toml
cargo metadata -q --format-version 1 > /dev/null
pins=$(grep -o 'bande-linux/desktop", rev = "[0-9a-f]*"' Cargo.toml | sort -u | wc -l)
[ "$pins" -eq 1 ] || { echo "bump-desktop: the workspace pins more than one desktop rev" >&2; exit 1; }
if grep -l 'github.com/bande-linux/desktop' apps/*/Cargo.toml template/Cargo.toml 2>/dev/null; then
    echo "bump-desktop: the manifests above pin the desktop themselves; use the workspace dependency" >&2
    exit 1
fi
[ -n "${BUMP_NO_GATE:-}" ] || ci/gate.sh
echo "Pin the desktop at ${rev:0:8}"
