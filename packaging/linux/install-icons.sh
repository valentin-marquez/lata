#!/usr/bin/env sh
# Installs the .desktop entry and icons into the hicolor theme.
# Usage: ./install-icons.sh [prefix]   (default ~/.local; use /usr for a system install)
set -eu
ID=io.github.valentin_marquez.Lata
PREFIX="${1:-$HOME/.local}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PNG="$HERE/../../assets/icons/png"
for s in 16 24 32 48 64 128 256 512; do
  install -Dm644 "$PNG/lata-$s.png" "$PREFIX/share/icons/hicolor/${s}x${s}/apps/$ID.png"
done
install -Dm644 "$HERE/$ID.desktop" "$PREFIX/share/applications/$ID.desktop"
# The entry runs `lata` from PATH. In the release package the binary sits two folders up and is
# in nobody's PATH, so point the entry at it; otherwise the menu entry and lata:// links fail.
BIN="$(cd "$HERE/../.." && pwd)/lata"
if [ -f "$BIN" ] && [ -x "$BIN" ]; then
  # ponytail: a path with | or & or " would break this sed; quote it properly if someone has one.
  sed -i "s|^Exec=lata |Exec=\"$BIN\" |" "$PREFIX/share/applications/$ID.desktop"
fi
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
# Makes this the program that opens lata:// links (the browser extension's).
command -v xdg-mime >/dev/null && xdg-mime default "$ID.desktop" x-scheme-handler/lata || true
echo "Installed to $PREFIX"
