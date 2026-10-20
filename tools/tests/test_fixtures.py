"""Every sanitized fixture parses, and every message shape seen in Phase 0 has one."""
import json
from pathlib import Path

from control_server import parse_device_message
from scentiment_ble import parse_info, parse_notification

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures" / "scentiment"
SHAPES = {
    "air2-ble-info", "hvac-ble-info", "air2-ble-status-text", "hvac-ble-status-text",
    "air2-ble-status-array", "hvac-ble-status-array", "air2-ble-value-enable", "air2-ble-value-level", "air2-ble-value-led",
    "air2-mqtt-device-connected-server-notifications", "air2-mqtt-device-connected-user", "hvac-mqtt-device-connected-user",
    "air2-mqtt-ack-added", "air2-mqtt-ack-already-added", "air2-mqtt-notification-invalid-json", "air2-mqtt-update-status-object",
}

def load():
    return {p.stem: json.loads(p.read_text()) for p in FIXTURES.glob("*.json")}

def test_every_observed_shape_has_a_fixture():
    assert SHAPES <= set(load())

def test_fixtures_parse_and_use_only_synthetic_ids():
    for name, f in load().items():
        assert f["source"].startswith("observed 2026-"), name
        if f["channel"] == "ble-info":
            assert parse_info(f["text"]).device_id in {"TESTDEVICE01", "TESTDEVICE02"}, name
        elif f["channel"] == "ble-notify":
            assert parse_notification(f["text"].encode())["kind"] != "unknown", name
        else:
            m = parse_device_message(f["topic"], f["text"].encode())
            assert m["action"] and m["deviceId"] in {"TESTDEVICE01", "TESTDEVICE02"}, name
