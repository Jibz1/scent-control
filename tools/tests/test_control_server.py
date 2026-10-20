import pytest
from control_server import build_publish, device_id_from_name, parse_device_message
from safety import RefusedCommand

def test_device_id_from_name():
    assert device_id_from_name("Scentiment-D B n 48F6EE9B8FB0") == "48F6EE9B8FB0"
    assert device_id_from_name("Scentiment-H B n f85b1b4ba63c") == "F85B1B4BA63C"
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
