# Phase 0: Device Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove on the owner's real Air 2 and HVAC 2 that the design works:
- Bluetooth control works.
- `SAVE_MQTT` over Bluetooth points a diffuser at our own broker.
- The diffuser runs schedules itself.

Then record everything observed, and decide at the gate whether Phase 1 goes ahead.

**Architecture:** This plan has two parts:
- **Python command-line tools** under `tools/`, built on `bleak` for Bluetooth and `paho-mqtt` for the broker. The pure helpers are unit-tested; the scripts that touch devices are checked against the real devices.
- **A throwaway Mosquitto test broker** on Railway, reachable over `wss://` with Railway's public certificate.

Device experiments are run together with the owner, who has to be physically near the diffusers. Every result goes into `docs/findings.md` as **[observed]** with a date. Sanitized captures become fixtures for Phase 1.

**Tech Stack:** Python ≥ 3.11 (3.14.2 on this Mac), `bleak` 1.x, `paho-mqtt` 2.x, `pytest` 8.x. Mosquitto 2.x (Docker) on Railway. Railway CLI 4.29.

**Spec:** `docs/superpowers/specs/2026-10-20-scent-control-design.md` (§7 Phase 0, plus §4 anti-goals). Device facts: `docs/findings.md`.

## Global Constraints

- Never contact Scentiment's servers. Never use, store or print credentials found in Scentiment's app.
- Never send `UPDATE_FIRMWARE`, `U_FW`, `RESET` other than `{"mode":"soft"}`, or `SECRET_COMMAND` other than `{"command":"restart"}`.
- WiFi passwords and broker passwords never go in the repo, logs, captures, fixtures or command-line arguments. They are typed at a `getpass` prompt or read from an environment variable.
- BLE: service `00000180-0000-1000-8000-00805f9b34fb`, write `0000dead-0000-1000-8000-00805f9b34fb`, notify `0000fef3-0000-1000-8000-00805f9b34fb`, info `0000fef4-0000-1000-8000-00805f9b34fb`.
- Frame: compact JSON `{"action":…,"payload":…}` (payload left out when `None`), then the bytes `-|`. Written with response, in chunks of `MTU − 3` bytes (20 if the MTU is unknown or ≤ 23).
- MQTT topics: the device subscribes to `scentiment-mqtt-prod/device/<deviceId>` and publishes to `scentiment-mqtt-prod/user/<userId>`. QoS 1, not retained.
- Weekdays in schedules: 0 = Sunday … 6 = Saturday. Times are `HH:mm`. `timezone` is the UTC offset in hours as a decimal.
- Device addresses live in `tools/devices.toml` (gitignored) and are never hardcoded. `tools/devices.example.toml` is committed.
- Raw output goes to `captures/` (gitignored). Only sanitized fixtures are committed, under `fixtures/scentiment/`.
- Test broker:
  - It uses `wss://` with a public certificate.
  - A plain `mqtt://` device connection, or moving off Railway, means stopping and asking the owner.
  - Never delete the test-broker service while a diffuser points at it.
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Secrets in logs.** A WiFi or broker password typed during `wifi` or `save-mqtt` must never appear in stdout, `captures/`, fixtures or shell history. Pinned by `test_redact_masks_passwords`, `test_capture_log_never_writes_passwords` and `test_cli_has_no_password_options`.
2. **Commands longer than one BLE packet** (`SAVE_MQTT` with a long URL) must go out as ordered chunks, each ≤ MTU − 3 bytes, that rejoin to the identical frame. Pinned by `test_chunk_*`.
3. **Dangerous or mistyped actions** typed by hand must be refused before anything is sent, over BLE and over MQTT. Pinned by `test_check_allowed_*` and `test_broker_publish_refuses_blocked_action`.
4. **Odd notifications** (a trailing `-|`, whitespace, invalid UTF-8, a short `data` array, `Bat:200`, unknown keys) must parse without crashing and keep the raw text. Pinned by `test_parse_notification_*`.
5. **Unknown device alias, a broken `devices.toml`, or a `0xFEF4` string with missing fields** must produce an error naming the problem, with no connection attempted. Pinned by `test_load_devices_*` and `test_parse_info_rejects_short`.

---

### Task 1: Rewrite CLAUDE.md for the approved design

**Files:**
- Modify: `CLAUDE.md` (full rewrite)

**Interfaces:**
- Consumes: the spec, `docs/findings.md`
- Produces: agent instructions every later task and session reads

- [ ] **Step 1: Rewrite `CLAUDE.md`.** Keep the current headings' spirit and replace the content with:
  - **Purpose:** the spec's §1, in two sentences.
  - **Architecture:** the spec's §6.1 diagram and the parts list.
  - **Stack:**
    - product code: Bun 1.4 + Turborepo + Effect v4 + Hono + Drizzle/Postgres + Expo 57 on Railway, following HCCNA's conventions;
    - Phase 0 tools: Python + bleak under `tools/`.
  - **Layout:** `apps/mobile`, `apps/server`, `packages/<protocol>`, `infra/`, `tools/`, `fixtures/scentiment/`, `docs/`.
  - **Rules:** the Global Constraints above, plus the conventions currently in CLAUDE.md that still hold:
    - protocol code is pure and fixture-tested;
    - findings are dated;
    - device addresses go in config files;
    - never contact Scentiment.
  - **Phases:** the spec's §7, with Phase 0 as the current phase.
  - **Pointers:** to the spec and to `docs/findings.md`.

  Remove the PWA, FastAPI/SQLite and "the app never uses Bluetooth" statements, and the open questions that `docs/findings.md` now answers.

