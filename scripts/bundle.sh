#!/bin/sh
# Build a release macscrap.app in target/. Usage: scripts/bundle.sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release -p macscrap
APP=target/macscrap.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/macscrap "$APP/Contents/MacOS/macscrap"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>macscrap</string>
  <key>CFBundleIdentifier</key><string>dev.macscrap.app</string>
  <key>CFBundleExecutable</key><string>macscrap</string>
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
