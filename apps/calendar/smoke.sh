#!/bin/sh
set -eu
bande-calendar --version
test -f /usr/share/applications/org.bande.Calendar.desktop
test -f /usr/share/icons/hicolor/scalable/apps/org.bande.Calendar.svg
