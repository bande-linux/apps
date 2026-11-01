#!/bin/sh
set -eu
bande-software --version
test -f /usr/share/applications/org.bande.Software.desktop
test -f /usr/share/icons/hicolor/scalable/apps/org.bande.Software.svg
test -f /usr/share/metainfo/org.bande.Software.metainfo.xml
