#!/usr/bin/env sh
# Builds Lata.app from a compiled binary.
# Usage: ./make-bundle.sh path/to/lulo
set -eu
BIN="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="Lata.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/lulo"
cp "$HERE/Info.plist" "$APP/Contents/Info.plist"
cp "$HERE/../../assets/icons/lata.icns" "$APP/Contents/Resources/lata.icns"
echo "Done: $APP"
