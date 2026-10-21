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

def test_capture_log_masks_echoed_secrets_in_rx(tmp_path):
    log = CaptureLog(tmp_path / "s.jsonl")
    echoed = '{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"hunter2!"}}'
    log.rx(echoed, {"kind": "json", "json": json.loads(echoed)}, "ble-notify")
    text = (tmp_path / "s.jsonl").read_text()
    assert "hunter2!" not in text and "ExampleWiFi" in text

def test_captures_dir_follows_environment(monkeypatch, tmp_path):
    import capture
    from pathlib import Path
    monkeypatch.setenv("SCENT_CAPTURES_DIR", str(tmp_path / "elsewhere"))
    assert capture.captures_dir() == tmp_path / "elsewhere"
    monkeypatch.delenv("SCENT_CAPTURES_DIR")
    assert capture.captures_dir() == Path(capture.__file__).resolve().parent.parent / "captures"
