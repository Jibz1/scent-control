#!/usr/bin/env bash
# Build the macOS app bundle: target/Scent Control.app
#
# Usage:
#   scripts/bundle-macos.sh
#   open "target/Scent Control.app"
#
# Always launch with `open`, never by running target/release/scent-control directly.
# macOS kills (SIGABRT) any process that touches Bluetooth unless the app responsible
# for it declares NSBluetoothAlwaysUsageDescription in its Info.plist. Only the bundle
# has that plist.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

cargo build --release -p scent-desktop

app="target/Scent Control.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/scent-control "$app/Contents/MacOS/scent-control"
cp apps/desktop/macos/Info.plist "$app/Contents/Info.plist"
# Drawn by the ignored `render_app_icon` test (apps/desktop/src/ui/icon.rs).
cp apps/desktop/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"

# Ad hoc signature: enough for a local app and for macOS to read the Info.plist.
codesign --force --sign - "$app"

echo "$root/$app"
