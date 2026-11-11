#!/bin/bash
# debian-release.sh: the Debian release suffix of this build, ~debNN: NN is the version number of
# the Debian release the build runs on (VERSION_CODENAME in $OS_RELEASE, numbered by
# distro-info-data). Every package built for one Debian release carries it after the release
# version, as Debian's backports carry ~bpoNN: X.Y.Z~deb14 sorts below X.Y.Z and below
# X.Y.Z~deb15, so upgrading Debian upgrades Bande to the build for the new release.
set -euo pipefail
os_release=${OS_RELEASE:-/etc/os-release}
csv=${DISTRO_INFO_CSV:-/usr/share/distro-info/debian.csv}
codename=$(sed -n 's/^VERSION_CODENAME=//p' "$os_release" | tr -d '"')
[ -n "$codename" ] || { echo "debian-release.sh: $os_release names no VERSION_CODENAME" >&2; exit 1; }
[ -r "$csv" ] || { echo "debian-release.sh: $csv is missing; install distro-info-data" >&2; exit 1; }
number=$(awk -F, -v c="$codename" '$3 == c && $1 ~ /^[0-9]+$/ { print $1 }' "$csv")
[ -n "$number" ] || { echo "debian-release.sh: Debian $codename has no version number in $csv" >&2; exit 1; }
echo "~deb$number"
