#!/bin/sh
set -eu
bande-terminal --version
test -f /usr/share/applications/org.bande.Terminal.desktop
test -f /usr/share/icons/hicolor/scalable/apps/org.bande.Terminal.svg
test -f /usr/share/metainfo/org.bande.Terminal.metainfo.xml
