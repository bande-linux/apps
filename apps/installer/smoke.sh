#!/bin/sh
set -eu
bande-installer --version
test -f /usr/share/applications/org.bande.Installer.desktop
test -f /usr/share/icons/hicolor/scalable/apps/org.bande.Installer.svg
