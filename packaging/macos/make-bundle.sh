#!/usr/bin/env sh
# Builds Lata.app from a compiled binary.
# Usage: ./make-bundle.sh path/to/lata
set -eu
BIN="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="Lata.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/lata"
# The app says the version the workspace has, not the one written in the template.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$HERE/../../Cargo.toml" | head -n 1)"
sed "s|<key>CFBundleShortVersionString</key><string>[^<]*</string>|<key>CFBundleShortVersionString</key><string>$VERSION</string>|; s|<key>CFBundleVersion</key><string>[^<]*</string>|<key>CFBundleVersion</key><string>$VERSION</string>|" \
  "$HERE/Info.plist" > "$APP/Contents/Info.plist"
cp "$HERE/../../assets/icons/lata.icns" "$APP/Contents/Resources/lata.icns"
echo "Done: $APP"
