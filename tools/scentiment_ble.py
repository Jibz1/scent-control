"""Pure helpers for the Scentiment Bluetooth protocol (no Bluetooth access here).

Protocol facts and their sources are in docs/findings.md.
"""

import datetime as dt
import json
from dataclasses import dataclass

SERVICE_UUID = "00000180-0000-1000-8000-00805f9b34fb"
WRITE_UUID = "0000dead-0000-1000-8000-00805f9b34fb"
NOTIFY_UUID = "0000fef3-0000-1000-8000-00805f9b34fb"
INFO_UUID = "0000fef4-0000-1000-8000-00805f9b34fb"
TERMINATOR = b"-|"

MODEL_CODES = {
    "D": "Diffuser Air 2",
    "H": "Hvac",
    "P": "Pro",
    "M": "Mini",
    "T": "Mini 2 LE",
    "W": "The Plug-In",
    "R": "Diffuser_4",
}
VARIANT_CODES = {"B": "Black", "C": "Graphite Gold", "G": "Grey", "W": "White", "S": "Silver", "R": "Red"}

# Indices 4-13 of the status array; 0-3 are a header the app ignores.
STATUS_FIELDS = ("power", "charging", "battery", "oil", "fan", "intensity", "hours", "gas_pump", "light", "factory_restore")
STATUS_TEXT_KEYS = {"s": "power", "l": "intensity", "c": "charging", "bat": "battery"}
VALUE_KEYS = {"enable": "power", "level": "intensity", "battery": "battery"}
SECRET_KEYS = {"password", "pass", "token", "secret"}

MIN_CHUNK = 20


def encode_command(action: str, payload: dict | None = None) -> bytes:
    obj: dict = {"action": action}
    if payload is not None:
        obj["payload"] = payload
    return _compact(obj).encode("utf-8") + TERMINATOR


def chunk(frame: bytes, mtu: int | None) -> list[bytes]:
    size = mtu - 3 if mtu and mtu > 23 else MIN_CHUNK
    return [frame[i : i + size] for i in range(0, len(frame), size)]


@dataclass(frozen=True)
class DeviceInfo:
    manufacturer_id: str
    model_name: str
    variant_name: str
    device_id: str
    firmware_version: str
    wifi_configured: bool
    wifi_status: int


def parse_info(text: str) -> DeviceInfo:
    fields = [f.strip() for f in text.strip().split("|")]
    if len(fields) < 7:
        raise ValueError(f"expected 7 '|'-separated fields in the 0xFEF4 info string, got {len(fields)}: {text!r}")
    manufacturer, model, variant, device_id, firmware, configured, status = fields[:7]
    return DeviceInfo(manufacturer, model, variant, device_id, firmware, configured.lower() == "true", int(status))


@dataclass(frozen=True)
class NameInfo:
    model_code: str | None
    model: str | None
    variant_code: str | None
    variant: str | None


def is_scentiment_name(name: str | None) -> bool:
    return name is not None and "scentiment" in name.lower()


def parse_name(name: str) -> NameInfo:
    """Read model and variant codes from `<prefix>-<ModelCode> <VariantCode> [suffix]`."""
    _, dash, rest = name.partition("-")
    tokens = rest.split() if dash else []
    model_code = tokens[0] if tokens else None
    variant_code = tokens[1] if len(tokens) > 1 else None
    return NameInfo(model_code, MODEL_CODES.get(model_code or ""), variant_code, VARIANT_CODES.get(variant_code or ""))


def parse_notification(data: bytes) -> dict[str, object]:
    text = data.decode("utf-8", errors="replace").strip()
    if text.endswith("-|"):
        text = text[:-2].strip()
    result: dict[str, object] = {"raw": text}

    if text.startswith(("{", "[")):
        try:
            obj = json.loads(text)
        except ValueError:
            obj = None
        if isinstance(obj, dict) and isinstance(obj.get("data"), list):
            array = obj["data"]
            result["kind"] = "status_array"
            result["array"] = array
            for index, field in enumerate(STATUS_FIELDS, start=4):
                if index < len(array):
                    result[field] = array[index]
            return result
        if obj is not None:
            result["kind"] = "json"
            result["json"] = obj
            return result

    parts = [p.partition(":") for p in text.split(",")]
    if len(parts) > 1 and all(sep and key.strip().lower() in STATUS_TEXT_KEYS for key, sep, _ in parts):
        result["kind"] = "status_text"
        for key, _, value in parts:
            result[STATUS_TEXT_KEYS[key.strip().lower()]] = _number(value)
        return result

    key, sep, value = text.partition(":")
    if sep and key.strip():
        key = key.strip().lower()
        result["kind"] = "value"
        result[VALUE_KEYS.get(key, key)] = _number(value)
        return result

    result["kind"] = "unknown"
    return result


def build_profile(entries: list[dict], timezone_hours: float, schedule_flag: int) -> dict:
    return {"profile": {"schedule": entries}, "timezone": timezone_hours, "schedule_flag": schedule_flag}


def scentiment_weekday(day: dt.date) -> int:
    """0 = Sunday ... 6 = Saturday."""
    return (day.weekday() + 1) % 7


def trial_schedule_entry(now: dt.datetime, start_in_min: int, duration_min: int, level: int) -> dict:
    start = (now + dt.timedelta(minutes=start_in_min)).replace(second=0, microsecond=0)
    end = start + dt.timedelta(minutes=duration_min)
    if start.date() != now.date() or end.date() != now.date():
        raise ValueError("the test schedule would cross midnight; pick a start time that ends before 23:59")
    return {
        "startTime": start.strftime("%H:%M"),
        "endTime": end.strftime("%H:%M"),
        "weekdays": [scentiment_weekday(start.date())],
        "workingState": {"level": level, "fan": 0, "led": 0, "timer": 0},
        "enable": True,
    }


def redact_frame(frame: bytes) -> str:
    """Return a loggable copy of a command frame with secret payload values masked."""
    try:
        text = frame.decode("utf-8")
        if text.endswith("-|"):
            text = text[:-2]
        obj = json.loads(text)
    except ValueError:
        return f"<unparsable {len(frame)} bytes>"
    return _compact(mask_secrets(obj)) + "-|"


def mask_secrets(value: object) -> object:
    """Copy of a JSON value with every secret-named key's value replaced by \"***\"."""
    if isinstance(value, dict):
        return {k: "***" if k.lower() in SECRET_KEYS else mask_secrets(v) for k, v in value.items()}
    if isinstance(value, list):
        return [mask_secrets(v) for v in value]
    return value


def _number(value: str) -> object:
    value = value.strip()
    try:
        return int(value)
    except ValueError:
        return value


def _compact(obj: object) -> str:
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False)
