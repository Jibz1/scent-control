import pytest
from control_server import build_publish, device_id_from_name, parse_device_message
from safety import RefusedCommand

def test_device_id_from_name():
    assert device_id_from_name("Scentiment-D B n A1B2C3D4E5F6") == "A1B2C3D4E5F6"
    assert device_id_from_name("Scentiment-H B n 0a1b2c3d4e5f") == "0A1B2C3D4E5F"
    assert device_id_from_name("Scentiment-D B") is None
    assert device_id_from_name(None) is None

def test_parse_device_message_repairs_unquoted_notification_data():
    # Exactly what an Air 2 (firmware 2.30) sent on 2026-10-20 in reply to GET_STATUS.
    raw = b'{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:0,"rssi":-46}}'
    m = parse_device_message("scentiment-mqtt-prod/user/route-test-1", raw)
    assert (m["action"], m["deviceId"], m["valid_json"]) == ("notification", "TESTDEVICE01", False)
    assert m["payload"]["rssi"] == -46 and m["status"]["power"] == 0

def test_parse_device_message_reads_status_array_inside_update_status():
    raw = b'{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":"{\\"data\\":[85,170,11,255,1,2,200,100,255,3,3,255,255,0,41,106]}"}}'
    m = parse_device_message("scentiment-mqtt-prod/user/route-test-1", raw)
    assert m["valid_json"] and m["status"]["power"] == 1 and m["status"]["intensity"] == 3

def test_parse_device_message_takes_device_id_from_device_topic():
    m = parse_device_message("scentiment-mqtt-prod/device/TESTDEVICE01", b'{"action":"PING","payload":{"userid":"route-test-1"}}')
    assert (m["direction"], m["deviceId"], m["action"]) == ("to_device", "TESTDEVICE01", "PING")
    connected = parse_device_message("scentiment-mqtt-prod/server-notifications/device",
        b'{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}')
    assert (connected["direction"], connected["deviceId"]) == ("from_device", "TESTDEVICE01")

def test_parse_device_message_never_raises():
    m = parse_device_message("anything", b"\xff not json at all")
    assert m["action"] is None and m["raw"]

def test_build_publish_targets_the_device_topic():
    topic, envelope = build_publish({"deviceId": "TESTDEVICE01", "action": "SET_LEVEL", "payload": {"intensity": 2}})
    assert topic == "scentiment-mqtt-prod/device/TESTDEVICE01"
    assert envelope == {"action": "SET_LEVEL", "payload": {"intensity": 2}}

def test_build_publish_refuses_blocked_actions():
    with pytest.raises(RefusedCommand):
        build_publish({"deviceId": "TESTDEVICE01", "action": "UPDATE_FIRMWARE", "payload": {"url": "https://x"}})

def test_build_publish_rejects_bad_device_ids():
    for bad in ("", "#", "TESTDEVICE01/../x", "abc"):
        with pytest.raises(ValueError):
            build_publish({"deviceId": bad, "action": "PING", "payload": None})

def test_parse_device_message_reads_status_array_given_as_an_object():
    # Firmware 2.30 over MQTT sends the array as an object, not a string (observed 2026-10-20).
    raw = b'{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}'
    m = parse_device_message("scentiment-mqtt-prod/user/route-test-1", raw)
    assert m["valid_json"] and m["status"]["power"] == 1 and m["status"]["intensity"] == 2 and m["payload"]["rssi"] == -66

def test_parse_status_output_takes_the_last_json_line():
    from control_server import parse_status_output
    out = 'connected to air2 (mtu 256)\n{"alias": "air2", "deviceId": "TESTDEVICE01", "status": {"power": 0}}\nlog: /tmp/x.jsonl\n'
    assert parse_status_output(out)["status"] == {"power": 0}
    with pytest.raises(ValueError):
        parse_status_output("error: could not connect\n")

def test_ble_status_event_carries_status_for_the_page():
    from control_server import ble_status_event
    event = ble_status_event({"alias": "air2", "deviceId": "TESTDEVICE01", "status": {"power": 1, "intensity": 2, "charging": 2, "battery": 200}, "info": {}})
    assert (event["deviceId"], event["action"], event["direction"], event["topic"]) == ("TESTDEVICE01", "ble_status", "from_device", "bluetooth")
    assert event["status"]["power"] == 1 and event["via"] == "bluetooth"

