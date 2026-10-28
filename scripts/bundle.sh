#!/bin/sh
# Build a release prunemymac.app in target/. Usage: scripts/bundle.sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release -p prunemymac
APP=target/prunemymac.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/prunemymac "$APP/Contents/MacOS/prunemymac"
ICONSET=$(mktemp -d)/prunemymac.iconset
mkdir -p "$ICONSET"
LOGO="$(dirname "$0")/../crates/macscrap/assets/images/chip-cleaner.png"
for SIZE in 16 32 128 256 512; do
  sips -z "$SIZE" "$SIZE" "$LOGO" --out "$ICONSET/icon_${SIZE}x${SIZE}.png" >/dev/null
  DOUBLE=$((SIZE * 2))
  sips -z "$DOUBLE" "$DOUBLE" "$LOGO" --out "$ICONSET/icon_${SIZE}x${SIZE}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/prunemymac.icns"
rm -rf "$(dirname "$ICONSET")"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>prunemymac</string>
  <key>CFBundleDisplayName</key><string>prunemymac</string>
  <key>CFBundleIdentifier</key><string>dev.prunemymac.app</string>
  <key>CFBundleExecutable</key><string>prunemymac</string>
  <key>CFBundleIconFile</key><string>prunemymac</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
</dict></plist>
PLIST
# Ad-hoc sign so macOS can remember Full Disk Access for this bundle.
codesign --force --sign - "$APP"
echo "Built $APP"
