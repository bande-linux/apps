#!/bin/sh
set -eu
bande-monitor --version
test -f /usr/share/applications/org.bande.ActivityMonitor.desktop
test -f /usr/share/metainfo/org.bande.ActivityMonitor.metainfo.xml
test -f /usr/share/icons/hicolor/scalable/apps/org.bande.ActivityMonitor.svg