- [ ] **Step 2: Verify that no stale statements remain.**

  Run: `grep -n -i -E 'PWA|FastAPI|SQLite|APScheduler|never uses Bluetooth' CLAUDE.md`
  Expected: no output.

- [ ] **Step 3: Commit.**

```bash
git add CLAUDE.md
git commit -m "Rewrite CLAUDE.md for the approved Scent Control design

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Tools setup and the pure Scentiment codec

**Files:**
- Create: `tools/pyproject.toml`, `tools/README.md`, `tools/scentiment_ble.py`, `tools/tests/test_scentiment_ble.py`
- Modify: `.gitignore` (add `tools/devices.toml` and `*.egg-info/`)

**Interfaces:**
- Produces, in `tools/scentiment_ble.py`:
  - `SERVICE_UUID`, `WRITE_UUID`, `NOTIFY_UUID`, `INFO_UUID: str`. `TERMINATOR = b"-|"`.
  - `encode_command(action: str, payload: dict | None = None) -> bytes`
  - `chunk(frame: bytes, mtu: int | None) -> list[bytes]`
  - `@dataclass(frozen=True) DeviceInfo(manufacturer_id: str, model_name: str, variant_name: str, device_id: str, firmware_version: str, wifi_configured: bool, wifi_status: int)`
  - `parse_info(text: str) -> DeviceInfo`. Raises `ValueError` when there are fewer than 7 fields.
  - `@dataclass(frozen=True) NameInfo(model_code: str | None, model: str | None, variant_code: str | None, variant: str | None)`
  - `is_scentiment_name(name: str | None) -> bool`
  - `parse_name(name: str) -> NameInfo`
  - `parse_notification(data: bytes) -> dict[str, object]`. Always includes `"raw"` and `"kind"`. Never raises.
  - `build_profile(entries: list[dict], timezone_hours: float, schedule_flag: int) -> dict`. Returns the `UPDATE_PROFILE` payload.
  - `scentiment_weekday(day: datetime.date) -> int`
  - `trial_schedule_entry(now: datetime.datetime, start_in_min: int, duration_min: int, level: int) -> dict`
  - `redact_frame(frame: bytes) -> str`

- [ ] **Step 1: Set up the project.** `tools/pyproject.toml`:
  - project `scent-tools`, `requires-python = ">=3.11"`;
  - dependencies `bleak>=1.0,<2` and `paho-mqtt>=2.1,<3`;
  - optional `dev = ["pytest>=8,<9"]`;
  - `[tool.pytest.ini_options] testpaths = ["tests"]`, `pythonpath = ["."]`.

  Create the environment and install:

  Run: `cd tools && python3 -m venv .venv && .venv/bin/pip install -e '.[dev]'`
  Expected: installs bleak, paho-mqtt and pytest without errors. If bleak's macOS dependency (pyobjc) fails on Python 3.14, run `brew install python@3.12` and repeat with `python3.12`.

- [ ] **Step 2: Write the failing tests** in `tools/tests/test_scentiment_ble.py`:

```python
import datetime as dt, json
import pytest
from scentiment_ble import *

def test_encode_command_compact_with_terminator():
    assert encode_command("SET_LEVEL", {"intensity": 2}) == b'{"action":"SET_LEVEL","payload":{"intensity":2}}-|'
    assert encode_command("PING") == b'{"action":"PING"}-|'

def test_chunk_splits_at_mtu_minus_3_and_rejoins():
    frame = encode_command("SAVE_MQTT", {"broker": "wss://" + "a" * 150 + "/mqtt", "username": "u", "password": "p"})
    parts = chunk(frame, 180)
    assert all(len(p) <= 177 for p in parts) and b"".join(parts) == frame and len(parts) == 2

def test_chunk_falls_back_to_20_bytes():
    frame = encode_command("TURN_ON", {"on": 1})
    for mtu in (None, 0, 23):
        parts = chunk(frame, mtu)
        assert all(len(p) <= 20 for p in parts) and b"".join(parts) == frame

def test_parse_info_seven_fields():
    info = parse_info("1|Diffuser Air 2|Black|TESTDEVICE01|2.3.1|true|3")
    assert info == DeviceInfo("1", "Diffuser Air 2", "Black", "TESTDEVICE01", "2.3.1", True, 3)

def test_parse_info_rejects_short():
    with pytest.raises(ValueError, match="7"):
        parse_info("1|Diffuser Air 2|Black")

def test_name_parsing():
    assert is_scentiment_name("Scentiment-D B 1A2B") and is_scentiment_name("SCENTIMENT-H W")
    assert not is_scentiment_name(None) and not is_scentiment_name("Scent K.A5.WIFI")
    assert parse_name("Scentiment-D B 1A2B") == NameInfo("D", "Diffuser Air 2", "B", "Black")
    assert parse_name("Scentiment-H W") == NameInfo("H", "Hvac", "W", "White")
    assert parse_name("Scentiment-Z Q") == NameInfo("Z", None, "Q", None)
    assert parse_name("Scentiment") == NameInfo(None, None, None, None)

