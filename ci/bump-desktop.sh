#!/bin/bash
# bump-desktop.sh [REV]: pin every app (and the template) to one bande-linux/desktop commit,
# by default the head of desktop main. REV must be on desktop main. Rewrites the `rev` of each
# desktop git dependency, re-locks only what that needs, then runs ci/gate.sh on every app.
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
mapfile -t manifests < <(grep -l 'github.com/bande-linux/desktop' apps/*/Cargo.toml template/Cargo.toml 2>/dev/null || true)
[ ${#manifests[@]} -gt 0 ] || { echo "bump-desktop: no app depends on the desktop" >&2; exit 1; }
for manifest in "${manifests[@]}"; do
    sed -i -E "/github\.com\/bande-linux\/desktop/s/rev = \"[0-9a-f]+\"/rev = \"$rev\"/" "$manifest"
    cargo metadata -q --format-version 1 --manifest-path "$manifest" > /dev/null
done
pins=$(grep -ho 'bande-linux/desktop", rev = "[0-9a-f]*"' "${manifests[@]}" | sort -u | wc -l)
[ "$pins" -eq 1 ] || { echo "bump-desktop: the apps pin more than one desktop rev" >&2; exit 1; }
[ -n "${BUMP_NO_GATE:-}" ] || ci/gate.sh
echo "Pin the desktop at ${rev:0:8}"
