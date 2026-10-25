import pytest
import broker
from broker import parse_broker_url, BrokerTarget, broker_setting, build_parser, main

BROKER_KEYS = ("SCENT_BROKER_URL", "SCENT_BROKER_USERNAME", "SCENT_ROUTE_ID")


@pytest.fixture
def setting_files(tmp_path, monkeypatch):
    """The repo-root .env and tools/.env.local, as empty temporary files, and none of the
    broker settings in the environment, so the developer's own don't leak in."""
    for key in BROKER_KEYS:
        monkeypatch.delenv(key, raising=False)
    root_env, env_local = tmp_path / "root.env", tmp_path / ".env.local"
    monkeypatch.setattr(broker, "SETTING_FILES", (root_env, env_local))
    return root_env, env_local


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


def test_a_broker_setting_comes_from_the_environment_then_the_repo_env_then_env_local(setting_files, monkeypatch):
    root_env, env_local = setting_files
    root_env.write_text(
        "# the repo-root .env\n"
        "SCENT_BROKER_URL=wss://root.example.test/mqtt\n"
        'export SCENT_BROKER_USERNAME="root-user"\n'
    )
    env_local.write_text(
        "SCENT_BROKER_URL=wss://local.example.test/mqtt\n"
        "SCENT_BROKER_USERNAME=local-user\n"
        "SCENT_ROUTE_ID=route-test-1\n"
    )
    assert broker_setting("SCENT_BROKER_URL") == "wss://root.example.test/mqtt"
    assert broker_setting("SCENT_BROKER_USERNAME") == "root-user"
    # Only in tools/.env.local.
    assert broker_setting("SCENT_ROUTE_ID") == "route-test-1"
    monkeypatch.setenv("SCENT_BROKER_URL", "wss://shell.example.test/mqtt")
    assert broker_setting("SCENT_BROKER_URL") == "wss://shell.example.test/mqtt"


def test_inline_comments_and_crlf_line_ends_are_not_part_of_a_setting(setting_files):
    root_env, _ = setting_files
    root_env.write_bytes(
        b"SCENT_BROKER_URL=wss://root.example.test/mqtt # mine\r\n"
        b'SCENT_BROKER_USERNAME="example # user" # the comment, not the hash inside\r\n'
        b"SCENT_ROUTE_ID=route#1\r\n"
    )
    assert broker_setting("SCENT_BROKER_URL") == "wss://root.example.test/mqtt"
    assert broker_setting("SCENT_BROKER_USERNAME") == "example # user"
    # A # with no space before it is part of the value.
    assert broker_setting("SCENT_ROUTE_ID") == "route#1"


def test_an_empty_setting_in_the_environment_does_not_hide_the_files(setting_files, monkeypatch):
    root_env, _ = setting_files
    root_env.write_text("SCENT_BROKER_URL=wss://root.example.test/mqtt\n")
    monkeypatch.setenv("SCENT_BROKER_URL", "")
    assert broker_setting("SCENT_BROKER_URL") == "wss://root.example.test/mqtt"


def test_a_broker_setting_found_nowhere_is_none(setting_files):
    for key in BROKER_KEYS:
        assert broker_setting(key) is None


def test_a_password_is_never_a_broker_setting(setting_files):
    with pytest.raises(ValueError):
        broker_setting("SCENT_BROKER_PASSWORD")


def test_url_and_username_default_to_the_broker_settings(setting_files):
    root_env, _ = setting_files
    root_env.write_text("SCENT_BROKER_URL=wss://root.example.test/mqtt\nSCENT_BROKER_USERNAME=root-user\n")
    args = build_parser().parse_args(["watch"])
    assert (args.url, args.username) == ("wss://root.example.test/mqtt", "root-user")
    # Given on the command line, they win.
    args = build_parser().parse_args(["watch", "--url", "wss://b.example.test/mqtt", "--username", "u"])
    assert (args.url, args.username) == ("wss://b.example.test/mqtt", "u")


def test_without_broker_settings_url_and_username_are_required(setting_files, capsys):
    with pytest.raises(SystemExit):
        build_parser().parse_args(["watch"])
    assert "--url" in capsys.readouterr().err
