# Phase 0 tools

Command-line tools for checking the Scentiment diffusers over Bluetooth and MQTT. See
`docs/superpowers/plans/2026-10-20-phase-0-device-verification.md`.

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

## Usage

Usage for `scan.py`, `gatt_dump.py`, `session.py` and `broker.py` is added as each
tool lands.
