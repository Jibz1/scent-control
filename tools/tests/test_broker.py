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
