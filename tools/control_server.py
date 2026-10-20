"""Local test bench: a web page for controlling diffusers through the Phase 0 broker.

Serves control.html on 127.0.0.1 only. This process holds the broker login and
checks every command against the same allow-list as the other tools, so the page
never sees the password and can't send a blocked command.

    .venv/bin/python control_server.py --url wss://<domain>/mqtt --username scent-test
"""

import argparse
import datetime as dt
import json
import queue
import re
import subprocess
import sys
import threading
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from broker import connect, parse_broker_url, password_from_env
from capture import CaptureLog, captures_dir, stamp
from devices import DEFAULT_CONFIG, load_devices
from safety import RefusedCommand, check_allowed
from scentiment_ble import parse_notification

PAGE = Path(__file__).parent / "control.html"
BT = Path(__file__).parent / "bt"
TOPIC_PREFIX = "scentiment-mqtt-prod/"
DEVICE_TOPIC = TOPIC_PREFIX + "device/"
DEVICE_ID = re.compile(r"[0-9A-Z]{12}")
# Firmware 2.30 sends `"data":enable:0` (no quotes) in notifications.
UNQUOTED_DATA = re.compile(r'"data":([^"{\[][^,}]*)')
LABELS = {"air2": "Air 2", "hvac": "HVAC 2"}


def device_id_from_name(name: str | None) -> str | None:
    """The device ID is the last token of the advertised Bluetooth name."""
    token = name.split()[-1] if name and name.split() else ""
    return token.upper() if re.fullmatch(r"[0-9A-Fa-f]{12}", token) else None


def parse_device_message(topic: str, payload: bytes) -> dict:
    text = payload.decode("utf-8", errors="replace")
    message: dict = {"topic": topic, "raw": text, "action": None, "deviceId": None, "payload": None, "status": None, "valid_json": True}
    message["direction"] = "to_device" if topic.startswith(DEVICE_TOPIC) else "from_device"
    if topic.startswith(DEVICE_TOPIC):
        message["deviceId"] = topic[len(DEVICE_TOPIC):]
    body = _loads(text)
    if body is None:
        message["valid_json"] = False
        body = _loads(UNQUOTED_DATA.sub(lambda m: '"data":' + json.dumps(m.group(1).strip()), text))
    if not isinstance(body, dict):
        return message
    message["action"] = body.get("action")
    inner = body.get("payload")
    if isinstance(inner, dict):
        message["payload"] = inner
        message["deviceId"] = inner.get("deviceId") or message["deviceId"]
        data = inner.get("data")
        if isinstance(data, dict):  # update_status over MQTT: the array arrives as an object
            data = json.dumps(data)
        if isinstance(data, str):
            parsed = parse_notification(data.encode("utf-8"))
            if parsed["kind"] != "unknown":
                message["status"] = parsed
    return message


def build_publish(body: dict) -> tuple[str, dict]:
    device_id = str(body.get("deviceId") or "")
    if not DEVICE_ID.fullmatch(device_id):
        raise ValueError(f"not a device ID: {device_id!r}")
    action = body.get("action")
    payload = body.get("payload")
    check_allowed(action, payload)
    envelope = {"action": action} if payload is None else {"action": action, "payload": payload}
    return DEVICE_TOPIC + device_id, envelope


