"""Dump a diffuser's GATT table and read every readable characteristic. Sends nothing."""

import argparse
import asyncio
import json
import sys
from dataclasses import asdict

from ble_client import ConnectError, connect_client
from capture import CAPTURES_DIR, stamp
from devices import DeviceEntry, resolve
from scentiment_ble import INFO_UUID, parse_info


async def dump(entry: DeviceEntry) -> dict:
    client = await connect_client(entry)
    try:
        services = []
        info = None
        for service in client.services:
            characteristics = []
            for ch in service.characteristics:
                item = {
                    "uuid": ch.uuid,
                    "handle": ch.handle,
                    "properties": ch.properties,
                    "descriptors": [{"uuid": d.uuid, "handle": d.handle} for d in ch.descriptors],
                }
                if "read" in ch.properties:
                    try:
                        value = bytes(await client.read_gatt_char(ch))
                        item["value_hex"] = value.hex()
                        item["value_utf8"] = value.decode("utf-8", errors="replace")
                        if ch.uuid == INFO_UUID:
                            try:
                                info = asdict(parse_info(item["value_utf8"]))
                            except ValueError as err:
                                info = {"error": str(err)}
                    except Exception as err:  # a failed read is a finding, not a crash
                        item["read_error"] = str(err)
                characteristics.append(item)
            services.append({"uuid": service.uuid, "description": service.description, "characteristics": characteristics})
        return {"alias": entry.alias, "address": entry.address, "mtu": client.mtu_size, "info": info, "services": services}
    finally:
        await client.disconnect()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("device", help="alias from devices.toml, or an address")
    args = parser.parse_args(argv)
    try:
        entry = resolve(args.device)
        result = asyncio.run(dump(entry))
    except (KeyError, FileNotFoundError, ValueError, ConnectError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1

    for service in result["services"]:
        print(f"service {service['uuid']}  {service['description']}")
        for ch in service["characteristics"]:
            value = f"  = {ch['value_utf8']!r}" if "value_utf8" in ch else ""
            print(f"  char {ch['uuid']}  {','.join(ch['properties'])}{value}")
            for d in ch["descriptors"]:
                print(f"    desc {d['uuid']}")
    print(f"mtu {result['mtu']}  info {result['info']}")
    out = CAPTURES_DIR / f"gatt-{entry.alias}-{stamp()}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2))
    print(f"saved {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
