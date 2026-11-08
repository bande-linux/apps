#!/bin/bash
# l10n-tool.sh <args>: runs bande-l10n-tool from the desktop at the workspace's pinned rev (the same
# tool, catalogs and checks the desktop uses), installed once per rev under the cache.
set -euo pipefail
cd "$(dirname "$0")/.."
rev=$(grep -o 'bande-linux/desktop", rev = "[0-9a-f]*"' Cargo.toml | head -n1 | grep -o '[0-9a-f]\{40\}')
[ -n "$rev" ] || { echo "l10n-tool: no pinned desktop rev in Cargo.toml" >&2; exit 1; }
root=${XDG_CACHE_HOME:-$HOME/.cache}/bande-ci/l10n-tool-$rev
if [ ! -x "$root/bin/bande-l10n-tool" ]; then
    cargo install --quiet --locked --git https://github.com/bande-linux/desktop --rev "$rev" \
        --root "$root" bande-l10n-tool
fi
exec "$root/bin/bande-l10n-tool" "$@"
