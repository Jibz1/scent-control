# Scent Control

Self-hosted replacement for the Scentiment app and cloud: our own app and backend
that control two Scentiment diffusers, including scheduling. No dependency on
Scentiment's servers at any point.

## Background

- The official Scentiment app contacts Scentiment's servers before it will connect
  to the diffusers (over Bluetooth or WiFi). That check fails, so the app can't reach
  the diffusers. The diffusers themselves work fine using their physical buttons.
- Prior art: Scent Assistant, https://github.com/mr-sparks/scent-assistant
  (MIT-licensed Home Assistant integration). It confirms the **Scentiment Diffuser
  Air 2** works over BLE using a JSON-over-BLE protocol: power, status, intensity
  (1-3), RGB LED, battery. Use its Scentiment source code in
  `custom_components/scent_assistant/` as the protocol reference. Its PROTOCOL.md
  covers other brands but not Scentiment in detail.
- If we reuse its code, keep the MIT copyright notice (add THIRD_PARTY_NOTICES.md).

## Open questions (resolve early, record answers in docs/findings.md)

- Exact models of both diffusers (Air 2, Pro 2, Mini 2, other?). Only Air 2 is confirmed.
- Does the device store schedules on-board, or must schedules be driven externally?
  Scent Assistant exposes no schedule controls for Scentiment, so assume external.
- Does BLE require pairing, a PIN, or a key from Scentiment's cloud?
- WiFi: which hosts do the diffusers contact, over what protocol, with or without TLS?

## Architecture

```
App (PWA) -> Backend (API + scheduler + DB) <-> MQTT broker <-> Bridge (BLE, at home) -> diffusers
Later, optional: diffusers' WiFi -> our backend directly (second transport)
```

Design principles:

- **Desired state, not one-off commands.** The backend stores what each device
  should be doing right now (computed from schedules plus manual overrides). The
  bridge compares actual vs desired on an interval and on every reconnect, and fixes
  any mismatch. A diffuser that was offline at 7am catches up when it comes back.
- **Transport abstraction.** A `DiffuserTransport` interface (connect, get_state,
  set_power, set_intensity, set_led). Implement BLE first; WiFi can be added later
  without touching the app or backend.
- **Bridge connects outbound only** (MQTT), so the backend can run at home or in
  the cloud.
- **Bridge caches the effective schedule locally** so an internet or backend
  outage doesn't stop scheduled runs.
- **BLE allows one connection at a time.** Connect on demand and disconnect soon
  after to save battery. The Scentiment phone app must be closed.
- The app never uses Bluetooth; it only talks to the backend over HTTP.

## Stack

- Python 3.11+
- `bleak` (BLE), `aiomqtt`, FastAPI, SQLite, APScheduler (or a simple scheduler loop)
- Mosquitto as the MQTT broker
- App: installable web app (PWA) served by the backend
- Hosting: one Raspberry Pi at home to start; backend may move to Railway later

## Proposed layout

```
protocol/   Scentiment BLE codec: pure functions, unit-tested
bridge/     BLE transport, MQTT client, local schedule cache
backend/    FastAPI app, scheduler, database models
app/        PWA frontend
tools/      scan.py, gatt_dump.py, send.py (CLI utilities)
docs/       findings.md, wifi.md
```

## MQTT topics (draft)

- `scent/<device_id>/desired`: retained, backend -> bridge
- `scent/<device_id>/state`: retained, bridge -> backend
- `scent/bridge/<bridge_id>/status`: online/offline via last-will message

## Phases

1. **Discovery and proof of control.** Write tools/scan.py and tools/gatt_dump.py,
   identify both models, port the Scentiment codec, and build a CLI that sets
   power, intensity and LED and reads battery and state.
   Done when both diffusers can be controlled from the command line.
2. **Bridge and backend.** MQTT, desired-state model, scheduler, API, database.
3. **App.** Device list, controls, schedule editor, battery and status display.
4. **WiFi investigation.** Check router or Pi-hole DNS logs for the hosts the
   diffusers contact and document the findings in docs/wifi.md. Build a WiFi
   transport only if it's feasible.

## Conventions

- Protocol code is pure (bytes/JSON in, structured data out) and tested against
  real captured messages saved as fixtures.
- Record every observed device behavior in docs/findings.md with a date.
- Device addresses and names go in a config file, never hardcoded.
- Never contact Scentiment's servers.

## Environment notes

- The development machine needs Bluetooth and must be within range of the
  diffusers. Run Claude Code on that machine or on the Pi.
- If range is a problem, an ESP32 running ESPHome's Bluetooth proxy can help.

## First task

Create tools/scan.py and tools/gatt_dump.py, run them near the diffusers, and
write the results (names, addresses, services, characteristics) to docs/findings.md.
