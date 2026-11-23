#!/bin/sh
# Install the latest prunemymac release into /Applications.
#   curl -fsSL https://raw.githubusercontent.com/omraval18/prunemymac/main/scripts/install-mac.sh | sh
# Set PRUNEMYMAC_VERSION=v0.2.1 to install a specific release.
set -eu
[ "$(uname -s)" = Darwin ] || { echo "This installer requires macOS." >&2; exit 1; }
REPO=omraval18/prunemymac
if [ -n "${PRUNEMYMAC_VERSION:-}" ]; then
  TAG=$PRUNEMYMAC_VERSION
else
  TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
fi
[ -n "$TAG" ] || { echo "Could not find the latest release." >&2; exit 1; }
DMG="prunemymac-${TAG}-universal.dmg"
BASE="https://github.com/$REPO/releases/download/$TAG"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/prunemymac-install.XXXXXX")
MOUNT="$WORK/mnt"
cleanup() { hdiutil detach -quiet "$MOUNT" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT HUP INT TERM

echo "Downloading prunemymac $TAG..."
curl -fL --progress-bar -o "$WORK/$DMG" "$BASE/$DMG"
curl -fsSL -o "$WORK/SHA256SUMS.txt" "$BASE/SHA256SUMS.txt"
(cd "$WORK" && grep " $DMG\$" SHA256SUMS.txt | shasum -a 256 -c - >/dev/null) || { echo "Checksum verification failed." >&2; exit 1; }

mkdir -p "$MOUNT"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$WORK/$DMG"
DEST=/Applications
[ -w "$DEST" ] || DEST="$HOME/Applications"
mkdir -p "$DEST"
osascript -e 'tell application "prunemymac" to quit' >/dev/null 2>&1 || true
rm -rf "$DEST/prunemymac.app"
ditto "$MOUNT/prunemymac.app" "$DEST/prunemymac.app"
xattr -dr com.apple.quarantine "$DEST/prunemymac.app" 2>/dev/null || true

echo "Installed $DEST/prunemymac.app"
echo "For complete scans, grant Full Disk Access in System Settings > Privacy & Security."
open "$DEST/prunemymac.app"
