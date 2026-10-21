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

# Settings → About shows the commit after the version. Empty outside a git checkout.
SCENT_BUILD_COMMIT="$(git rev-parse --short HEAD 2>/dev/null || true)"
export SCENT_BUILD_COMMIT

# The broker the app starts with when nothing is saved: SCENT_BROKER_URL,
# SCENT_BROKER_USERNAME and SCENT_ROUTE_ID, from the shell or the repo-root .env (see
# .env.example). Without them the app starts with no broker and asks for one in
# Settings. A password is never compiled in: it goes in the Keychain, from Settings.
# shellcheck source=scripts/broker-env.sh
source "$root/scripts/broker-env.sh"
load_broker_env "$root/.env"
unset_passwords
echo "Default broker: ${SCENT_BROKER_URL:-none, so the app asks for one in Settings}" >&2
cargo build --release -p scent-desktop

app="target/Scent Control.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/scent-control "$app/Contents/MacOS/scent-control"
cp apps/desktop/macos/Info.plist "$app/Contents/Info.plist"
# The crate's version, as Settings → About shows it, so that Get Info agrees.
version="$(cargo pkgid -p scent-desktop)"
version="${version##*[@#]}"
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$app/Contents/Info.plist"
# Drawn by the ignored `render_app_icon` test (apps/desktop/src/ui/icon.rs).
cp apps/desktop/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"

# Ad hoc signature: enough for a local app and for macOS to read the Info.plist.
codesign --force --sign - "$app"

echo "$root/$app"
