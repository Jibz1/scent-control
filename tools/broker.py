"""Watch and publish on the Phase 0 test broker (encrypted connections only)."""

import argparse
import getpass
import json
import os
import sys
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

import paho.mqtt.client as mqtt

from capture import CaptureLog, captures_dir, redact_text, stamp
from safety import RefusedCommand, check_allowed
from scentiment_ble import build_profile, parse_notification

ENV_LOCAL = Path(__file__).parent / ".env.local"


@dataclass(frozen=True)
class BrokerTarget:
    host: str
    port: int
    path: str
    tls: bool
    transport: str


def parse_broker_url(url: str) -> BrokerTarget:
    parts = urlsplit(url)
    if not parts.hostname:
        raise ValueError(f"no host in broker URL {url!r}")
    if parts.scheme == "wss":
        return BrokerTarget(parts.hostname, parts.port or 443, parts.path or "/mqtt", True, "websockets")
    if parts.scheme == "mqtts":
        return BrokerTarget(parts.hostname, parts.port or 8883, "", True, "tcp")
    raise ValueError(f"only encrypted wss:// and mqtts:// brokers are allowed, got {parts.scheme or 'no scheme'}://")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    def command(name: str, help: str) -> argparse.ArgumentParser:
        p = sub.add_parser(name, help=help)
        p.add_argument("--url", required=True, help="e.g. wss://<domain>/mqtt")
        p.add_argument("--username", required=True)
        return p

    command("watch", "print every message on the broker").add_argument("--seconds", type=float, help="stop after N seconds")
    publish = command("publish", "publish one {action, payload} envelope")
    publish.add_argument("--topic", required=True)
    publish.add_argument("--json", required=True, dest="envelope")
    profile = command("publish-profile", "publish UPDATE_PROFILE from a schedule file")
    profile.add_argument("--topic", required=True)
    profile.add_argument("--file", required=True, type=Path)
    profile.add_argument("--timezone", required=True, type=float, help="UTC offset in hours")
    profile.add_argument("--flag", type=int, default=1, help="schedule_flag (default 1)")
    command("sync-time", "publish SYNC_TIME with the current UTC time").add_argument("--topic", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        target = parse_broker_url(args.url)
        envelope = _envelope(args)
        if envelope is not None:
            check_allowed(envelope.get("action"), envelope.get("payload"))
    except RefusedCommand as err:
        print(f"refused: {err}", file=sys.stderr)
        return 2
    except (ValueError, OSError, KeyError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2

    log = CaptureLog(captures_dir() / f"broker-{stamp()}.jsonl")
    try:
        client = _connect(target, args.username, _password())
    except (OSError, ConnectionError) as err:
        print(f"error: could not connect to {target.host}:{target.port}: {err}", file=sys.stderr)
        return 1
    try:
        if envelope is None:
            _watch(client, log, args.seconds)
        else:
            _publish(client, log, args.topic, envelope)
    except KeyboardInterrupt:
        pass
    finally:
        client.loop_stop()
        client.disconnect()
    print(f"log: {log.path}")
    return 0


def _envelope(args: argparse.Namespace) -> dict | None:
    if args.command == "watch":
        return None
    if args.command == "publish":
        envelope = json.loads(args.envelope)
        if not isinstance(envelope, dict) or "action" not in envelope:
            raise ValueError('--json must be an envelope like {"action":"PING","payload":{...}}')
        return envelope
    if args.command == "publish-profile":
        entries = json.loads(args.file.read_text())["profile"]["schedule"]
        return {"action": "UPDATE_PROFILE", "payload": build_profile(entries, args.timezone, args.flag)}
    if args.command == "sync-time":
        return {"action": "SYNC_TIME", "payload": {"serverTime": int(time.time())}}
    raise ValueError(f"unknown command {args.command}")


def _password() -> str:
    if os.environ.get("SCENT_BROKER_PASSWORD"):
        return os.environ["SCENT_BROKER_PASSWORD"]
    if ENV_LOCAL.exists():
        for line in ENV_LOCAL.read_text().splitlines():
            key, _, value = line.partition("=")
            if key.strip() == "SCENT_BROKER_PASSWORD" and value.strip():
                return value.strip()
    return getpass.getpass("Broker password: ")


def _connect(target: BrokerTarget, username: str, password: str) -> mqtt.Client:
    client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=f"scent-tools-{uuid.uuid4().hex[:8]}", transport=target.transport)
    client.username_pw_set(username, password)
    if target.tls:
        client.tls_set()
    if target.transport == "websockets":
        client.ws_set_options(path=target.path)
    connected = threading.Event()
    result: dict = {}

    def on_connect(_client, _userdata, _flags, reason_code, _properties):
        result["reason"] = reason_code
        connected.set()

    client.on_connect = on_connect
    client.connect(target.host, target.port, keepalive=30)
    client.loop_start()
    if not connected.wait(15):
        client.loop_stop()
        raise ConnectionError("no reply from the broker within 15 s")
    if result["reason"].is_failure:
        client.loop_stop()
        raise ConnectionError(f"broker refused the login: {result['reason']}")
    print(f"connected to {target.host}:{target.port}{target.path} as {username}")
    return client


def _watch(client: mqtt.Client, log: CaptureLog, seconds: float | None) -> None:
    def on_message(_client, _userdata, message):
        text = message.payload.decode("utf-8", errors="replace")
        parsed: dict = {"kind": "mqtt"}
        try:
            body = json.loads(text)
            inner = body.get("payload", {}).get("data") if isinstance(body, dict) else None
            if isinstance(inner, str):
                parsed = parse_notification(inner.encode("utf-8"))
        except (ValueError, AttributeError):
            pass
        log.rx(text, parsed, f"mqtt:{message.topic}")
        summary = f"  [{parsed['kind']}]" if parsed["kind"] != "mqtt" else ""
        print(f"{time.strftime('%H:%M:%S')} {message.topic} {redact_text(text)}{summary}", flush=True)

    client.on_message = on_message
    client.subscribe("#", qos=1)
    print("watching '#' (Ctrl-C to stop)", flush=True)
    deadline = time.monotonic() + seconds if seconds else None
    while deadline is None or time.monotonic() < deadline:
        time.sleep(0.5)


def _publish(client: mqtt.Client, log: CaptureLog, topic: str, envelope: dict) -> None:
    body = json.dumps(envelope, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    info = client.publish(topic, body, qos=1)
    info.wait_for_publish(10)
    log.tx(body, f"mqtt:{topic}")
    print(f"published to {topic}: {redact_text(body.decode('utf-8'))}")


if __name__ == "__main__":
    sys.exit(main())
