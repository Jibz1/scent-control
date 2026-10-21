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

def test_status_report_combines_info_and_status():
    from session import status_report
    from scentiment_ble import parse_info, parse_notification
    report = status_report("air2", parse_info("01|Diffuser|Black|TESTDEVICE01|2.30|true|3"), parse_notification(b"S:1,L:3,C:2,Bat:200"))
    assert report["deviceId"] == "TESTDEVICE01" and report["alias"] == "air2"
    assert report["status"] == {"power": 1, "intensity": 3, "charging": 2, "battery": 200}
    assert report["info"]["wifi_status"] == 3

def test_make_test_schedule_keeps_the_always_on_entry_off_by_default(capsys):
    assert main(["make-test-schedule"]) == 0
    factory = json.loads(capsys.readouterr().out)["profile"]["schedule"][0]
    assert factory["isFactory"] is True and factory["enable"] is False
