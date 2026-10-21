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
