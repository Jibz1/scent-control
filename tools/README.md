# Phase 0 tools

Command-line tools for checking the Scentiment diffusers over Bluetooth and MQTT. See
`docs/superpowers/plans/2026-10-20-phase-0-device-verification.md`.

Everyday control has moved to the desktop app (`apps/desktop`). These tools and the web
test bench stay for protocol experiments and for checking device behaviour.

## Setup

```bash
cd tools
python3 -m venv .venv
.venv/bin/pip install -e '.[dev]'
.venv/bin/python -m pytest -q
```

If bleak's macOS dependency (pyobjc) fails to install on your Python, run
`brew install python@3.12` and use `python3.12` instead.

## Before you connect

- **Close the Scentiment app, or turn the phone's Bluetooth off.** A diffuser accepts
  only one Bluetooth connection at a time.
- The first scan triggers a macOS prompt asking to allow Bluetooth. Allow it.
- Copy `devices.example.toml` to `devices.toml` and fill in the addresses that
  `scan.py` prints. `devices.toml` stays out of git.

## Safety rules

- Never contact Scentiment's servers. Never use credentials found in Scentiment's app.
- The tools refuse `UPDATE_FIRMWARE`, `U_FW`, any `RESET` other than
  `{"mode":"soft"}`, any `SECRET_COMMAND` other than `{"command":"restart"}`, and any
  action not on the allow-list.
- Passwords are typed at a prompt, or read from `SCENT_BROKER_PASSWORD`. They are
  never taken as command-line arguments, and they are masked in every log.
- Raw output goes to `../captures/`, which stays out of git. Commit only sanitized
  fixtures under `../fixtures/scentiment/`.

## Running Bluetooth tools on macOS

macOS only lets a program use Bluetooth if the app responsible for it declares that
it does. A tool started from Terminal.app runs under Terminal's permission.
A tool started by Claude (or another app without that declaration) gets killed.

- **From Terminal.app:** run `.venv/bin/python <script> …` directly. macOS asks once
  to allow Terminal to use Bluetooth. Use this for `session.py wifi`, which prompts
  for the WiFi password.
- **Through `./bt`:** `./bt <script> …` runs the script inside `macos/ScentTools.app`,
  a minimal wrapper whose Info.plist declares Bluetooth use. macOS asks once to allow
  "ScentTools". macOS also stops that app from reading `~/Documents`, so `./bt` keeps
  a runtime copy of the tools in `~/Library/Application Support/ScentControl/tools-runtime`
  (synced on every run), runs the script there, and copies its captures back to
  `../captures/`. Files a command reads, such as a schedule file, must be inside that
  runtime copy: write them to its `captures/` folder. Output is shown when the
  command finishes. `./bt` has no keyboard
  input, so it can't run commands that prompt for a password. It also doesn't see
  your shell's environment variables, so it loads `tools/.env.local` (gitignored),
  e.g. `SCENT_BROKER_PASSWORD=…`.

## Usage

```bash
./bt scan.py --timeout 15                 # list nearby Scentiment diffusers (--all for everything)
./bt gatt_dump.py air2                    # services, characteristics, readable values; sends nothing
./bt session.py info air2                 # parsed 0xFEF4 device info
./bt session.py listen air2 --seconds 30  # print notifications
./bt session.py send air2 TURN_ON '{"on":1}'
./bt session.py sync-time air2
RT="$HOME/Library/Application Support/ScentControl/tools-runtime"
.venv/bin/python session.py make-test-schedule --start-in 3 --duration 4 > "$RT/captures/sched.json"
./bt session.py profile air2 captures/sched.json --timezone -4   # path relative to the runtime copy
./bt session.py restart air2              # soft restart only
./bt session.py save-mqtt air2 --broker wss://<your-broker-host>/mqtt --username <username>   # password from .env.local
.venv/bin/python session.py wifi air2     # from Terminal.app: prompts for the WiFi name and password
```

Every session writes `../captures/session-<alias>-<timestamp>.jsonl`, with secrets
masked.

### Test broker

`broker.py` needs no Bluetooth, so run it directly. It only connects over encrypted
`wss://` or `mqtts://`. The password comes from `SCENT_BROKER_PASSWORD`, from
`.env.local`, or from a prompt.

`--url` and `--username` default to `SCENT_BROKER_URL` and `SCENT_BROKER_USERNAME`,
taken from the environment, then the repo-root `.env` (copy `../.env.example`), then
`.env.local`. Give them on the command line to use another broker. The password never
goes in the repo-root `.env`.

```bash
.venv/bin/python broker.py watch
.venv/bin/python broker.py watch --url wss://<your-broker-host>/mqtt --username <username>
.venv/bin/python broker.py publish --topic scentiment-mqtt-prod/device/<deviceId> --json '{"action":"PING","payload":{"userid":"route-test-1"}}'
.venv/bin/python broker.py sync-time --topic scentiment-mqtt-prod/device/<deviceId>
.venv/bin/python broker.py publish-profile --topic scentiment-mqtt-prod/device/<deviceId> --file ../captures/sched.json --timezone -4
```

### Test bench (web page)

A local page for clicking through commands and watching every reply, before the
real app exists:

```bash
.venv/bin/python control_server.py   # the broker from SCENT_BROKER_URL and SCENT_BROKER_USERNAME, as above
.venv/bin/python control_server.py --url wss://<your-broker-host>/mqtt --username <username>
# open http://127.0.0.1:8765
```

`--route-id` defaults to `SCENT_ROUTE_ID` the same way, else `route-test-1`.

The helper listens only on 127.0.0.1 and holds the broker password, taken from
`.env.local` or a prompt. The page never sees the password. Every command goes
through the same allow-list as the command-line tools. It works over MQTT only, so a
diffuser must already be pointed at the test broker with `session.py save-mqtt`.

