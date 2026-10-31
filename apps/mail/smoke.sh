#!/bin/sh
set -eu
bande-mail --version
test -f /usr/share/applications/org.bande.Mail.desktop
test -f /usr/share/icons/hicolor/scalable/apps/org.bande.Mail.svg
