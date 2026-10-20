"""Append-only JSONL log of everything sent to and received from a diffuser.

Secrets are masked before anything is written, in both directions.
"""

import datetime as dt
import json
import os
from pathlib import Path

from scentiment_ble import mask_secrets, redact_frame

def captures_dir() -> Path:
    """Where raw output goes: $SCENT_CAPTURES_DIR, else the repo's gitignored captures/."""
    override = os.environ.get("SCENT_CAPTURES_DIR")
    return Path(override) if override else Path(__file__).resolve().parent.parent / "captures"


def stamp() -> str:
    """Local timestamp for capture file names."""
    return dt.datetime.now().strftime("%Y%m%dT%H%M%S")


class CaptureLog:
    def __init__(self, path: Path) -> None:
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)

    def tx(self, frame: bytes, channel: str) -> None:
        self._write({"dir": "tx", "channel": channel, "text": redact_frame(frame)})

    def rx(self, text: str, parsed: dict, channel: str) -> None:
        self._write({"dir": "rx", "channel": channel, "text": redact_text(text), "parsed": mask_secrets(parsed)})

    def note(self, message: str) -> None:
        self._write({"dir": "note", "channel": "", "text": message})

    def _write(self, record: dict) -> None:
        record = {"ts": dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds"), **record}
        with self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")


def redact_text(text: str) -> str:
    """Mask secrets in text that parses as JSON; return other text unchanged."""
    try:
        obj = json.loads(text)
    except ValueError:
        return text
    return json.dumps(mask_secrets(obj), separators=(",", ":"), ensure_ascii=False)
