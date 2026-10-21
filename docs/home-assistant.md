# Home Assistant adapter

`apps/home-assistant` is an initial, headless MQTT adapter for the Air 2 and HVAC 2.
It reuses `scent-core` for the device protocol and exposes each configured diffuser
as a Home Assistant fan: on/off and three intensity levels. It has no Bluetooth or
desktop dependency and can run on an always-on Mac or Linux machine.

```mermaid
flowchart LR
    home[Apple Home / Siri] <-->|HomeKit Bridge| ha[Home Assistant]
    ha <--> local[HA MQTT broker]
    local <--> adapter[Scent Control adapter]
    adapter <-->|TLS| broker[Our diffuser MQTT broker]
    broker <-->|wss| diffuser[Diffuser]
```

The two broker connections are independent: HA keeps using its existing broker.
The adapter does not move diffusers to another broker or change their WiFi setup.
Using the Railway broker still requires internet access. No Scentiment service is
used. A future API server will replace the adapter's direct device-command path;
this first version is for the current desktop-and-broker phase.

## Setup

1. Set up the diffuser through Scent Control's desktop app first.
2. Enable HA's [MQTT integration](https://www.home-assistant.io/integrations/mqtt/)
   and keep its default discovery prefix, `homeassistant`. The adapter also listens
   for HA's default birth message, `homeassistant/status` with payload `online`.
3. Copy the example to a gitignored local configuration at the repository root:

   ```sh
   cp apps/home-assistant/config.example.json home-assistant.local.json
   ```

4. Edit the broker addresses, usernames, and devices in that local file. Use the
   actual device ID shown by Scent Control. Models are `air2` and `hvac2`. Names and
   IDs belong only in local config, never in committed examples. `route_id` **must
   match the route already used by the desktop app and stored on the diffuser**:
   `PING` stores this value on the device. Do not invent a new one for the adapter.
5. Choose a stable `instance_id`, unique to this home/adapter. Run only one instance
   with that ID: its MQTT client IDs, discovery IDs and topics depend on it. Changing
   it creates different HA entities. Up to 32 devices can be configured.
6. Build and validate without connecting to anything:

   ```sh
   cargo build -p scent-home-assistant
   target/debug/scent-home-assistant --check home-assistant.local.json
   ```

7. Supply broker passwords through environment variables. This interactive launcher
   prompts without echoing or putting passwords in shell history or arguments:

   ```sh
   python3 - <<'PY'
   import getpass
   import os

   os.environ["SCENT_MQTT_PASSWORD"] = getpass.getpass("Diffuser broker password: ")
   os.environ["HA_MQTT_PASSWORD"] = getpass.getpass("HA broker password: ")
   os.execv("target/debug/scent-home-assistant", [
       "scent-home-assistant", "home-assistant.local.json"
   ])
   PY
   ```

   For unattended use, have your service manager supply the same variables from its
   secret storage. Run the binary continuously; the desktop app need not be open.

The diffuser broker requires `wss` or `mqtts` with a certificate trusted by the OS.
The HA broker can use either TLS transport, or `tcp` with explicit `allow_insecure:
true` on a trusted local network. TCP to the diffuser broker is allowed only on
`localhost` or `127.0.0.1` for testing. `--check` validates the config structure; it
does not test credentials, certificates, access permissions or device reachability.
Passwords cannot be embedded in the JSON or broker addresses. Errors and logs never
include raw config, message bodies, passwords or device identifiers.

## Broker permissions and topics

Give the adapter its own credentials on each broker. Never give HA the diffuser's
login or use a diffuser client ID for the adapter. Its minimum permissions are:

| Broker | Read / subscribe | Write / publish |
|---|---|---|
| Diffuser broker | `scentiment-mqtt-prod/user/<route_id>`, `scentiment-mqtt-prod/server-notifications/device` | `scentiment-mqtt-prod/device/<id>` for each configured device |
| HA broker | `scent-control/<instance>/<id>/power/set`, `scent-control/<instance>/<id>/level/set`, `homeassistant/status` | Discovery topics below, `scent-control/<instance>/availability`, each device's state and availability topics |