def test_parse_notification_status_text():
    n = parse_notification(b"S:1,L:3,C:2,Bat:200-|")
    assert n["kind"] == "status_text" and (n["power"], n["intensity"], n["charging"], n["battery"]) == (1, 3, 2, 200)

def test_parse_notification_status_array_from_scent_assistant_issue_42():
    n = parse_notification(b'{"data":[85,170,11,255,0,2,200,100,255,0,0,255,255,0,48,106]}')
    assert n["kind"] == "status_array"
    assert {k: n[k] for k in ("power", "charging", "battery", "oil", "fan", "intensity", "hours", "gas_pump", "light", "factory_restore")} == \
        {"power": 0, "charging": 2, "battery": 200, "oil": 100, "fan": 255, "intensity": 0, "hours": 0, "gas_pump": 255, "light": 255, "factory_restore": 0}

def test_parse_notification_short_array_and_values():
    n = parse_notification(b'{"data":[85,170,11,255,1,0,90]}')
    assert (n["power"], n["charging"], n["battery"]) == (1, 0, 90) and "oil" not in n
    assert parse_notification(b"enable:1")["power"] == 1
    assert parse_notification(b"level:2")["intensity"] == 2
    assert parse_notification(b" battery:80 ")["battery"] == 80

def test_parse_notification_never_raises():
    for data in (b"\xff\xfe", b"", b"hello", b"{not json", b'{"other":1}'):
        n = parse_notification(data)
        assert "raw" in n and n["kind"] in {"unknown", "json", "value"}

def test_build_profile_shape():
    entry = {"startTime": "07:00", "endTime": "09:00", "weekdays": [1], "workingState": {"level": 3, "fan": 0, "led": 0, "timer": 0}, "enable": True}
    assert build_profile([entry], -4.0, 1) == {"profile": {"schedule": [entry]}, "timezone": -4.0, "schedule_flag": 1}

def test_scentiment_weekday_sunday_is_zero():
    assert scentiment_weekday(dt.date(2026, 10, 18)) == 0
    assert scentiment_weekday(dt.date(2026, 10, 19)) == 1
    assert scentiment_weekday(dt.date(2026, 10, 24)) == 6

def test_trial_schedule_entry():
    e = trial_schedule_entry(dt.datetime(2026, 10, 19, 14, 58), 3, 4, 3)
    assert e == {"startTime": "15:01", "endTime": "15:05", "weekdays": [1], "workingState": {"level": 3, "fan": 0, "led": 0, "timer": 0}, "enable": True}
    with pytest.raises(ValueError, match="midnight"):
        trial_schedule_entry(dt.datetime(2026, 10, 19, 23, 58), 3, 4, 3)

def test_redact_masks_passwords():
    out = redact_frame(encode_command("WIFI_CONNECT", {"ssid": "ExampleWiFi", "password": "hunter2!"}))
    assert "hunter2!" not in out and '"password":"***"' in out and "ExampleWiFi" in out
    out = redact_frame(encode_command("SAVE_MQTT", {"broker": "wss://b.example.test/mqtt", "username": "dev1", "password": "s3cret"}))
    assert "s3cret" not in out
    assert redact_frame(b"\x00garbage") == "<unparsable 8 bytes>"
```

- [ ] **Step 3: Run the tests to verify they fail.**

  Run: `cd tools && .venv/bin/python -m pytest tests/test_scentiment_ble.py -q`
  Expected: errors with `ModuleNotFoundError: No module named 'scentiment_ble'`.

- [ ] **Step 4: Implement `tools/scentiment_ble.py`.** The tests fix the signatures and values. A few rules they don't spell out:

  `parse_notification`:
  - decode with `errors="replace"`;
  - strip whitespace and a trailing `-|`;
  - then try, in order:
    1. JSON with a `data` list: map indices 4–13 to `power, charging, battery, oil, fan, intensity, hours, gas_pump, light, factory_restore`, include only the indices that exist, and keep `array`;
    2. other JSON: `kind="json"`;
    3. a comma list with `S/L/C/Bat`;
    4. a single `key:value` (`enable`→`power`, `level`→`intensity`, `battery`; others are kept under their own key, all with `kind="value"`);
    5. anything else: `kind="unknown"`.

  `redact_frame` replaces the value of any payload key in `{"password", "pass", "token", "secret"}` with `"***"`.

- [ ] **Step 5: Run the tests to verify they pass.**

  Run: `cd tools && .venv/bin/python -m pytest tests/test_scentiment_ble.py -q`
  Expected: `14 passed`.

- [ ] **Step 6: Write `tools/README.md`** covering:
  - setup (Step 1's commands);
  - the safety rules from Global Constraints;
  - "close the Scentiment app or turn the phone's Bluetooth off first";
  - the first-run macOS Bluetooth permission prompt;
  - each script's usage (filled in by Tasks 4 and 5).

- [ ] **Step 7: Commit.**

```bash
git add .gitignore tools/pyproject.toml tools/README.md tools/scentiment_ble.py tools/tests/test_scentiment_ble.py
git commit -m "Add Phase 0 tools setup and pure Scentiment codec with tests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Safety rules, capture log and device config

