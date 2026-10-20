"""Talk to one diffuser over Bluetooth: read info, listen, and send allowed commands."""

import argparse
import asyncio
import datetime as dt
import getpass
import json
import os
import sys
import time
from dataclasses import asdict
from pathlib import Path

from ble_client import ConnectError, DiffuserLink
from capture import CaptureLog, captures_dir, stamp
from devices import resolve
from safety import RefusedCommand, check_allowed
from scentiment_ble import DeviceInfo, build_profile, trial_schedule_entry

DEFAULT_FACTORY = {
    "index": 1,
    "startTime": "00:00",
    "endTime": "23:59",
    "weekdays": [0, 1, 2, 3, 4, 5, 6],
    "workingState": {"level": 1, "fan": 0, "led": 0, "timer": 0},
    "enable": True,
    "isFactory": True,
}


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    def device_command(name: str, help: str, listen: float) -> argparse.ArgumentParser:
        p = sub.add_parser(name, help=help)
        p.add_argument("alias", help="alias from devices.toml, or an address")
        p.add_argument("--listen", type=float, default=listen, help=f"seconds to listen afterwards (default {listen:g})")
        return p

    device_command("info", "read the 0xFEF4 device info", 0)
    device_command("status", "read info and status over Bluetooth, print one JSON line", 0)
    device_command("listen", "print notifications", 30).add_argument("--seconds", type=float, default=None, help="alias for --listen")
    send = device_command("send", "send one allowed action", 10)
    send.add_argument("action")
    send.add_argument("payload", nargs="?", help="JSON payload, e.g. '{\"on\":1}'")
    device_command("wifi", "send WIFI_CONNECT (prompts for the WiFi name and password)", 20)
    save = device_command("save-mqtt", "send SAVE_MQTT (prompts for the broker password)", 20)
    save.add_argument("--broker", required=True, help="e.g. wss://<domain>/mqtt")
    save.add_argument("--username", required=True)
    device_command("sync-time", "send SYNC_TIME with the current UTC time", 5)
    profile = device_command("profile", "send UPDATE_PROFILE from a schedule file", 10)
    profile.add_argument("file", type=Path)
    profile.add_argument("--timezone", type=float, required=True, help="UTC offset in hours, e.g. -4.0")
    profile.add_argument("--flag", type=int, default=1, help="schedule_flag (default 1)")
    device_command("restart", "soft restart (SECRET_COMMAND restart)", 5)

    trial = sub.add_parser("make-test-schedule", help="print a schedule file with a short test entry")
    trial.add_argument("--start-in", type=int, default=3, help="minutes from now (default 3)")
    trial.add_argument("--duration", type=int, default=4, help="minutes (default 4)")
    trial.add_argument("--level", type=int, default=1, choices=(1, 2, 3))
    trial.add_argument("--factory", type=Path, help="JSON file with the device's own factory entry")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "make-test-schedule":
        return _make_test_schedule(args)

    try:
        action, payload = _command(args)
        if action:
            check_allowed(action, payload)
    except RefusedCommand as err:
        print(f"refused: {err}", file=sys.stderr)
        return 2
    except (ValueError, OSError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2

    try:
        entry = resolve(args.alias)
    except (KeyError, FileNotFoundError, ValueError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2

    payload = _prompt_secrets(args, payload)
    log = CaptureLog(captures_dir() / f"session-{entry.alias}-{stamp()}.jsonl")
    listen = args.seconds if getattr(args, "seconds", None) is not None else args.listen
    try:
        asyncio.run(_run(entry, log, args.command, action, payload, listen))
    except ConnectError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    print(f"log: {log.path}")
    return 0


def _command(args: argparse.Namespace) -> tuple[str | None, dict | None]:
    """The action and payload a subcommand sends (secrets filled in later)."""
    if args.command in ("info", "listen", "status"):
        return None, None
    if args.command == "send":
        return args.action, json.loads(args.payload) if args.payload else None
    if args.command == "wifi":
        return "WIFI_CONNECT", {"ssid": "", "password": ""}
    if args.command == "save-mqtt":
        return "SAVE_MQTT", {"broker": args.broker, "username": args.username, "password": ""}
    if args.command == "sync-time":
        return "SYNC_TIME", {"serverTime": int(time.time())}
    if args.command == "profile":
        entries = json.loads(args.file.read_text())["profile"]["schedule"]
        return "UPDATE_PROFILE", build_profile(entries, args.timezone, args.flag)
    if args.command == "restart":
        return "SECRET_COMMAND", {"command": "restart"}
    raise ValueError(f"unknown command {args.command}")


def _prompt_secrets(args: argparse.Namespace, payload: dict | None) -> dict | None:
    if args.command == "wifi":
        ssid = input("WiFi name (SSID, 2.4 GHz): ").strip()
        if not ssid:
            raise SystemExit("error: the WiFi name can't be empty")
        return {"ssid": ssid, "password": getpass.getpass("WiFi password: ")}
    if args.command == "save-mqtt":
        password = os.environ.get("SCENT_BROKER_PASSWORD") or getpass.getpass("Broker password: ")
        return {**payload, "password": password}
    return payload


async def _run(entry, log: CaptureLog, command: str, action: str | None, payload: dict | None, listen: float) -> None:
    async with DiffuserLink(entry, log) as link:
        if command == "info":
            print(json.dumps(asdict(await link.read_info()), indent=2))
        if command == "status":
            info = await link.read_info()
            print(json.dumps(status_report(entry.alias, info, await link.read_status())))
        if action:
            await link.send(action, payload)
        if listen:
            await link.listen(listen)


STATUS_FIELDS = ("power", "intensity", "charging", "battery")


def status_report(alias: str, info: DeviceInfo, status: dict) -> dict:
    return {
        "alias": alias,
        "deviceId": info.device_id,
        "info": asdict(info),
        "status": {k: status[k] for k in STATUS_FIELDS if k in status},
    }


def _make_test_schedule(args: argparse.Namespace) -> int:
    factory = json.loads(args.factory.read_text()) if args.factory else DEFAULT_FACTORY
    try:
        entry = trial_schedule_entry(dt.datetime.now(), args.start_in, args.duration, args.level)
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    offset = dt.datetime.now().astimezone().utcoffset().total_seconds() / 3600
    print(json.dumps({"profile": {"schedule": [factory, entry]}}, indent=2))
    print(f"test entry {entry['startTime']}-{entry['endTime']}; this Mac's UTC offset is {offset:g} (use --timezone {offset:g})", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
