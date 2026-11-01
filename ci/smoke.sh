#!/bin/bash
# smoke.sh <package>: checks an installed app package: `<package> --version` prints its name,
# every path the package installed exists, and its .desktop and metainfo files validate.
set -euo pipefail
package=${1:?usage: smoke.sh <package>}
"$package" --version | grep -q "^$package " || { echo "smoke: $package --version does not name $package" >&2; exit 1; }
while read -r path; do
    [ -e "$path" ] || { echo "smoke: $package installed $path, which is missing" >&2; exit 1; }
done < <(dpkg -L "$package")
mapfile -t desktops < <(dpkg -L "$package" | grep '^/usr/share/applications/.*\.desktop$')
mapfile -t metainfos < <(dpkg -L "$package" | grep '^/usr/share/metainfo/.*\.xml$')
[ ${#desktops[@]} -gt 0 ] || { echo "smoke: $package installs no .desktop file" >&2; exit 1; }
[ ${#metainfos[@]} -gt 0 ] || { echo "smoke: $package installs no metainfo" >&2; exit 1; }
desktop-file-validate "${desktops[@]}"
appstreamcli validate --no-net "${metainfos[@]}"
echo "smoke: $package passed"