def test_events_from_session_log_turns_a_bluetooth_session_into_events():
    import json as _json
    from control_server import events_from_session_log
    lines = [
        _json.dumps({"ts": "2026-10-20T14:50:00.000+00:00", "dir": "note", "channel": "", "text": "connected to air2, mtu 256"}),
        _json.dumps({"ts": "2026-10-20T14:50:01.000+00:00", "dir": "rx", "channel": "ble-notify", "text": "level:3", "parsed": {"raw": "level:3", "kind": "value", "intensity": 3}}),
        _json.dumps({"ts": "2026-10-20T14:50:01.000+00:00", "dir": "tx", "channel": "ble", "text": '{"action":"SET_LEVEL","payload":{"intensity":3}}-|'}),
        _json.dumps({"ts": "2026-10-20T14:50:02.000+00:00", "dir": "rx", "channel": "ble-notify", "text": "led:50", "parsed": {"raw": "led:50", "kind": "value", "led": 50}}),
    ]
    events = events_from_session_log(lines, "TESTDEVICE01")
    assert [(e["direction"], e["action"]) for e in events] == [("from_device", "level:3"), ("to_device", "SET_LEVEL"), ("from_device", "led:50")]
    assert all(e["via"] == "bluetooth" and e["deviceId"] == "TESTDEVICE01" for e in events)
    assert events[0]["status"] == {"raw": "level:3", "kind": "value", "intensity": 3}
    assert events[1]["payload"] == {"intensity": 3}

def test_replay_captures_rebuilds_last_known_state(tmp_path):
    import json as _json
    from control_server import replay_captures
    (tmp_path / "broker-20261020T090829.jsonl").write_text(_json.dumps({
        "ts": "2026-10-20T14:29:17.000+00:00", "dir": "rx", "channel": "mqtt:scentiment-mqtt-prod/user/route-test-1",
        "text": '{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-67}}'}) + "\n")
    (tmp_path / "session-hvac-20261020T084839.jsonl").write_text(_json.dumps({
        "ts": "2026-10-20T13:48:40.000+00:00", "dir": "rx", "channel": "ble-notify", "text": "S:0,L:1,C:2,Bat:200",
        "parsed": {"raw": "S:0,L:1,C:2,Bat:200", "kind": "status_text", "power": 0, "intensity": 1, "charging": 2, "battery": 200}}) + "\n")
    devices = {"TESTDEVICE01": {"deviceId": "TESTDEVICE01", "label": "Air 2", "alias": "air2"},
               "TESTDEVICE02": {"deviceId": "TESTDEVICE02", "label": "HVAC 2", "alias": "hvac"}}
    assert replay_captures(tmp_path, devices) == 2
    air2, hvac = devices["TESTDEVICE01"], devices["TESTDEVICE02"]
    assert (air2["status"]["charging"], air2["status"]["battery"], air2["rssi"]) == (2, 200, -67) and air2["lastSeen"]
    assert (hvac["status"]["power"], hvac["status"]["charging"]) == (0, 2) and hvac["bleSeen"] and "lastSeen" not in hvac

def test_online_status_from_ping_and_ack():
    from control_server import online_status
    assert online_status(ping_at=None, ack_at=None, now="2026-10-20T10:30:00") == "unknown"
    assert online_status("2026-10-20T10:29:50", "2026-10-20T10:29:51", "2026-10-20T10:30:00") == "online"
    assert online_status("2026-10-20T10:29:50", "2026-10-20T10:28:00", "2026-10-20T10:30:00") == "waiting"
    assert online_status("2026-10-20T10:29:30", "2026-10-20T10:20:00", "2026-10-20T10:30:00") == "offline"

def test_apply_event_keeps_each_reading_with_its_time_and_source():
    from control_server import apply_event
    devices = {}
    apply_event(devices, {"deviceId": "TESTDEVICE01", "direction": "from_device", "ts": "2026-10-20T09:29:17", "action": "update_status",
                          "payload": {"rssi": -67}, "status": {"kind": "status_array", "power": 1, "charging": 2, "battery": 200}})
    apply_event(devices, {"deviceId": "TESTDEVICE01", "direction": "from_device", "ts": "2026-10-20T10:13:00", "via": "bluetooth",
                          "action": "enable:0", "payload": None, "status": {"kind": "value", "power": 0}})
    readings = devices["TESTDEVICE01"]["readings"]
    assert readings["power"] == {"value": 0, "at": "2026-10-20T10:13:00", "via": "bluetooth"}
    assert readings["charging"] == {"value": 2, "at": "2026-10-20T09:29:17", "via": "wifi"}
    assert readings["rssi"]["value"] == -67