**Files:**
- Create: `tools/safety.py`, `tools/capture.py`, `tools/devices.py`, `tools/devices.example.toml`, `tools/tests/test_safety_capture_devices.py`

**Interfaces:**
- Consumes: `encode_command`, `redact_frame` (Task 2)
- Produces:
  - `safety.RefusedCommand(Exception)`
  - `safety.check_allowed(action: str, payload: dict | None) -> None`. Raises `RefusedCommand`.
  - `capture.CaptureLog(path: pathlib.Path)`, with:
    - `.tx(frame: bytes, channel: str) -> None`
    - `.rx(text: str, parsed: dict, channel: str) -> None`
    - `.note(message: str) -> None`

    It writes JSONL records `{"ts": ISO-8601, "dir": "tx"|"rx"|"note", "channel": str, "text": str, "parsed"?: dict}` and creates parent directories.
  - `devices.DeviceEntry(alias: str, address: str, name: str | None)`
  - `devices.load_devices(path: pathlib.Path) -> dict[str, DeviceEntry]`
  - `devices.resolve(alias_or_address: str, path: pathlib.Path) -> DeviceEntry`
  - `devices.DEFAULT_CONFIG = Path(__file__).parent / "devices.toml"`

- [ ] **Step 1: Write the failing tests** in `tools/tests/test_safety_capture_devices.py`:

```python
import json
import pytest
from scentiment_ble import encode_command
from safety import check_allowed, RefusedCommand
from capture import CaptureLog
from devices import load_devices, resolve

ALLOWED = [("TURN_ON", {"on": 1}), ("SET_LEVEL", {"intensity": 3}), ("SET_RGB_LEVEL", {"red": 1, "green": 2, "blue": 3}),
           ("SET_RGB_LED", {"on": 0}), ("PING", None), ("PING", {"userid": "route-test-1"}), ("WIFI_CONNECT", {"ssid": "x", "password": "y"}),
           ("SAVE_MQTT", {"broker": "wss://b.example.test/mqtt", "username": "u", "password": "p"}), ("SYNC_TIME", {"serverTime": 1}),
           ("UPDATE_PROFILE", {"profile": {"schedule": []}, "timezone": -4.0, "schedule_flag": 1}), ("INITIAL_LOAD", {"profile": {"schedule": []}}),
           ("GET_STATUS", {}), ("RESET", {"mode": "soft"}), ("SECRET_COMMAND", {"command": "restart"})]

@pytest.mark.parametrize("action,payload", ALLOWED)
def test_check_allowed_accepts_documented_safe_commands(action, payload):
    check_allowed(action, payload)

@pytest.mark.parametrize("action,payload", [("UPDATE_FIRMWARE", {"url": "https://x"}), ("U_FW", None), ("RESET", {"mode": "hard"}),
    ("RESET", {"type": "FACTORY"}), ("SECRET_COMMAND", {"command": "anything-else"}), ("SET_SCHEDULE", {}), ("TURN_ONN", {"on": 1})])
def test_check_allowed_refuses_dangerous_or_unknown(action, payload):
    with pytest.raises(RefusedCommand):
        check_allowed(action, payload)

def test_capture_log_never_writes_passwords(tmp_path):
    log = CaptureLog(tmp_path / "c" / "s.jsonl")
    log.tx(encode_command("WIFI_CONNECT", {"ssid": "ExampleWiFi", "password": "hunter2!"}), "ble")
    log.tx(encode_command("SAVE_MQTT", {"broker": "wss://b.example.test/mqtt", "username": "u", "password": "s3cret"}), "ble")
    text = (tmp_path / "c" / "s.jsonl").read_text()
    assert "hunter2!" not in text and "s3cret" not in text
    assert [json.loads(l)["dir"] for l in text.splitlines()] == ["tx", "tx"]

def test_load_devices_and_resolve(tmp_path):
    cfg = tmp_path / "devices.toml"
    cfg.write_text('[devices.air2]\naddress = "AAAA-1111"\nname = "Scentiment-D B"\n')
    assert load_devices(cfg)["air2"].address == "AAAA-1111"
    assert resolve("air2", cfg).address == "AAAA-1111"
    assert resolve("BBBB-2222", cfg).address == "BBBB-2222"   # a raw address passes through

def test_load_devices_unknown_alias_names_known_ones(tmp_path):
    cfg = tmp_path / "devices.toml"
    cfg.write_text('[devices.air2]\naddress = "AAAA-1111"\n')
    with pytest.raises(KeyError, match="air2"):
        resolve("hvac", cfg)

def test_load_devices_missing_or_broken_file(tmp_path):
    with pytest.raises(FileNotFoundError, match="devices.example.toml"):
        load_devices(tmp_path / "nope.toml")
    bad = tmp_path / "bad.toml"; bad.write_text("[devices.air2]\nname = 1\n")
    with pytest.raises(ValueError, match="address"):
        load_devices(bad)
```

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cd tools && .venv/bin/python -m pytest tests/test_safety_capture_devices.py -q`
  Expected: errors with `ModuleNotFoundError`.

- [ ] **Step 3: Implement the three modules.**
  - `check_allowed` is an allow-list of the actions in `ALLOWED`. `RESET` must be exactly `{"mode":"soft"}`, and `SECRET_COMMAND` exactly `{"command":"restart"}`.
  - `CaptureLog.tx` always stores `redact_frame(frame)`.
  - `resolve`: a value containing `-` or `:` that isn't an alias is treated as a raw address. An unknown alias raises `KeyError` listing the known aliases.
  - `load_devices` uses `tomllib`. A missing file raises `FileNotFoundError` telling the user to copy `devices.example.toml`.

  `devices.example.toml` holds `air2` and `hvac` entries with placeholder addresses and a comment that addresses are macOS CoreBluetooth UUIDs printed by `scan.py`.

- [ ] **Step 4: Run all tool tests.**

  Run: `cd tools && .venv/bin/python -m pytest -q`
  Expected: `39 passed` (14 + 25).

- [ ] **Step 5: Commit.**

```bash
git add tools/safety.py tools/capture.py tools/devices.py tools/devices.example.toml tools/tests/test_safety_capture_devices.py
git commit -m "Add command allow-list, redacting capture log and device config

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Bluetooth tools: scan, GATT dump and session

