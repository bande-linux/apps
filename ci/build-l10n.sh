#!/bin/bash
# build-l10n.sh <catalogs> <app> <out dir> <version>: one bande-<app>-l10n-<lang> package per
# language that has a catalog for one of the app's programs, from bande-l10n-tool pack's output.
set -euo pipefail
cd "$(dirname "$0")/.."
catalogs=$(realpath "${1:?usage: build-l10n.sh <catalogs> <app> <out dir> <version>}")
app=${2:?} out=$(realpath -m "${3:?}") version=${4:?}
name=bande-$app
maintainer="Mikael Hug <mikaelhug@gmail.com>"
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
mapfile -t programs < <(cargo metadata --no-deps --format-version 1 |
    jq -r --arg name "$name" '.packages[] | select(.name == $name) | .targets[] | select(.kind[] == "bin") | .name')
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$out"
for folder in "$catalogs"/packs/*/; do
    language=$(basename "$folder")
    package="$name-l10n-${language,,}"
    root="$scratch/$package"
    for program in "${programs[@]}"; do
        [ -d "$folder/$program" ] || continue
        mkdir -p "$root/usr/share/bande/l10n"
        cp -r "$folder/$program" "$root/usr/share/bande/l10n/"
    done
    [ -d "$root" ] || continue
    team=$(sed -n 's/^"Language-Team: \(.*\)\\n"$/\1/p' "apps/$app/locales/$language.po" 2>/dev/null | head -n1)
    [ -n "$team" ] || team=$(find apps libs -path "*/locales/$language.po" -exec sed -n 's/^"Language-Team: \(.*\)\\n"$/\1/p' {} + | head -n1)
    [ -n "$team" ] || { echo "build-l10n: no Language-Team for $language" >&2; exit 1; }
    mkdir -p "$root/DEBIAN" "$root/usr/share/doc/$package"
    printf 'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nUpstream-Name: Bande\nSource: https://github.com/bande-linux/apps\n\nFiles: *\nCopyright: 2026 %s\nLicense: Apache-2.0 or MIT\n On Debian systems, the full text of the Apache License, Version 2.0 can be\n found in /usr/share/common-licenses/Apache-2.0.\n' "$maintainer" > "$root/usr/share/doc/$package/copyright"
    printf '%s (%s) unstable; urgency=medium\n\n  * Built from the translations at this version.\n\n -- %s  %s\n' \
        "$package" "$version" "$maintainer" "$(date -R -d "@$SOURCE_DATE_EPOCH")" | gzip -9n > "$root/usr/share/doc/$package/changelog.gz"
    find "$root" -type d -exec chmod 755 {} + -o -type f -exec chmod 644 {} +
    cat > "$root/DEBIAN/control" << CONTROL
Package: $package
Version: $version
Architecture: all
Maintainer: $maintainer
Installed-Size: $(du -sk --exclude=DEBIAN "$root" | cut -f1)
Depends: $name
Enhances: $name
Section: localization
Priority: optional
Multi-Arch: foreign
Description: $team translations for $name
CONTROL
    echo "The $team text of $name, read when it starts in a $team session. Without it the app shows English." |
        fold -s -w 78 | sed 's/ *$//; s/^/ /' >> "$root/DEBIAN/control"
    (cd "$root" && find usr -type f -print0 | sort -z | xargs -0 md5sum) > "$root/DEBIAN/md5sums"
    dpkg-deb --root-owner-group -Zxz --build "$root" "$out/${package}_${version}_all.deb" > /dev/null
    echo "build-l10n: ${package}_${version}_all.deb"
done
