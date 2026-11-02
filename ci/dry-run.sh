#!/bin/bash
# dry-run.sh <app>: dispatches build.yml on main with tag=<app>-v<version of apps/<app> on
# origin/main>, which builds and checks that commit exactly as pushing the tag would, without
# publishing, and waits for it. When it passes, tag the commit it prints. Needs gh.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: dry-run.sh <app>}
repo=bande-linux/apps
git fetch -q origin main
sha=$(git rev-parse origin/main)
version=$(git show "origin/main:apps/$app/Cargo.toml" | sed -n 's/^version = "\([0-9]*\.[0-9]*\.[0-9]*\)"$/\1/p' | head -1)
[ -n "$version" ] || { echo "dry-run: no version in apps/$app/Cargo.toml on origin/main" >&2; exit 1; }
tag=$app-v$version
if git ls-remote --exit-code -q --tags origin "refs/tags/$tag" > /dev/null; then
    echo "dry-run: $tag is already released; bump apps/$app/Cargo.toml to release again" >&2
fi
run=$(gh api -X POST "repos/$repo/actions/workflows/build.yml/dispatches" -f ref=main -f "inputs[tag]=$tag" \
    -F return_run_details=true --jq .workflow_run_id)
[ -n "$run" ] || { echo "dry-run: GitHub returned no run" >&2; exit 1; }
built=$(gh api "repos/$repo/actions/runs/$run" --jq .head_sha)
echo "dry-run: $tag on $built: https://github.com/$repo/actions/runs/$run"
[ "$built" = "$sha" ] || echo "dry-run: main moved since the fetch; the run built $built" >&2
gh run watch -R "$repo" --exit-status --interval 30 "$run" > /dev/null ||
    { gh run view -R "$repo" --log-failed "$run" | tail -50; echo "dry-run: $tag failed" >&2; exit 1; }
echo "dry-run: $tag passed; release it with: git tag $tag $built && git push origin $tag"
