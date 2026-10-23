"""Copies what the Mac app knows into build/Seed/seed.json, for the phone's first run.

The phone starts from this file when it has no saved state of its own, so it knows the
broker and the diffusers the Mac set up without setting any of them up again. Only the
broker's address, username and route ID, and each diffuser's ID, name, room, info and last
schedule are copied: no Bluetooth IDs (each phone and Mac has its own), no readings, and no
password, which the Mac keeps in its Keychain and this never reads. The file stays in the
gitignored build folder and goes only into local builds. Without the Mac's file, nothing is
written and the phone starts empty.

Run from apps/ios: python3 scripts/seed-from-mac.py
"""

import json
import os
import pathlib
import sys

MAC_STATE = pathlib.Path.home() / "Library/Application Support/ScentControl/desktop.json"
SEED = pathlib.Path("build/Seed/seed.json")
DEVICE_KEYS = ("id", "label", "room", "info", "sent_profile", "sent_profile_at")
BROKER_KEYS = ("url", "username", "route_id")


def main() -> int:
    if not MAC_STATE.exists():
        if SEED.exists():
            SEED.unlink()
        print("No Mac app state found; the phone will start empty.")
        return 0
    state = json.loads(MAC_STATE.read_text())
    broker = {key: state.get("broker", {}).get(key, "") for key in BROKER_KEYS}
    devices = [
        {key: device[key] for key in DEVICE_KEYS if key in device}
        for device in state.get("devices", [])
    ]
    for device in devices:
        device.setdefault("readings", {})
    seed = {"broker": broker, "devices": devices}
    text = json.dumps(seed, indent=2, sort_keys=True)
    if "password" in text.lower():
        print("Refusing to write a seed that mentions a password.", file=sys.stderr)
        return 1
    SEED.parent.mkdir(parents=True, exist_ok=True)
    SEED.write_text(text + "\n")
    os.chmod(SEED, 0o600)
    print(f"Seeded {len(devices)} diffusers and the broker's address from the Mac app.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
