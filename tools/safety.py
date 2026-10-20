"""Allow-list of commands the Phase 0 tools may send to a diffuser.

Anything not listed is refused before a connection is made. Firmware updates and
hard or factory resets are never allowed (spec §4: never brick a diffuser).
"""

ALLOWED_ACTIONS = {
    "TURN_ON",
    "SET_LEVEL",
    "SET_RGB_LEVEL",
    "SET_RGB_LED",
    "PING",
    "WIFI_CONNECT",
    "SAVE_MQTT",
    "SYNC_TIME",
    "UPDATE_PROFILE",
    "INITIAL_LOAD",
    "GET_STATUS",
}
EXACT_PAYLOADS = {
    "RESET": {"mode": "soft"},
    "SECRET_COMMAND": {"command": "restart"},
}


class RefusedCommand(Exception):
    pass


def check_allowed(action: str, payload: dict | None) -> None:
    if action in EXACT_PAYLOADS:
        if payload != EXACT_PAYLOADS[action]:
            raise RefusedCommand(f"{action} is only allowed with payload {EXACT_PAYLOADS[action]}")
        return
    if action in {"UPDATE_FIRMWARE", "U_FW"}:
        raise RefusedCommand(f"{action} is never sent: a bad firmware update could brick the diffuser")
    if action not in ALLOWED_ACTIONS:
        raise RefusedCommand(f"{action} is not on the allow-list ({', '.join(sorted(ALLOWED_ACTIONS | EXACT_PAYLOADS.keys()))})")