**Files:**
- Create: `tools/ble_client.py`, `tools/scan.py`, `tools/gatt_dump.py`, `tools/session.py`, `tools/tests/test_session_cli.py`
- Modify: `tools/README.md` (usage)

**Interfaces:**
- Consumes: Task 2's codec, and Task 3's `check_allowed`, `CaptureLog` and `resolve`
- Produces:
  - `ble_client.DiffuserLink(entry: DeviceEntry, log: CaptureLog)`, an async context manager that connects on enter and disconnects on exit, with:
    - `async read_info() -> DeviceInfo`
    - `async send(action: str, payload: dict | None = None) -> None`. Calls `check_allowed`, encodes, chunks with `client.mtu_size`, writes each chunk to `WRITE_UUID` with `response=True` in order, and logs `tx`.
    - `async listen(seconds: float, on_message: Callable[[dict], None] | None = None) -> list[dict]`. Uses `start_notify(NOTIFY_UUID)`, then `parse_notification`, logs `rx`, prints a one-line summary, and returns the parsed messages.
  - `session.build_parser() -> argparse.ArgumentParser`
  - `session.main(argv: list[str] | None = None) -> int`

  CLI contract:
  - `scan.py [--timeout 10] [--all]`: lists name, address, RSSI, `parse_name` result, service UUIDs and manufacturer data in hex. Filters with `is_scentiment_name` unless `--all` is given. Writes `captures/scan-<ts>.json`.
  - `gatt_dump.py <alias|address>`: lists services, characteristics with properties, and descriptors. Reads every readable characteristic (hex plus UTF-8) and parses `0xFEF4` with `parse_info`. Writes `captures/gatt-<alias>-<ts>.json`.
  - `session.py`, which logs to `captures/session-<alias>-<ts>.jsonl`, with these subcommands:
    - `info <alias>`
    - `listen <alias> [--seconds 30]`
    - `send <alias> <ACTION> [PAYLOAD_JSON] [--listen 10]`
    - `wifi <alias>`: SSID via `input`, password via `getpass`
    - `save-mqtt <alias> --broker URL --username USER`: password via `getpass`, or the `SCENT_BROKER_PASSWORD` environment variable
    - `sync-time <alias>`
    - `profile <alias> <FILE.json> --timezone HOURS [--flag 1]`
    - `make-test-schedule --start-in 3 --duration 4 --level 3 [--factory FILE.json]`: prints a profile file with the factory entry (from `--factory`, or the default `{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":2,"fan":0,"led":0,"timer":0},"enable":true,"isFactory":true}`) plus `trial_schedule_entry(now, …)`. It prints `{"profile":{"schedule":[factory, entry]}}`. `profile` (and `broker.py publish-profile`) read `profile.schedule` from such a file and pass it to `build_profile(entries, timezone, flag)`
    - `restart <alias>`: `SECRET_COMMAND {"command":"restart"}`

- [ ] **Step 1: Write the failing CLI tests** in `tools/tests/test_session_cli.py`:

```python
import json
from session import build_parser, main

def test_cli_has_no_password_options():
    opts = [o for a in build_parser()._subparsers._group_actions[0].choices.values() for act in a._actions for o in act.option_strings]
    assert not [o for o in opts if "pass" in o.lower() or "secret" in o.lower()]

def test_send_refuses_blocked_action_before_connecting(capsys):
    assert main(["send", "does-not-matter", "UPDATE_FIRMWARE", '{"url":"https://x"}']) == 2
    assert "refused" in capsys.readouterr().err.lower()

def test_make_test_schedule_prints_profile(capsys):
    assert main(["make-test-schedule", "--start-in", "3", "--duration", "4", "--level", "3"]) == 0
    profile = json.loads(capsys.readouterr().out)
    entries = profile["profile"]["schedule"]
    assert entries[0]["isFactory"] is True and entries[1]["workingState"]["level"] == 3
```

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cd tools && .venv/bin/python -m pytest tests/test_session_cli.py -q`
  Expected: errors with `ModuleNotFoundError: No module named 'session'`.

- [ ] **Step 3: Implement `ble_client.py`, `scan.py`, `gatt_dump.py` and `session.py`** to the contract above.
  - `send` checks `check_allowed` before resolving the device or connecting, and returns 2 with `refused: <reason>` on stderr.
  - Connection errors print a hint that the Scentiment app or a phone may be holding the connection.

- [ ] **Step 4: Run all tool tests.**

  Run: `cd tools && .venv/bin/python -m pytest -q`
  Expected: `42 passed`.

- [ ] **Step 5: Smoke-test with the owner nearby** (no command sent).

  Run: `cd tools && .venv/bin/python scan.py --timeout 15`
  Expected: one or more `Scentiment…` rows with an address and a parsed model. Approve the macOS Bluetooth prompt on the first run. If no rows appear, run with `--all`, and check the diffusers are powered on and the phone's Bluetooth is off.

- [ ] **Step 6: Commit.**

```bash
git add tools/ble_client.py tools/scan.py tools/gatt_dump.py tools/session.py tools/tests/test_session_cli.py tools/README.md
git commit -m "Add Bluetooth scan, GATT dump and session tools for Phase 0

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Test broker on Railway, and the broker tool

