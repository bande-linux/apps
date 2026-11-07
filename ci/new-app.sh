#!/bin/bash
# new-app.sh <dir> <Id> "<Name>": a new app in apps/<dir>, copied from template/: the package
# bande-<dir>, the app id org.bande.<Id> and the name <Name>, e.g.
# `ci/new-app.sh mail-merge MailMerge "Mail Merge"`. Then checks it with ci/check-app.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
dir=${1:?usage: new-app.sh <dir> <Id> "<Name>"} id=${2:?usage: new-app.sh <dir> <Id> "<Name>"}
name=${3:?usage: new-app.sh <dir> <Id> "<Name>"}
[[ $dir =~ ^[a-z][a-z0-9-]*$ ]] || { echo "new-app: $dir is not a lower-case directory name" >&2; exit 1; }
[[ $id =~ ^[A-Z][A-Za-z0-9]*$ ]] || { echo "new-app: $id is not an id like MailMerge" >&2; exit 1; }
[ ! -e "apps/$dir" ] || { echo "new-app: apps/$dir exists" >&2; exit 1; }
cp -r template "apps/$dir"
target=apps/$dir
for language in "$target"/locales/*/; do mv "${language}bande-template.ftl" "${language}bande-$dir.ftl"; done
for file in "$target"/data/org.bande.Template.*; do
    mv "$file" "${file/org.bande.Template/org.bande.$id}"
done
find "$target" -type f -exec sed -i \
    -e "s/bande-template/bande-$dir/g" \
    -e "s/org\.bande\.Template/org.bande.$id/g" {} +
find "$target/src" -type f -exec sed -i "s/Template/$id/g" {} +
find "$target" -path "$target/src" -prune -o -type f -exec sed -i "s/Template/$name/g" {} +
[ ! -f ~/.cargo/env ] || . ~/.cargo/env
cargo metadata -q --format-version 1 > /dev/null
ci/check-app.sh "$target"
cat <<NEXT
new-app: apps/$dir is bande-$dir (org.bande.$id). Next:
- draw its Bande theme icon in the desktop's data/icons/src/apps.py
- add it to the table in README.md
- if it is a default app, add it to the desktop's packaging/bande-desktop/default-apps
- compile it: cargo check --locked -p bande-$dir, then merge to main
NEXT
