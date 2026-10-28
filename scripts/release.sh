#!/bin/sh
# Build a universal macOS DMG and its SHA-256 checksum in target/.
set -eu
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || { echo "Could not determine the workspace version" >&2; exit 1; }
scripts/bundle.sh --universal
lipo target/prunemymac.app/Contents/MacOS/prunemymac -verify_arch arm64 x86_64
codesign --verify --deep --strict target/prunemymac.app

STAGING=$(mktemp -d "${TMPDIR:-/tmp}/prunemymac-release.XXXXXX")
trap 'rm -rf "$STAGING"' EXIT HUP INT TERM
ditto target/prunemymac.app "$STAGING/prunemymac.app"
ln -s /Applications "$STAGING/Applications"
DMG="prunemymac-v${VERSION}-universal.dmg"
hdiutil create -volname "prunemymac" -srcfolder "$STAGING" -ov -format UDZO "target/$DMG"
(cd target && shasum -a 256 "$DMG" > SHA256SUMS.txt)
echo "Built target/$DMG and target/SHA256SUMS.txt"
