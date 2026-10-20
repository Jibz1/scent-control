"""Bluetooth connection to one Scentiment diffuser, built on bleak."""

import asyncio
from typing import Callable

from bleak import BleakClient, BleakScanner
from bleak.exc import BleakError

from capture import CaptureLog, redact_text
from devices import DeviceEntry
from safety import check_allowed
from scentiment_ble import (
    INFO_UUID,
    NOTIFY_UUID,
    WRITE_UUID,
    DeviceInfo,
    chunk,
    encode_command,
    parse_info,
    parse_notification,
    redact_frame,
)

CONNECT_HINT = (
    "Close the Scentiment app or turn the phone's Bluetooth off (a diffuser accepts one "
    "connection at a time), and check the diffuser is powered on and within a few meters."
)


class ConnectError(Exception):
    pass


async def connect_client(entry: DeviceEntry, timeout: float = 20.0) -> BleakClient:
    device = await BleakScanner.find_device_by_address(entry.address, timeout=timeout)
    if device is None:
        raise ConnectError(f"{entry.alias} ({entry.address}) was not found. {CONNECT_HINT}")
    client = BleakClient(device, timeout=timeout)
    try:
        await client.connect()
    except (BleakError, asyncio.TimeoutError, OSError) as err:
        raise ConnectError(f"could not connect to {entry.alias}: {err}. {CONNECT_HINT}") from err
    return client


class DiffuserLink:
    """Connects on enter, subscribes to notifications, and disconnects on exit."""

    def __init__(self, entry: DeviceEntry, log: CaptureLog, timeout: float = 20.0) -> None:
        self.entry = entry
        self.log = log
        self.timeout = timeout
        self.messages: list[dict] = []
        self._on_message: Callable[[dict], None] | None = None
        self.client: BleakClient | None = None

    async def __aenter__(self) -> "DiffuserLink":
        self.client = await connect_client(self.entry, self.timeout)
        self.log.note(f"connected to {self.entry.alias}, mtu {self.client.mtu_size}")
        print(f"connected to {self.entry.alias} (mtu {self.client.mtu_size})")
        await self.client.start_notify(NOTIFY_UUID, self._handle_notification)
        return self

    async def __aexit__(self, *exc: object) -> None:
        if self.client and self.client.is_connected:
            await self.client.disconnect()
        self.log.note(f"disconnected from {self.entry.alias}")

    async def read_info(self) -> DeviceInfo:
        text = bytes(await self.client.read_gatt_char(INFO_UUID)).decode("utf-8", errors="replace")
        self.log.rx(text, {"kind": "info"}, "ble-info")
        return parse_info(text)

    async def send(self, action: str, payload: dict | None = None) -> None:
        check_allowed(action, payload)
        frame = encode_command(action, payload)
        for part in chunk(frame, self.client.mtu_size):
            await self.client.write_gatt_char(WRITE_UUID, part, response=True)
        self.log.tx(frame, "ble")
        print(f"→ {redact_frame(frame)}")

    async def listen(self, seconds: float, on_message: Callable[[dict], None] | None = None) -> list[dict]:
        self._on_message = on_message
        await asyncio.sleep(seconds)
        self._on_message = None
        return list(self.messages)

    def _handle_notification(self, _characteristic: object, data: bytearray) -> None:
        parsed = parse_notification(bytes(data))
        self.messages.append(parsed)
        self.log.rx(str(parsed["raw"]), parsed, "ble-notify")
        print(f"← [{parsed['kind']}] {redact_text(str(parsed['raw']))}")
        if self._on_message:
            self._on_message(parsed)
