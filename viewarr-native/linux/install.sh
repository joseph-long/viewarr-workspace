#!/bin/sh
# Install viewarr for the current user: the binary into ~/.local/bin and a
# desktop entry that lists it under Open With for FITS files.
#
#   ./install.sh                  into ~/.local
#   PREFIX=/usr/local ./install.sh   system-wide (run as root)
#
# FITS itself needs no registering: shared-mime-info already defines
# image/fits, which is what the desktop entry claims.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"

install -Dm755 "$here/viewarr" "$prefix/bin/viewarr"
install -Dm644 "$here/viewarr.desktop" "$prefix/share/applications/viewarr.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi

echo "Installed $prefix/bin/viewarr"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Note: $prefix/bin is not on your PATH" ;;
esac
