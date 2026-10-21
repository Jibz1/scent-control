"""List nearby Scentiment diffusers (or every Bluetooth device with --all)."""

import argparse
import asyncio
import json
import sys

from bleak import BleakScanner

from capture import captures_dir, stamp
from scentiment_ble import is_scentiment_name, parse_name


async def scan(timeout: float, show_all: bool) -> list[dict]:
    found = await BleakScanner.discover(timeout=timeout, return_adv=True)
    rows = []
    for address, (device, adv) in found.items():
        name = adv.local_name or device.name
        if not show_all and not is_scentiment_name(name):
            continue
        parsed = parse_name(name) if name else None
        rows.append(
            {
                "name": name,
                "address": address,
                "rssi": adv.rssi,
                "model_code": parsed.model_code if parsed else None,
                "model": parsed.model if parsed else None,
                "variant": parsed.variant if parsed else None,
                "service_uuids": adv.service_uuids,
                "manufacturer_data": {str(k): v.hex() for k, v in adv.manufacturer_data.items()},
                "service_data": {k: v.hex() for k, v in adv.service_data.items()},
                "tx_power": adv.tx_power,
            }
        )
    return sorted(rows, key=lambda r: r["rssi"], reverse=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=10.0, help="seconds to scan (default 10)")
    parser.add_argument("--all", action="store_true", help="show every device, not only Scentiment ones")
    args = parser.parse_args(argv)

    rows = asyncio.run(scan(args.timeout, args.all))
    for r in rows:
        model = f"{r['model'] or '?'} ({r['model_code'] or '-'}), {r['variant'] or '?'}"
        print(f"{r['rssi']:>5} dBm  {r['address']}  {r['name']!s:<32}  {model}")
        if r["service_uuids"] or r["manufacturer_data"]:
            print(f"         services {r['service_uuids']}  manufacturer {r['manufacturer_data']}")
    out = captures_dir() / f"scan-{stamp()}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(rows, indent=2))
    print(f"{len(rows)} device(s); saved {out}")
    if not rows:
        print("No Scentiment devices seen. Try --all, check they're powered on, and turn the phone's Bluetooth off.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