**Files:**
- Create: `infra/test-broker/Dockerfile`, `infra/test-broker/mosquitto.conf`, `infra/test-broker/entrypoint.sh`, `infra/test-broker/README.md`, `tools/broker.py`, `tools/tests/test_broker.py`

**Interfaces:**
- Consumes: `check_allowed`, `CaptureLog`, `parse_notification`
- Produces:
  - `broker.BrokerTarget(host: str, port: int, path: str, tls: bool, transport: str)`
  - `broker.parse_broker_url(url: str) -> BrokerTarget`. `wss://` defaults to port 443 and path `/mqtt`. `mqtts://` defaults to 8883 over TCP. Anything else raises `ValueError`.
  - `broker.py watch --url URL --username U [--seconds N]`: subscribes to `#` and prints `topic payload`, parsing the inner `data` with `parse_notification`. Logs to `captures/broker-<ts>.jsonl`.
  - `broker.py publish --url URL --username U --topic T --json PAYLOAD`: the payload is the full `{"action","payload"}` envelope, checked with `check_allowed` before connecting. QoS 1.
  - `broker.py publish-profile --url URL --username U --topic T --file F.json --timezone HOURS [--flag 1]`: publishes `UPDATE_PROFILE` with `build_profile(...)`.
  - `broker.py sync-time --url URL --username U --topic T`: publishes `SYNC_TIME {"serverTime": <now, UTC epoch seconds>}`.
  - Both take the password from `getpass` or `SCENT_BROKER_PASSWORD`.
  - The deployed service `test-broker` in the Railway project `scent-control`, with domain `<generated>.up.railway.app` and target port 8080.

- [ ] **Step 1: Write the failing tests** in `tools/tests/test_broker.py`:

```python
import pytest
from broker import parse_broker_url, BrokerTarget, main

def test_parse_broker_url():
    assert parse_broker_url("wss://b.example.test/mqtt") == BrokerTarget("b.example.test", 443, "/mqtt", True, "websockets")
    assert parse_broker_url("wss://b.example.test:8084/mqtt") == BrokerTarget("b.example.test", 8084, "/mqtt", True, "websockets")
    assert parse_broker_url("mqtts://b.example.test") == BrokerTarget("b.example.test", 8883, "", True, "tcp")
    with pytest.raises(ValueError):
        parse_broker_url("mqtt://b.example.test")

def test_broker_publish_refuses_blocked_action(capsys):
    assert main(["publish", "--url", "wss://b.example.test/mqtt", "--username", "u", "--topic", "t",
                 "--json", '{"action":"UPDATE_FIRMWARE","payload":{"url":"https://x"}}']) == 2
    assert "refused" in capsys.readouterr().err.lower()
```

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cd tools && .venv/bin/python -m pytest tests/test_broker.py -q`
  Expected: errors with `ModuleNotFoundError: No module named 'broker'`.

- [ ] **Step 3: Implement `tools/broker.py`** with paho-mqtt 2.x: `transport="websockets"`, `ws_set_options(path=…)` and `tls_set()` (system CA bundle) for `wss`.

- [ ] **Step 4: Run all tool tests.**

  Run: `cd tools && .venv/bin/python -m pytest -q`
  Expected: `44 passed`.

- [ ] **Step 5: Write the broker files.**
  - **`Dockerfile`:** `FROM eclipse-mosquitto:<exact 2.x tag at least two weeks old>`. Copy the config and entrypoint, then set `ENTRYPOINT ["/entrypoint.sh"]`.
  - **`mosquitto.conf`:**

```
per_listener_settings false
listener 8080
protocol websockets
allow_anonymous false
password_file /mosquitto/config/passwd
persistence false
log_dest stdout
log_type all
connection_messages true
```

  - **`entrypoint.sh`:** exit with an error if `TEST_BROKER_USERNAME` or `TEST_BROKER_PASSWORD` is unset. Otherwise run `mosquitto_passwd -c -b /mosquitto/config/passwd "$TEST_BROKER_USERNAME" "$TEST_BROKER_PASSWORD"` and then `exec mosquitto -c /mosquitto/config/mosquitto.conf`.
  - **`README.md`:**
    - it's a throwaway test broker;
    - one shared login; every diffuser in Phase 0 uses it;
    - never delete it while a diffuser points at it;
    - how to deploy;
    - how to watch.

- [ ] **Step 6: Choose the workspace with the owner (owner).** The owner is creating a new Pro workspace for this project. Ask for its name, then confirm it with `railway list` / the `list-workspaces` tool. If it doesn't exist yet, ask whether to use the owner's personal workspace for now. Railway can transfer projects between workspaces later.

- [ ] **Step 7: Deploy.** Steps:
  1. Generate a password with `python3 -c "import secrets; print(secrets.token_urlsafe(24))"`, without echoing it into any committed file. The owner keeps it in their password manager.
  2. Create the project `scent-control` in the chosen workspace and add the service `test-broker`.
  3. Set the variables `TEST_BROKER_USERNAME=scent-test` and `TEST_BROKER_PASSWORD`.
  4. Deploy with `railway up infra/test-broker --path-as-root --service test-broker` (or the Railway MCP tools).
  5. Generate a domain with target port 8080.

  Expected: the deployment succeeds, and the logs show `mosquitto version 2.… running`.

- [ ] **Step 8: Verify a round trip over `wss`.** In one terminal, run `SCENT_BROKER_PASSWORD=… .venv/bin/python broker.py watch --url wss://<domain>/mqtt --username scent-test --seconds 60`. In another, publish `{"action":"PING"}` to `scentiment-mqtt-prod/device/TESTDEVICE01`.

  Expected: the watcher prints the topic and payload. The Railway logs show both clients connecting with username `scent-test`.