def test_ack_events_mark_the_device_as_answering():
    from control_server import apply_event
    devices = {}
    apply_event(devices, {"deviceId": "TESTDEVICE01", "direction": "from_device", "ts": "2026-10-20T10:30:09", "action": "acknowledgement",
                          "payload": {"deviceId": "TESTDEVICE01", "msg": "{\"result\":\"ALREADY_ADDED\"}"}, "status": None})
    assert devices["TESTDEVICE01"]["ackAt"] == "2026-10-20T10:30:09"

FACTORY_OFF = {"index": 1, "startTime": "00:00", "endTime": "23:59", "weekdays": [0, 1, 2, 3, 4, 5, 6],
               "workingState": {"level": 1, "fan": 0, "led": 0, "timer": 0}, "enable": False, "isFactory": True}
WINDOW = {"startTime": "10:24", "endTime": "10:27", "weekdays": [2], "workingState": {"level": 3, "fan": 0, "led": 0, "timer": 0}, "enable": True}

def test_expected_state_follows_the_schedule_in_the_devices_time_zone():
    import datetime as dt
    from control_server import expected_state
    payload = {"profile": {"schedule": [FACTORY_OFF, WINDOW]}, "timezone": -5.0, "schedule_flag": 1}
    during = expected_state(payload, dt.datetime(2026, 10, 20, 15, 25, tzinfo=dt.timezone.utc))  # Tue 10:25 at UTC-5
    assert during == {"power": 1, "level": 3, "source": "schedule 10:24–10:27", "until": "10:27"}
    after = expected_state(payload, dt.datetime(2026, 10, 20, 15, 28, tzinfo=dt.timezone.utc))
    assert after == {"power": 0, "level": None, "source": "always-on entry", "until": None}

def test_sent_profiles_are_remembered_and_flag_0_only_replaces_the_always_on_entry():
    from control_server import apply_event
    devices = {}
    sent = lambda payload: {"deviceId": "TESTDEVICE01", "direction": "to_device", "ts": "2026-10-20T10:21:06", "action": "UPDATE_PROFILE", "payload": payload, "status": None}
    apply_event(devices, sent({"profile": {"schedule": [FACTORY_OFF, WINDOW]}, "timezone": -5.0, "schedule_flag": 1}))
    on = dict(FACTORY_OFF, enable=True)
    apply_event(devices, sent({"profile": {"schedule": [on]}, "timezone": -5.0, "schedule_flag": 0}))
    schedule = devices["TESTDEVICE01"]["sentProfile"]["profile"]["schedule"]
    assert schedule == [on, WINDOW] and devices["TESTDEVICE01"]["sentProfileAt"] == "2026-10-20T10:21:06"

def test_hub_resubscribes_after_the_broker_reconnects(tmp_path):
    from capture import CaptureLog
    from control_server import Hub, TOPIC_PREFIX
    class Reason:
        is_failure = False
    class FakeClient:
        def __init__(self):
            self.subscriptions, self.on_connect, self.on_message = [], None, None
        def subscribe(self, topic, qos=0):
            self.subscriptions.append(topic)
    client = FakeClient()
    Hub(client, CaptureLog(tmp_path / "c.jsonl"), {}, "route-test-1")
    client.on_connect(client, None, None, Reason(), None)  # paho reconnected with a clean session
    assert client.subscriptions == [TOPIC_PREFIX + "#", TOPIC_PREFIX + "#"]

def test_post_requests_must_come_from_the_bench_page():
    from control_server import allowed_post
    ok = {"Host": "127.0.0.1:8765", "Content-Type": "application/json", "Origin": "http://127.0.0.1:8765"}
    assert allowed_post(ok, 8765)
    assert allowed_post({**ok, "Host": "localhost:8765", "Origin": "http://localhost:8765"}, 8765)
    assert allowed_post({"Host": "127.0.0.1:8765", "Content-Type": "application/json"}, 8765)  # curl: no Origin
    assert not allowed_post({**ok, "Content-Type": "text/plain"}, 8765)        # a cross-site form or no-preflight fetch
    assert not allowed_post({**ok, "Origin": "https://evil.example"}, 8765)
    assert not allowed_post({**ok, "Host": "attacker.example:8765"}, 8765)     # DNS rebinding
