#!/bin/sh
# Install a Linux build for the current user without administrator access.
set -eu
[ "$(uname -s)" = Linux ] || { echo "This installer requires Linux." >&2; exit 1; }
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -f "$SCRIPT_DIR/prunemymac" ]; then
  BINARY="$SCRIPT_DIR/prunemymac"
  ICON="$SCRIPT_DIR/chip-cleaner.png"
else
  BINARY="$SCRIPT_DIR/../target/release/prunemymac"
  ICON="$SCRIPT_DIR/../crates/macscrap/assets/images/chip-cleaner.png"
fi
[ -f "$BINARY" ] && [ -f "$ICON" ] || { echo "Build the app first with scripts/bundle-linux.sh." >&2; exit 1; }
BIN_DIR="${HOME:?}/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
case "$BIN_DIR$DATA_DIR" in *'
'*) echo "Installation paths must not contain newlines." >&2; exit 1 ;; esac
mkdir -p "$BIN_DIR" "$DATA_DIR/applications" "$DATA_DIR/icons"
install -m 755 "$BINARY" "$BIN_DIR/prunemymac"
install -m 644 "$ICON" "$DATA_DIR/icons/prunemymac.png"
EXEC_PATH=$(printf '%s' "$BIN_DIR/prunemymac" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/[$`]/\\&/g' -e 's/%/%%/g')
cat > "$DATA_DIR/applications/prunemymac.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=prunemymac
Comment=Storage, performance, and protection tools
Exec="$EXEC_PATH"
Icon=$DATA_DIR/icons/prunemymac.png
Terminal=false
Categories=Utility;System;
DESKTOP
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DATA_DIR/applications" || true
fi
printf 'Installed %s and the desktop launcher.\n' "$BIN_DIR/prunemymac"