def parse_status_output(stdout: str) -> dict:
    """The last JSON line printed by `session.py status`."""
    for line in reversed(stdout.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            return json.loads(line)
    raise ValueError("the Bluetooth read printed no status")


def ble_status_event(report: dict) -> dict:
    return {
        "topic": "bluetooth",
        "raw": json.dumps(report),
        "action": "ble_status",
        "deviceId": report.get("deviceId"),
        "payload": report,
        "status": report.get("status") or {},
        "valid_json": True,
        "direction": "from_device",
        "via": "bluetooth",
    }


def events_from_session_log(lines: list[str], device_id: str) -> list[dict]:
    """Conversation events from a `session.py` capture log (one Bluetooth session)."""
    events = []
    for line in lines:
        record = _loads(line)
        if not isinstance(record, dict) or record.get("dir") not in ("tx", "rx"):
            continue
        ts = dt.datetime.fromisoformat(record["ts"]).astimezone().replace(tzinfo=None).isoformat(timespec="seconds")
        text = record.get("text", "")
        event = {"topic": "bluetooth", "raw": text, "deviceId": device_id, "payload": None, "status": None,
                 "valid_json": True, "via": "bluetooth", "ts": ts}
        if record["dir"] == "tx":
            body = _loads(text[:-2] if text.endswith("-|") else text)
            body = body if isinstance(body, dict) else {}
            event.update(direction="to_device", action=body.get("action"), payload=body.get("payload"))
        else:
            parsed = record.get("parsed") or {}
            kind = parsed.get("kind")
            event.update(direction="from_device", action=text if kind in ("value", "unknown", None) else "status")
            if kind in ("status_text", "status_array", "value"):
                event["status"] = parsed
        events.append(event)
    return events


PING_EVERY_S = 30
ANSWER_WITHIN_S = 15
ONLINE_FOR_S = 75


def online_status(ping_at: str | None, ack_at: str | None, now: str) -> str:
    """"online", "waiting" (pinged, no answer yet), "offline" (pinged, no answer in time) or "unknown"."""
    seconds = lambda a, b: (dt.datetime.fromisoformat(b) - dt.datetime.fromisoformat(a)).total_seconds()
    if ack_at and (not ping_at or ack_at >= ping_at):
        return "online" if seconds(ack_at, now) <= ONLINE_FOR_S else "unknown"
    if ping_at:
        return "waiting" if seconds(ping_at, now) <= ANSWER_WITHIN_S else "offline"
    return "unknown"


def _is_factory(entry: dict) -> bool:
    return bool(entry.get("isFactory")) or entry.get("index") == 1


def expected_state(payload: dict, now_utc: dt.datetime) -> dict:
    """What a diffuser should be doing at `now_utc` according to a sent UPDATE_PROFILE payload."""
    schedule = (payload.get("profile") or {}).get("schedule") or []
    local = now_utc.astimezone(dt.timezone(dt.timedelta(hours=float(payload.get("timezone") or 0))))
    weekday, clock = (local.weekday() + 1) % 7, local.strftime("%H:%M")
    for entry in schedule:
        if not _is_factory(entry) and entry.get("enable") and weekday in (entry.get("weekdays") or []) \
                and entry.get("startTime", "") <= clock < entry.get("endTime", ""):
            level = (entry.get("workingState") or {}).get("level")
            return {"power": 1, "level": level, "source": f"schedule {entry['startTime']}–{entry['endTime']}", "until": entry["endTime"]}
    factory = next((e for e in schedule if _is_factory(e)), None)
    if factory and factory.get("enable"):
        return {"power": 1, "level": (factory.get("workingState") or {}).get("level"), "source": "always-on entry", "until": None}
    return {"power": 0, "level": None, "source": "always-on entry", "until": None}


def _remember_sent_profile(device: dict, event: dict) -> None:
    payload = event.get("payload") or {}
    if payload.get("schedule_flag") == 0 and device.get("sentProfile"):
        # flag 0 changes only the always-on entry (manual state); keep the timed entries we sent before
        factory = [e for e in (payload.get("profile") or {}).get("schedule") or [] if _is_factory(e)]
        timed = [e for e in device["sentProfile"]["profile"]["schedule"] if not _is_factory(e)]
        payload = dict(payload, profile={"schedule": factory + timed})
    device["sentProfile"] = payload
    device["sentProfileAt"] = event["ts"]


def apply_event(devices: dict, event: dict) -> None:
    """Fold one message into what we know about its device, keeping each reading's time and source."""
    device_id = event.get("deviceId")
    if not device_id or not DEVICE_ID.fullmatch(device_id):
        return
    if event.get("direction") == "to_device":
        if event.get("action") == "UPDATE_PROFILE" and isinstance(event.get("payload"), dict):
            _remember_sent_profile(devices.setdefault(device_id, {"deviceId": device_id, "label": device_id}), event)
        return
    if event.get("direction") != "from_device":
        return
    device = devices.setdefault(device_id, {"deviceId": device_id, "label": device_id})
    via = "bluetooth" if event.get("via") == "bluetooth" else "wifi"
    device["bleSeen" if via == "bluetooth" else "lastSeen"] = event["ts"]
    payload = event.get("payload") or {}
    if event.get("action") == "acknowledgement":
        device["ackAt"] = event["ts"]
    if event.get("action") == "device_connected" and payload:
        device["info"] = payload
    readings = device.setdefault("readings", {})
    if event.get("status"):
        fresh = {k: v for k, v in event["status"].items() if k not in ("raw", "kind", "array")}
        device.setdefault("status", {}).update(fresh)
        for key, value in fresh.items():
            readings[key] = {"value": value, "at": event["ts"], "via": via}
    if "rssi" in payload:
        device["rssi"] = payload["rssi"]
        readings["rssi"] = {"value": payload["rssi"], "at": event["ts"], "via": via}
    if isinstance(payload.get("profile"), dict):
        device["profile"] = payload["profile"]


def replay_captures(folder: Path, devices: dict) -> int:
    """Rebuild each device's last known state from earlier captures, oldest first."""
    alias_to_id = {d["alias"]: device_id for device_id, d in devices.items() if d.get("alias")}
    events: list[dict] = []
    for path in sorted(folder.glob("*.jsonl")):
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        if path.name.startswith(("broker-", "control-")):
            for line in lines:
                record = _loads(line)
                if isinstance(record, dict) and record.get("dir") == "rx" and str(record.get("channel", "")).startswith("mqtt:"):
                    event = parse_device_message(record["channel"][5:], record.get("text", "").encode("utf-8"))
                    event["ts"] = dt.datetime.fromisoformat(record["ts"]).astimezone().replace(tzinfo=None).isoformat(timespec="seconds")
                    events.append(event)
        elif path.name.startswith("session-"):
            device_id = alias_to_id.get(path.name[len("session-"):].split("-")[0])
            if device_id:
                events.extend(events_from_session_log(lines, device_id))
    events.sort(key=lambda e: e["ts"])
    for event in events:
        apply_event(devices, event)
    return len([e for e in events if e.get("direction") == "from_device"])


def allowed_post(headers, port: int) -> bool:
    """Only the bench page itself may send commands, not other sites open in the same browser.

    JSON bodies force a CORS preflight that this server never answers, and Host/Origin
    checks stop DNS rebinding. Tools without an Origin (curl) are still allowed.
    """
    local = {f"127.0.0.1:{port}", f"localhost:{port}"}
    origin = headers.get("Origin")
    return (
        (headers.get("Content-Type") or "").split(";")[0].strip() == "application/json"
        and headers.get("Host") in local
        and (origin is None or origin in {f"http://{h}" for h in local})
    )


def _loads(text: str) -> object:
    try:
        return json.loads(text)
    except ValueError:
        return None


class Hub:
    """Broker connection, recent messages, and what we know about each device."""

    def __init__(self, client, log: CaptureLog, devices: dict, route_id: str) -> None:
        self.client = client
        self.log = log
        self.route_id = route_id
        self.devices = devices
        self.events: deque = deque(maxlen=500)
        self.listeners: list[queue.Queue] = []
        self.lock = threading.Lock()
        client.on_message = self._on_message
        client.on_connect = self._on_connect
        client.subscribe(TOPIC_PREFIX + "#", qos=1)

    def _on_connect(self, client, _userdata, _flags, reason_code, _properties) -> None:
        # paho reconnects on its own with a clean session, which drops subscriptions
        if not reason_code.is_failure:
            client.subscribe(TOPIC_PREFIX + "#", qos=1)

    def state(self) -> dict:
        now = dt.datetime.now().isoformat(timespec="seconds")
        with self.lock:
            now_utc = dt.datetime.now(dt.timezone.utc)
            for device in self.devices.values():
                device["online"] = online_status(device.get("pingAt"), device.get("lastSeen"), now)
                if device.get("sentProfile"):
                    device["expected"] = expected_state(device["sentProfile"], now_utc)
            return {"devices": self.devices, "routeId": self.route_id, "now": now}

    def keep_pinging(self) -> None:
        """Ping every diffuser that has used this broker, so "online" means "answered recently"."""
        def loop() -> None:
            while True:
                threading.Event().wait(PING_EVERY_S)
                with self.lock:
                    on_broker = [d for d, info in self.devices.items() if info.get("lastSeen")]
                for device_id in on_broker:
                    self.ping(device_id)
        threading.Thread(target=loop, daemon=True).start()

    def ping(self, device_id: str) -> None:
        topic, envelope = build_publish({"deviceId": device_id, "action": "PING", "payload": {"userid": self.route_id}})
        with self.lock:
            self.devices.setdefault(device_id, {"deviceId": device_id, "label": device_id})["pingAt"] = dt.datetime.now().isoformat(timespec="seconds")
        self.publish(topic, envelope)

    def recent(self) -> list[dict]:
        with self.lock:
            return list(self.events)

    def listen(self) -> queue.Queue:
        q: queue.Queue = queue.Queue()
        with self.lock:
            self.listeners.append(q)
        return q

    def forget(self, q: queue.Queue) -> None:
        with self.lock:
            if q in self.listeners:
                self.listeners.remove(q)

    def publish(self, topic: str, envelope: dict) -> None:
        body = json.dumps(envelope, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        self.client.publish(topic, body, qos=1).wait_for_publish(10)
        self.log.tx(body, f"mqtt:{topic}")

    def greet(self) -> None:
        """Ask every known diffuser to answer, so the page shows which ones are on this broker."""
        for device_id in list(self.devices):
            self.ping(device_id)

    def read_over_bluetooth(self, device_id: str) -> dict:
        """Run `bt session.py status <alias>` (Bluetooth needs the ScentTools wrapper) and record the result."""
        alias = self.devices.get(device_id, {}).get("alias")
        if not alias:
            raise ValueError(f"no tools/devices.toml alias for {device_id}, so its Bluetooth address is unknown")
        result = subprocess.run([str(BT), "session.py", "status", alias], capture_output=True, text=True, timeout=90)
        try:
            report = parse_status_output(result.stdout)
        except ValueError:
            detail = (result.stderr.strip().splitlines() or ["no output"])[-1]
            raise ValueError(f"Bluetooth read of {alias} failed: {detail}") from None
        event = ble_status_event(report)
        event["ts"] = dt.datetime.now().isoformat(timespec="seconds")
        self.log.rx(event["raw"], {k: v for k, v in event.items() if k != "raw"}, "ble-status-read")
        self.ingest(event)
        return report

    def send_over_bluetooth(self, device_id: str, action: str, payload: dict | None) -> int:
        """Send one command with `bt session.py send` and add the session's messages to the conversation."""
        build_publish({"deviceId": device_id, "action": action, "payload": payload})  # same checks as WiFi
        alias = self.devices.get(device_id, {}).get("alias")
        if not alias:
            raise ValueError(f"no tools/devices.toml alias for {device_id}, so its Bluetooth address is unknown")
        args = [str(BT), "session.py", "send", alias, action] + ([json.dumps(payload)] if payload is not None else []) + ["--listen", "6"]
        result = subprocess.run(args, capture_output=True, text=True, timeout=90)
        log_line = next((l for l in reversed(result.stdout.splitlines()) if l.startswith("log: ")), None)
        if not log_line:
            detail = (result.stderr.strip().splitlines() or ["no output"])[-1]
            raise ValueError(f"Bluetooth send to {alias} failed: {detail}")
        events = events_from_session_log(Path(log_line[5:]).read_text().splitlines(), device_id)
        for event in events:
            self.ingest(event)
        return len(events)

    def _on_message(self, _client, _userdata, message) -> None:
        event = parse_device_message(message.topic, message.payload)
        event["ts"] = dt.datetime.now().isoformat(timespec="seconds")
        self.log.rx(event["raw"], {k: v for k, v in event.items() if k != "raw"}, f"mqtt:{message.topic}")
        self.ingest(event)

    def ingest(self, event: dict) -> None:
        with self.lock:
            apply_event(self.devices, event)
            self.events.append(event)
            listeners = list(self.listeners)
        for q in listeners:
            q.put(event)


def make_handler(hub: Hub):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args) -> None:  # keep the terminal for broker traffic
            pass

        def do_GET(self) -> None:
            if self.path in ("/", "/index.html"):
                self._send(200, PAGE.read_bytes(), "text/html; charset=utf-8")
            elif self.path == "/api/state":
                self._json(200, hub.state())
            elif self.path == "/api/events":
                self._events()
            else:
                self._json(404, {"error": "not found"})

        def do_POST(self) -> None:
            if not allowed_post(self.headers, self.server.server_port):
                return self._json(403, {"error": "commands are only accepted from the bench page"})
            if self.path == "/api/ble-read":
                try:
                    body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
                    report = hub.read_over_bluetooth(str(body.get("deviceId") or ""))
                except (ValueError, subprocess.TimeoutExpired) as err:
                    return self._json(502, {"error": str(err)})
                return self._json(200, {"status": report["status"]})
            if self.path == "/api/ble-send":
                try:
                    body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
                    count = hub.send_over_bluetooth(str(body.get("deviceId") or ""), body.get("action"), body.get("payload"))
                except RefusedCommand as err:
                    return self._json(403, {"refused": str(err)})
                except (ValueError, TypeError, OSError, subprocess.TimeoutExpired) as err:
                    return self._json(502, {"error": str(err)})
                return self._json(200, {"ok": True, "events": count})
            if self.path != "/api/publish":
                return self._json(404, {"error": "not found"})
            try:
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
                topic, envelope = build_publish(body)
                hub.publish(topic, envelope)
            except RefusedCommand as err:
                return self._json(403, {"refused": str(err)})
            except (ValueError, TypeError) as err:
                return self._json(400, {"error": str(err)})
            self._json(200, {"ok": True, "topic": topic})

        def _events(self) -> None:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            q = hub.listen()
            try:
                for event in hub.recent():
                    self._event(event)
                while True:
                    try:
                        self._event(q.get(timeout=15))
                    except queue.Empty:
                        self.wfile.write(b": keep-alive\n\n")
                        self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            finally:
                hub.forget(q)

        def _event(self, event: dict) -> None:
            self.wfile.write(b"data: " + json.dumps(event).encode("utf-8") + b"\n\n")
            self.wfile.flush()

        def _json(self, status: int, body: dict) -> None:
            self._send(status, json.dumps(body).encode("utf-8"), "application/json")

        def _send(self, status: int, body: bytes, content_type: str) -> None:
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

    return Handler


def known_devices(config: Path) -> dict:
    devices: dict = {}
    if not config.exists():
        return devices
    for alias, entry in load_devices(config).items():
        device_id = device_id_from_name(entry.name)
        if device_id:
            devices[device_id] = {"deviceId": device_id, "label": LABELS.get(alias, alias), "alias": alias}
    return devices


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", required=True, help="broker URL, e.g. wss://<domain>/mqtt")
    parser.add_argument("--username", required=True)
    parser.add_argument("--route-id", default="route-test-1", help="userid sent in PING (default route-test-1)")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args(argv)

    try:
        target = parse_broker_url(args.url)
        client = connect(target, args.username, password_from_env())
    except (ValueError, OSError, ConnectionError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    devices = known_devices(DEFAULT_CONFIG)
    replayed = replay_captures(captures_dir(), devices) if captures_dir().exists() else 0
    print(f"rebuilt last known state from {replayed} earlier messages", flush=True)
    hub = Hub(client, CaptureLog(captures_dir() / f"control-{stamp()}.jsonl"), devices, args.route_id)
    hub.greet()
    hub.keep_pinging()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(hub))
    print(f"test bench at http://127.0.0.1:{args.port}  (Ctrl-C to stop)", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        client.loop_stop()
        client.disconnect()
    return 0


if __name__ == "__main__":
    sys.exit(main())