On the HA broker, each device uses:

| Topic suffix under `scent-control/<instance>/<id>/` | Payload |
|---|---|
| `power/set` | `ON` or `OFF` |
| `level/set` | `0` (off), `1`, `2`, or `3` |
| `power/state` | Reported `ON`, `OFF`, or `None` (unknown) |
| `level/state` | Reported level `0`–`3`, or `None` (unknown) |
| `availability` | `online` or `offline` |

Discovery is retained at
`homeassistant/fan/scent_control_<instance>_<id>/config`. Home Assistant's
[MQTT fan](https://www.home-assistant.io/integrations/fan.mqtt/) maps its percentage
slider to the configured three speeds; it sends 1–3 to this adapter, not 33–100.
Setting a nonzero speed also turns the diffuser on. Turning on uses its latest
fresh reported intensity, or Low when no valid reading is available.

Commands are unretained, QoS 0, and best effort. Offline, retained-replay, malformed,
and unconfigured-device commands are discarded. MQTT 3.1.1 does not mark a retained
write forwarded to an already subscribed client as retained, so publishers must
always use `retain: false`; the adapter rejects retained messages replayed on
subscription. Do not bridge command topics with persistent/offline queues.

Discovery, reported state and availability are retained. After removing a device
from config, clear its discovery topic with an empty retained message using HA's
MQTT publish UI, then remove its old state/availability retained topics if desired.
Changing the instance ID requires cleaning up the old discovery topics too.

## State and outages

- Both subscriptions must be accepted before commands can be forwarded. Failed
  connections or subscriptions close both sessions and retry after 5 seconds with
  empty queues and unknown state. No manual command is restored or retried.
- A retained last will marks the bridge offline when its HA broker connection
  closes. With a lost network, broker keepalive detection takes additional time.
- The adapter pings every 30 seconds with the existing route ID. A diffuser is
  available only when heard from within the last 75 seconds. A broker connection
  or echoed command alone is not evidence that a diffuser is reachable.
- Power and intensity are independently reset to unknown after 5 minutes without
  a report of that field. Ping replies refresh reachability, not readings. Sending
  a command never updates reported state. Firmware 2.30 often does not acknowledge
  intensity, so a slider may remain at its old value until a later report.
- A device reconnect invalidates old readings; it does not imply a user change.
  Restarting HA republishes discovery and current reported state.

## Apple Home and scope

After the fans appear in HA, add its built-in
[HomeKit Bridge](https://www.home-assistant.io/integrations/homekit/) integration
and include those fan entities. Pair the bridge in Apple Home. No custom HomeKit
or Matter implementation is needed for that path. HA's official Matter integration
does not export ordinary HA entities as Matter accessories.

This adapter does not configure devices, edit schedules, expose oil/LED controls,
or provide the planned server's accounts and desired-state reconciler. The existing
on-device schedules continue running. A manual command has the firmware's existing
interaction with those schedules; a timed override/resume policy is not implemented.
HA automations require HA and the adapter to be running.

Changes reported over MQTT, including reports caused by another controller, are
reflected in HA. Bluetooth-only changes that firmware does not publish over MQTT
cannot be inferred. Sharing desired state with the desktop's Bluetooth path belongs
to the later server integration. This adapter never repeatedly enforces an old
requested power or intensity.

## Validation

```sh
bunx turbo run build test lint fmt:check --filter=@scent-control/home-assistant
```

Fixture tests use the sanitized Air 2/HVAC captures in `fixtures/scentiment`.
Socket tests run the adapter against two loopback MQTT peers and cover discovery,
strict command translation, retained-command rejection, HA birth, subscription
refusal and reconnect behavior. They do not connect to Railway or real devices.

Before calling this ready for everyday use, verify on a real HA instance and both
diffusers: discovery/UI behavior, HomeKit pairing, manual control while schedules
are active, physical-button/Bluetooth reports, and broker/adapter/HA restarts.
Record any newly observed device behavior in `docs/findings.md` with date and source.
