#!/bin/sh
# Renders each screen on the simulator with sample data, in light and dark, into
# build/screens, for review. Build for the simulator first (`bun run ios:build:sim`, or
# with SIM_UDID set). The app's `-screen` and `-frozen` launch arguments pick the screen
# and hold the clock at the samples' time.
set -eu
UDID=${SIM_UDID:-$(cat build/sim-udid)}
APP=build/DerivedData/Build/Products/Debug-iphonesimulator/ScentControl.app
SCREENS=${SCREENS:-home diffuser diffuserDetails diffuserOff diffuserNotKnown schedule copy add log settings}
MODES=${MODES:-light dark}

xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
xcrun simctl status_bar "$UDID" override --time "7:42" --batteryState charged --batteryLevel 100 \
  --wifiBars 3 --cellularMode active --cellularBars 4
xcrun simctl install "$UDID" "$APP"
mkdir -p build/screens
for mode in $MODES; do
  xcrun simctl ui "$UDID" appearance "$mode"
  for screen in $SCREENS; do
    xcrun simctl terminate "$UDID" dev.scentcontrol.app 2>/dev/null || true
    xcrun simctl launch "$UDID" dev.scentcontrol.app -screen "$screen" -frozen YES >/dev/null
    sleep 3
    xcrun simctl io "$UDID" screenshot "build/screens/$screen-$mode.png" >/dev/null 2>&1
    echo "build/screens/$screen-$mode.png"
  done
done
