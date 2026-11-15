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
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
# Makes this the program that opens lulo:// links (the browser extension's).
command -v xdg-mime >/dev/null && xdg-mime default "$ID.desktop" x-scheme-handler/lulo || true
echo "Installed to $PREFIX"
