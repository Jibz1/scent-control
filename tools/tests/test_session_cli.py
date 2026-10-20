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
