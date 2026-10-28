#!/bin/sh
# Build a Linux release archive for the current architecture.
set -eu
[ "$(uname -s)" = Linux ] || { echo "Build this package on Linux." >&2; exit 1; }
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || { echo "Could not determine the workspace version." >&2; exit 1; }
cargo build --release --locked -p prunemymac
TARGET_DIR=${CARGO_TARGET_DIR:-target}
ARCH=$(uname -m)
STAGING=$(mktemp -d "${TMPDIR:-/tmp}/prunemymac-linux.XXXXXX")
trap 'rm -rf "$STAGING"' EXIT HUP INT TERM
mkdir -p "$STAGING/prunemymac/licenses"
cp "$TARGET_DIR/release/prunemymac" "$STAGING/prunemymac/prunemymac"
cp crates/macscrap/assets/images/chip-cleaner.png "$STAGING/prunemymac/chip-cleaner.png"
cp scripts/install-linux.sh "$STAGING/prunemymac/install.sh"
cp crates/macscrap/assets/fonts/*-OFL.txt rules/linux-intezer/LICENSE vendor/xattr/LICENSE-* "$STAGING/prunemymac/licenses/"
cp rules/linux-intezer/README.md "$STAGING/prunemymac/licenses/linux-yara-rules.md"
ARCHIVE="prunemymac-v${VERSION}-linux-${ARCH}.tar.gz"
tar -czf "$TARGET_DIR/$ARCHIVE" -C "$STAGING" prunemymac
(cd "$TARGET_DIR" && sha256sum "$ARCHIVE" > "${ARCHIVE}.sha256")
printf 'Built %s/%s and its checksum.\n' "$TARGET_DIR" "$ARCHIVE"