- [ ] **Step 9: Commit.**

```bash
git add infra/test-broker tools/broker.py tools/tests/test_broker.py tools/README.md
git commit -m "Add throwaway wss test broker for Railway and broker watch/publish tool

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Read-only survey of both diffusers

Nothing in this task changes either device.

**Files:**
- Create: `tools/devices.toml` (local, gitignored), `fixtures/scentiment/README.md`, `fixtures/scentiment/*.json`
- Modify: `docs/findings.md`

**Interfaces:**
- Consumes: `scan.py`, `gatt_dump.py`, `session.py info|listen`
- Produces: the fixture format `{"source": "observed YYYY-MM-DD", "device": "air2"|"hvac", "firmware": str, "channel": "ble-notify"|"ble-info"|"mqtt", "text": str}`, with sanitized values: `deviceId` → `TESTDEVICE01` / `TESTDEVICE02`, SSID → `ExampleWiFi`, broker host → `broker.example.test`

- [ ] **Step 1: Prepare (owner).** The diffusers are powered on, the phone's Bluetooth is off, and the Mac is within a few meters.
- [ ] **Step 2: Scan and map the aliases.** Run `scan.py --timeout 20`. Copy `devices.example.toml` to `devices.toml` and fill in the `air2` and `hvac` addresses.
- [ ] **Step 3: Dump the GATT tables.** Run `gatt_dump.py air2` and `gatt_dump.py hvac`.

  Expected: both show service `0x0180` with `0xDEAD`/`0xFEF3`/`0xFEF4`. If the HVAC doesn't, note its whole GATT table and go straight to Task 8, Step 5.
- [ ] **Step 4: Read info and listen.** For each device, run `session.py info <alias>` and `session.py listen <alias> --seconds 30`. Record:
  - model code, variant and firmware version;
  - `deviceId` format and WiFi status;
  - the notifications sent on connect.
- [ ] **Step 5: Record and save fixtures.** Add a dated **[observed]** section to `docs/findings.md`. Copy one sanitized fixture per message shape seen into `fixtures/scentiment/`, and describe the format in `fixtures/scentiment/README.md`.
- [ ] **Step 6: Commit.**

```bash
git add docs/findings.md fixtures/scentiment
git commit -m "Record read-only Bluetooth survey of the Air 2 and HVAC 2

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Air 2 experiments (gate evidence)

Run with the owner. Keep `broker.py watch` running throughout Steps 3–8. Each step's result goes into the Phase 0 findings section as **[observed]**.

**Files:**
- Modify: `docs/findings.md`, `fixtures/scentiment/*.json`

**Interfaces:**
- Consumes: Tasks 4–6. The `deviceId` comes from Task 6.

- [ ] **Step 1: Bluetooth control.** Send these, and have the owner confirm what the diffuser visibly does:
  - `TURN_ON {"on":1}`
  - `SET_LEVEL` 1, 2 and 3
  - `SET_RGB_LEVEL {"red":255,"green":0,"blue":0}`
  - `SET_RGB_LED {"on":1}`, then `{"on":0}`
  - `TURN_ON {"on":0}`

  Record which commands work and which notifications follow each one.
- [ ] **Step 2: Bluetooth schedule and time** (evidence for fallback (a)). Send `sync-time`, then a `make-test-schedule --start-in 3 --duration 4` profile with `profile air2 … --timezone <Mac's current UTC offset>`. Record whether the device acknowledges and whether the entry runs.
- [ ] **Step 3: `SAVE_MQTT` before `WIFI_CONNECT`.** Steps:
  1. `save-mqtt air2 --broker wss://<domain>/mqtt --username scent-test`
  2. `wifi air2` (the owner types the WiFi name and password)
  3. `restart air2`
  4. Poll `info air2` every 15 s for 2 minutes.

  Expected: a client logs in as `scent-test` in the Railway logs within 90 s, and subscribes to `scentiment-mqtt-prod/device/<deviceId>`. Record:
  - the client ID;
  - the subscriptions;
  - the first publishes;
  - the WiFi status progression.
- [ ] **Step 4: If Step 3 didn't connect, try in order** and stop at the first that works:
  1. `WIFI_CONNECT` first, then `SAVE_MQTT`, then restart. Record that this order opens the short window toward Scentiment's broker (spec §5).
  2. URL `wss://<domain>:443/mqtt`.
  3. URL `wss://<domain>/`.

  If WiFi connects (status 2/3) but no broker login ever appears, **stop: this is the gate in spec §7.** Go to Task 9 with the evidence.
- [ ] **Step 5: `PING` and routing.** Publish `{"action":"PING","payload":{"userid":"route-test-1"}}` to `scentiment-mqtt-prod/device/<deviceId>`.

  Expected: an `acknowledgement` on `scentiment-mqtt-prod/user/route-test-1`. Then `GET_STATUS {}`, and record the `update_status` array.
- [ ] **Step 6: Control over MQTT.** Publish `TURN_ON {"on":1}` and `SET_LEVEL {"intensity":3}` directly. Then try the app's way: `UPDATE_PROFILE` with only the factory entry, `enable:false`, `schedule_flag:0`. Record which ones change the device.
- [ ] **Step 7: A schedule with the phone away.** Publish `SYNC_TIME` and a `make-test-schedule` profile (`schedule_flag:1`) over MQTT. The Mac disconnects from Bluetooth, and the owner confirms the diffuser starts and stops at the scheduled minutes. Record what the device publishes.
- [ ] **Step 8: Restart and power cut** (input for spec §6.6). Steps:
  1. With a schedule entry spanning the next 15 minutes, the owner presses the power button to turn the diffuser off by hand. Record what the device publishes, if anything.
  2. The owner unplugs it for 60 s and plugs it back in. Don't send `SYNC_TIME` or `PING`.
  3. Record whether it reconnects, whether it publishes to `user/route-test-1` without a new `PING`, and whether its power, intensity, schedule and clock survive (does the entry still run?).
- [ ] **Step 9: Oil level.** Record index 7 from `update_status`. The owner records the visible oil level in the bottle.
- [ ] **Step 10: Save sanitized fixtures and commit.**

```bash
git add docs/findings.md fixtures/scentiment
git commit -m "Record Air 2 Phase 0 experiments: BLE control, SAVE_MQTT, schedules, power loss

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: HVAC 2 experiments

**Files:**
- Modify: `docs/findings.md`, `fixtures/scentiment/*.json`

**Interfaces:**
- Consumes: the same tools as Task 7. Its own `deviceId` comes from Task 6.

- [ ] **Step 1: Bluetooth control.** As in Task 7, Step 1, but `SET_RGB_*` only if its GATT dump or notifications suggest it has an LED.
- [ ] **Step 2: `SAVE_MQTT` and WiFi.** As in Task 7, Steps 3–5, using whichever command order and URL worked for the Air 2.
- [ ] **Step 3: Schedule and power cut.** As in Task 7, Steps 7–8.
- [ ] **Step 4: Touchscreen.** The owner edits a schedule and changes intensity on the touchscreen. Record what the device publishes (`update_profile`? `notification`?) and whether a later `UPDATE_PROFILE` from us overwrites the touchscreen schedule.
- [ ] **Step 5: If the HVAC 2 speaks a different protocol** (Task 6, Step 3): record its full GATT table, advertisement and any notifications. Don't send any commands. Per spec §5, v1 continues with the Air 2 only.
- [ ] **Step 6: Commit.**

```bash
git add docs/findings.md fixtures/scentiment
git commit -m "Record HVAC 2 Phase 0 experiments

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Gate report

**Files:**
- Modify: `docs/findings.md` (a "Phase 0 results" summary at the top), `docs/superpowers/specs/2026-10-20-scent-control-design.md` (an "Appendix: Phase 0 results" listing every place reality differed from the spec)

**Interfaces:**
- Produces: the go/no-go decision and the facts the Phase 1 plan is written from

- [ ] **Step 1: Write the summary.** One row per design assumption, each marked confirmed, different or unknown, with a link to its findings entry:
  - BLE control;
  - `SAVE_MQTT` over BLE;
  - command order;
  - URL form;
  - `PING`/`userid` persistence;
  - control over MQTT;
  - on-device schedules;
  - power-cut behavior;
  - button and touchscreen reporting;
  - oil;
  - HVAC protocol.
- [ ] **Step 2: Apply the gate rules** from spec §7:
  - `SAVE_MQTT` works over BLE **and** schedules run on the device → **go**;
  - `SAVE_MQTT` fails → **stop**, and present fallbacks (a) and (b);
  - schedules don't run on the device → **stop**, and present fallback (c);
  - only plain `mqtt://` worked, or a non-Railway endpoint is needed → **stop and ask**.
- [ ] **Step 3: Commit, and present the result to the owner.**

```bash
git add docs/findings.md docs/superpowers/specs/2026-10-20-scent-control-design.md
git commit -m "Phase 0 gate report

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

  After a **go**, write the Phase 1 plan from the confirmed facts. That's a new plan, not part of this one. The diffusers stay pointed at the test broker until Phase 1's broker exists, and are then moved over with `SAVE_MQTT`.
