#!/bin/bash
# smoke.sh [--install] <deb>: checks an app package unpacked into a scratch root: it ships a
# .desktop file and metainfo that validate, its binary's --version names it, and apt can resolve
# its dependencies here. With --install (as root, in CI's clean debian:testing) it then installs the
# package with apt and checks it installed: every path it installed exists, `<package> --version`
# names it, and the app's own apps/<app>/smoke.sh, if any, passes.
set -euo pipefail
cd "$(dirname "$0")/.."
install=0
[ "${1:-}" != --install ] || { install=1; shift; }
deb=$(realpath "${1:?usage: smoke.sh [--install] <deb>}")
package=$(dpkg-deb -f "$deb" Package)
fail() { echo "smoke: $package: $*" >&2; exit 1; }
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
dpkg-deb -x "$deb" "$root"
mapfile -t desktops < <(find "$root/usr/share/applications" -name '*.desktop' 2>/dev/null)
mapfile -t metainfos < <(find "$root/usr/share/metainfo" -name '*.xml' 2>/dev/null)
[ ${#desktops[@]} -gt 0 ] || fail "ships no .desktop file"
[ ${#metainfos[@]} -gt 0 ] || fail "ships no metainfo"
desktop-file-validate "${desktops[@]}"
appstreamcli validate --no-net "${metainfos[@]}"
"$root/usr/bin/$package" --version | grep -q "^$package " || fail "$root/usr/bin/$package --version does not name $package"
apt-get install -s -qq "$deb" > /dev/null || fail "apt cannot resolve its dependencies"
if [ $install -eq 1 ]; then
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$deb" > /dev/null
    while read -r path; do
        [ -e "$path" ] || fail "installed $path, which is missing"
    done < <(dpkg -L "$package")
    "$package" --version | grep -q "^$package " || fail "$package --version does not name $package"
    app=apps/${package#bande-}
    [ ! -x "$app/smoke.sh" ] || "$app/smoke.sh"
fi
echo "smoke: $package passed"
