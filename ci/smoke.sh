#!/bin/bash
# smoke.sh [--install] <deb>...: checks an app's packages unpacked into a scratch root. The app
# package ships a .desktop file and metainfo that validate, its binary's --version names it, and apt
# can resolve its dependencies here. A language pack (<app>-l10n-<lang>) ships only catalogs under
# /usr/share/bande/l10n and depends on its app (any version: a translation matches its English text). With --install (as root, in
# CI's clean debian:testing) it then installs them all with apt and checks every installed path
# exists, `<package> --version` names the app, and the app's own apps/<app>/smoke.sh, if any, passes.
set -euo pipefail
cd "$(dirname "$0")/.."
install=0
[ "${1:-}" != --install ] || { install=1; shift; }
[ $# -gt 0 ] || { echo "usage: smoke.sh [--install] <deb>..." >&2; exit 2; }
debs=()
for deb in "$@"; do debs+=("$(realpath "$deb")"); done
fail() { echo "smoke: $1: $2" >&2; exit 1; }
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT

check_app() {
    local deb=$1 package=$2 dir=$root/$2
    dpkg-deb -x "$deb" "$dir"
    mapfile -t desktops < <(find "$dir/usr/share/applications" -name '*.desktop' 2>/dev/null)
    mapfile -t metainfos < <(find "$dir/usr/share/metainfo" -name '*.xml' 2>/dev/null)
    [ ${#desktops[@]} -gt 0 ] || fail "$package" "ships no .desktop file"
    [ ${#metainfos[@]} -gt 0 ] || fail "$package" "ships no metainfo"
    desktop-file-validate "${desktops[@]}"
    appstreamcli validate --no-net "${metainfos[@]}"
    "$dir/usr/bin/$package" --version | grep -q "^$package " || fail "$package" "$dir/usr/bin/$package --version does not name $package"
}

check_pack() {
    local deb=$1 package=$2 app=${2%%-l10n-*} dir=$root/$2
    dpkg-deb -x "$deb" "$dir"
    [ -n "$(find "$dir/usr/share/bande/l10n" -name '*.mo' 2>/dev/null)" ] || fail "$package" "ships no catalog"
    [ -z "$(find "$dir" -type f -not -path "$dir/usr/share/bande/l10n/*" -not -path "$dir/usr/share/doc/*")" ] || fail "$package" "ships files outside its catalogs"
    dpkg-deb -f "$deb" Depends | grep -qE "(^|, )$app( |,|$)" || fail "$package" "does not depend on $app"
}

apps=()
for deb in "${debs[@]}"; do
    package=$(dpkg-deb -f "$deb" Package)
    case $package in
        *-l10n-*) check_pack "$deb" "$package" ;;
        *) check_app "$deb" "$package"; apps+=("$package") ;;
    esac
done
[ ${#apps[@]} -gt 0 ] || { echo "smoke: no app package among: $*" >&2; exit 1; }
apt-get install -s -qq "${debs[@]}" > /dev/null || fail "${apps[*]}" "apt cannot resolve the dependencies"
if [ $install -eq 1 ]; then
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${debs[@]}" > /dev/null
    for deb in "${debs[@]}"; do
        package=$(dpkg-deb -f "$deb" Package)
        while read -r path; do
            [ -e "$path" ] || fail "$package" "installed $path, which is missing"
        done < <(dpkg -L "$package")
    done
    for package in "${apps[@]}"; do
        "$package" --version | grep -q "^$package " || fail "$package" "$package --version does not name $package"
        app=apps/${package#bande-}
        [ ! -x "$app/smoke.sh" ] || "$app/smoke.sh"
    done
fi
echo "smoke: ${apps[*]} passed with $((${#debs[@]} - ${#apps[@]})) language packs"
