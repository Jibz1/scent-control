"""Device aliases from tools/devices.toml (gitignored), so addresses are never hardcoded."""

import tomllib
from dataclasses import dataclass
from pathlib import Path

DEFAULT_CONFIG = Path(__file__).parent / "devices.toml"


@dataclass(frozen=True)
class DeviceEntry:
    alias: str
    address: str
    name: str | None


def load_devices(path: Path) -> dict[str, DeviceEntry]:
    if not path.exists():
        raise FileNotFoundError(
            f"{path} not found: copy devices.example.toml to devices.toml and fill in the addresses scan.py prints"
        )
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    devices: dict[str, DeviceEntry] = {}
    for alias, entry in data.get("devices", {}).items():
        address = entry.get("address") if isinstance(entry, dict) else None
        if not isinstance(address, str) or not address:
            raise ValueError(f"device {alias!r} in {path} needs an address (a string from scan.py)")
        name = entry.get("name")
        devices[alias] = DeviceEntry(alias, address, name if isinstance(name, str) else None)
    return devices


def resolve(alias_or_address: str, path: Path = DEFAULT_CONFIG) -> DeviceEntry:
    looks_like_address = "-" in alias_or_address or ":" in alias_or_address
    if looks_like_address and not path.exists():
        return DeviceEntry(alias_or_address, alias_or_address, None)
    devices = load_devices(path)
    if alias_or_address in devices:
        return devices[alias_or_address]
    if looks_like_address:
        return DeviceEntry(alias_or_address, alias_or_address, None)
    known = ", ".join(sorted(devices)) or "none"
    raise KeyError(f"unknown device alias {alias_or_address!r}; known aliases: {known}")
