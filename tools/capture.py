"""Append-only JSONL log of everything sent to and received from a diffuser.

Secrets are masked before anything is written, in both directions.
"""

import datetime as dt
import json
from pathlib import Path

from scentiment_ble import mask_secrets, redact_frame


class CaptureLog:
    def __init__(self, path: Path) -> None:
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)

    def tx(self, frame: bytes, channel: str) -> None:
        self._write({"dir": "tx", "channel": channel, "text": redact_frame(frame)})

    def rx(self, text: str, parsed: dict, channel: str) -> None:
        self._write({"dir": "rx", "channel": channel, "text": _redact_text(text), "parsed": mask_secrets(parsed)})

    def note(self, message: str) -> None:
        self._write({"dir": "note", "channel": "", "text": message})

    def _write(self, record: dict) -> None:
        record = {"ts": dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds"), **record}
        with self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")


def _redact_text(text: str) -> str:
    try:
        obj = json.loads(text)
    except ValueError:
        return text
    return json.dumps(mask_secrets(obj), separators=(",", ":"), ensure_ascii=False)
