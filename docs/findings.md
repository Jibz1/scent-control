# Findings

Every device behavior we learn goes here with a date and a source tag:

- **[observed]**: seen on our own diffusers
- **[prior art]**: reported in Scent Assistant's code or issues
- **[web]**: found online (manuals, FCC filings, forums)

Sections tagged **[observed]** come from our own diffusers. Everything else is a lead
until an observation confirms it.

## Phase 0 results (2026-10-20) [observed]

Run on our Air 2 and HVAC 2 (both firmware 2.30) with the Phase 0 tools and test bench.
**Gate: go.** `SAVE_MQTT` over Bluetooth works on both units, and schedules run on
the device itself, starting and stopping on time.

| Design assumption | Result |
|---|---|
| Bluetooth control | **Partly confirmed.** `TURN_ON` and `SET_LEVEL` work and are acknowledged (`enable:N`, `level:N`, then a status array). `SET_RGB_LEVEL`/`SET_RGB_LED` get no reply and have no visible effect; the main light follows the diffuser's own state (green, white while intensity changes). |
| `SAVE_MQTT` over Bluetooth | **Confirmed on both.** The device makes a test login within ~20 s and then disconnects. After a soft restart it connects to the new broker (keepalive 1200 s, no will message). |
| `WIFI_CONNECT` needed | **Not for our units.** Both were still on WiFi from their original setup, so `SAVE_MQTT` plus a restart was enough. |
| Command order | Only `SAVE_MQTT` then soft restart was used, so the device never needed to reach Scentiment's broker during the move. |
| URL form | `wss://<railway-domain>/mqtt` with Railway's public certificate worked first time. |
| Topics | The device subscribes to `scentiment-mqtt-prod/device/<deviceId>` **and** `scentiment-mqtt-prod/announcements` (QoS 0). It publishes `device_connected` to `scentiment-mqtt-prod/server-notifications/device` before it knows a user ID, and to `scentiment-mqtt-prod/user/<userId>` after. Client ID is `ESP32_` + the last 6 characters of the device ID. |
| `PING`/`userid` persistence | **Confirmed.** The first `PING {"userid"}` replies `ADDED`, later ones `ALREADY_ADDED`. The user ID survives a soft restart: the device announces itself on `user/<userId>` without a new `PING`. A `PING` over Bluetooth also stores it. |
| Control over WiFi | `TURN_ON` works and is acknowledged. `SET_LEVEL` and `UPDATE_PROFILE` (`schedule_flag 0`, the always-on entry) work **without** any reply. Light commands have no effect. `GET_STATUS` answers only sometimes. `SYNC_TIME` never replies. WiFi commands seemed held while a Bluetooth session was open, and replies can lag ~14 s. |
| On-device schedules | **Confirmed on the Air 2** (10:24 start; 11:03–11:06 start and stop), with no phone or Bluetooth involved. **The order matters:** `UPDATE_PROFILE` (`schedule_flag 1`) then `SYNC_TIME` works, as in Scentiment's app. `SYNC_TIME` first, then the schedule, did not run. |
| Schedule over Bluetooth | **Unknown.** `SYNC_TIME` then `UPDATE_PROFILE` were sent over BLE for a 09:31–09:35 window, but nobody observed the result, and that order also failed over WiFi. Retest with the working order (`UPDATE_PROFILE`, then `SYNC_TIME`). |
| Power cut | Not tested as a true power cut: the Air 2 **has a battery** and kept running unplugged. A soft restart keeps the broker, the user ID and WiFi. |
| Button / touchscreen reporting | Not tested yet (Phase 2). |
| Status reporting | The device reports **only on a change**. `0xFEF3` holds the last value it sent (e.g. `led:50`), not a live status. `PING`, `GET_STATUS` over Bluetooth, and re-sending the same intensity trigger nothing. Unplugging the Air 2 sent no power-source change. |
| Online detection | With keepalive 1200 s and no will message, the broker can take ~30 min to notice a dead device, so **online status needs active pings**. The bench pings every 30 s, and answers come within ~1 s. |
| Oil level | **Not available.** Status index 7 (Scentiment's app's "oil") flips between 50 and 100 at constant intensity, so it looks like the spray/pause phase. The HVAC has no oil reading at all. |
| Status frame | 16 bytes: `55 AA 0B FF`, fields at 4–13, byte 14 a checksum, byte 15 `0x6A`. On the Air 2, `sum(bytes 0..14) & 0xFF == 0x64` in 7 of 7 frames. On the HVAC that rule fails (one sample), so it differs per model. |
| JSON quirks | `notification` sends `"data":enable:0` without quotes, which is invalid JSON. `update_status` over MQTT carries the array as an object, `{"data":[...]}`, not a string. |
| HVAC 2 protocol | **Same as the Air 2:** same GATT layout, info and status formats, and move to our broker. Model `H`, manufacturer `03`. Its status array uses fan, gas pump and light fields. |
| macOS Bluetooth | A process may use Bluetooth only if its responsible app declares `NSBluetoothAlwaysUsageDescription`. Tools started by Claude need the `ScentTools.app` wrapper, and that wrapper can't read `~/Documents`. |
| Coexisting with Scentiment's app | A diffuser stores **one** broker address. While it points at ours, Scentiment's cloud can't reach it over WiFi. Bluetooth works for both apps, one connection at a time. Each app sends the whole schedule table, so mixing them on one diffuser conflicts. |

## Our devices (user-reported, 2026-10-20)

- **Scentiment Diffuser Air 2**
- **Scentiment Diffuser HVAC 2** (touchscreen, 500 ml). This is **not** the Scent
  Diffuser Pro 2, so the Pro 2 hardware notes below don't apply to it. No FCC filing for
  the HVAC 2 has turned up, so nothing public is known about its chips or its Bluetooth
  protocol. Scanning showed it speaks the same protocol as the Air 2 (see Phase 0 results).

## 2026-10-20: Read-only survey of our two diffusers [observed]

Tools: `tools/scan.py`, `gatt_dump.py` and `session.py info/listen`, run from a Mac
(macOS 27) about 3–5 m from both units, with the phone's Bluetooth off. Nothing was
sent to either device. Real device IDs are replaced below with `TESTDEVICE01` (Air 2)
and `TESTDEVICE02` (HVAC 2).

| | Diffuser Air 2 | Diffuser HVAC 2 |
|---|---|---|
| Advertised name | `Scentiment-D B n <deviceId>` | `Scentiment-H B n <deviceId>` |
| Model / variant code | `D` / `B` (Black) | `H` / `B` (Black) |
| RSSI at the Mac | −71 dBm | −60 dBm |
| Advertisement | name only: no service UUIDs, manufacturer data or service data | same |
| GATT | service `0x0180` with `0xFEF4` (read), `0xFEF3` (read, notify, CCCD), `0xDEAD` (write) | **identical** |
| MTU (macOS) | 256 | 256 |
| `0xFEF4` info | `01\|Diffuser\|Black\|TESTDEVICE01\|2.30\|true\|3` | `03\|Hvac\|Black\|TESTDEVICE02\|2.30\|true\|2` |
| Firmware | 2.30 | 2.30 |
| WiFi configured / status | true / 3 | true / 2 |
| `0xFEF3` read | `S:0,L:0,C:2,Bat:200` | `S:0,L:1,C:2,Bat:200` |
| Unprompted notifications in 30 s after connecting | none | one: `S:0,L:1,C:2,Bat:200` right after connect |

What this tells us:

- **The HVAC 2 speaks the same Bluetooth protocol as the Air 2**: same GATT layout, info
  format and status text. The manufacturer ID differs (`03` vs `01`), and the model
  name in `0xFEF4` is `Hvac`, matching Scentiment's app's `H` code.
- **`deviceId` is 12 hex digits and matches the last token of the advertised name**
  (it looks like a MAC address). The third name token is `n` on both. Its meaning is
  unknown.
- **Both units are still on a WiFi network from their original setup**
  (`wifi_configured` true, status 2 and 3). On firmware > 2.0, Scentiment's app treats
  2 or 3 as connected. What 2 versus 3 means (for example, WiFi only versus WiFi plus
  broker) is unknown. Until `SAVE_MQTT` moves them, they are presumably trying to reach
  Scentiment's broker.
- **Firmware 2.30 on both:** `SYNC_TIME` applies (> 1.3.8), up to 100 schedule entries
  (> 2.21), and the `wifiStatus` 2/3 rule applies (> 2.0).
- **`0xFEF3` can be read directly.** It returns the status text, so a state read
  doesn't depend on the device choosing to send a notification. The Air 2 sent no
  notification on connect, unlike the report in Scent Assistant's issue #42.
- Both report `C:2,Bat:200`, which is "on external power".

## 2026-10-19: Scent Assistant (prior art)

Source: https://github.com/mr-sparks/scent-assistant at commit `9a5fc05` (2026-10-09),
mainly `custom_components/scent_assistant/protocol_ble.py` (`ScentimentProtocol`) and
`const.py`, plus issues #7 (first Air 2 capture) and #42 (Mini 2 support, oil level).

### Models

- **Diffuser Air 2**: confirmed over BLE. It also has WiFi: issue #7 lists separate WiFi
  and Bluetooth MAC addresses.
- **Mini 2 (DA37)**: same protocol as the Air 2. It also has WiFi: its owner says the
  official app reaches it over WiFi. The integration can't tell the Air 2 and Mini 2
  apart from the advertised name yet (names end in `-D` or `-T`, and which suffix
  means which model is unconfirmed).
- BLE advertised names start with `Scentiment`. The integration finds them by that prefix.

### GATT layout

| UUID (16-bit, SIG base) | Use |
|---|---|
| `0x0180` | Service |
| `0xDEAD` | Write: commands (JSON) |
| `0xFEF3` | Notify: state |
| `0xFEF4` | Read: device metadata (contents not documented) |

Full form: `0000xxxx-0000-1000-8000-00805f9b34fb`.

### Commands (write to `0xDEAD`)

Compact JSON followed by the two bytes `-|` (`0x2D 0x7C`). The device ignores
commands without that terminator.

| Command | Bytes before the terminator |
|---|---|
| Power on / off | `{"action":"TURN_ON","payload":{"on":1}}` / `{"on":0}` |
| Intensity 1–3 | `{"action":"SET_LEVEL","payload":{"intensity":2}}` |
| LED color | `{"action":"SET_RGB_LEVEL","payload":{"red":255,"green":0,"blue":0}}` |
| LED on / off | `{"action":"SET_RGB_LED","payload":{"on":1}}` |
| Ping | `{"action":"PING"}` |

There is no explicit state query. The device pushes state as notifications, and the
integration reads state just by connecting and listening.

### Notifications (from `0xFEF3`)

Three text shapes, sometimes with a trailing `-|`:

- `S:1,L:3,C:1,Bat:100`: `S` = power, `L` = level, `Bat` = battery %. `Bat:200` means
  "full, on external power". `C` is undecoded: `C:2` appeared on a plugged-in unit, so
  it may be charge state (guess). The Air 2 sends this line when a client connects.
  The Mini 2 sometimes never sends it.
- `level:N` and `enable:N` (power) as single values.
- `{"data":[…16 bytes…]}`: a binary frame, undecoded. Example from an Air 2:
  `[85,170,11,255,0,2,200,100,255,0,0,255,255,0,48,106]` (starts `55 AA 0B FF`).
  Byte 7 is **not** oil: it changed from 50 to 100 with no refill. Byte 6 was 200 once,
  next to `Bat:200`, which is a single sample.

### What the BLE protocol doesn't cover (as far as anyone has found)

- **Pairing / PIN**: none. The integration connects and writes without a handshake.
- **Schedules**: no schedule or clock commands. Scent Assistant lists Scentiment as
  "schedule unsupported" and skips the time-sync button for it. Schedules have to be
  driven from outside unless the WiFi side reveals an on-board scheduler.
- **Oil level**: the official app shows the Air 2's oil as bars (issue #42 shows
  "Medium", 2 of 3). Nobody knows where that value comes from. It may be in the binary
  frame, in `0xFEF4`, or sent over WiFi only. The Mini 2 shows no oil reading in the app.
- **WiFi setup**: no command for sending WiFi credentials is known.
- **One connection at a time**: the phone app has to let go of the device before
  anything else can connect.

### Platform clue

The Air 2's WiFi MAC starts `8C:BF:EA`, and its Bluetooth MAC is the WiFi MAC + 2.
That is the standard ESP32 address layout (WiFi station = base MAC, Bluetooth =
base + 2). The OUI is Espressif's (see the 2026-10-19 web entry below).

## 2026-10-19: Online research (web)

Sources read on 2026-10-19. Nothing here contacted Scentiment's app or device servers.
Items marked *verified* were re-checked by hand.

| Key | Source |
|---|---|
| FP2 | FCC ID 2BA6L-SDPRO2, Scent Diffuser Pro 2: https://fccid.io/2BA6L-SDPRO2 (test report `pdf.php?id=8137020`, internal photos `id=8137016`, manual `id=8137034`) |
| FM2 | FCC ID 2BMW9-DA37, Mini 2: https://fccid.io/2BMW9-DA37 (photos `id=7940687`) |
| FLE | FCC ID 2BUVD-AIR2LE, Air 2 LE (battery): https://fccid.io/2BUVD-AIR2LE (photos `id=9088771`) |
| PP2, PA2, HV2 | scentiment.com product pages: `/products/scent-diffuser-pro-2`, `/products/scent-diffuser-2`, `/products/diffuser-hvac-2` |
| MWM | App listing mirror: https://mwm.ai/fr/apps/scentiment-1-0/6478093653 |

### Which model is the "HVAC Pro 2"

- The Pro 2's FCC test report reads: "Series model: SS2000, HVAC Diffuser, Scent
  Diffuser Pro 2" (FP2, *verified*). So the HVAC Pro 2 is the **Scent Diffuser Pro 2**.
- Scentiment also sells a separate **Diffuser HVAC 2** (touchscreen, 500 ml; HV2). The
  label on our unit settles which one we have. FCC ID `2BA6L-SDPRO2` means Pro 2.

### Radios

- All models are sold as WiFi + Bluetooth. WiFi is 2.4 GHz only (802.11b/g/n,
  2412–2462 MHz in the Pro 2, Mini 2 and Air 2 LE grants).
- The Pro 2 ships with a **433 MHz remote** and has a 433 MHz receiver on the board
  (FP2 photos). That's another local control path: capture the remote's codes and replay
  them.

### Hardware: Espressif chip plus a separate microcontroller

- OUI `8C:BF:EA` belongs to Espressif Inc. (https://maclookup.app/macaddress/8CBFEA,
  *verified*). Espressif chips with Bluetooth are ESP32-family (the ESP8266 has none).
- Each board shows an Espressif chip **and a second, separate microcontroller**:
  - **Pro 2** (OEM Guangdong Baixiang): Espressif module with external flash, a
    TSSOP-20 MCU with a `V/G/DAT/CLK` programming header, a 32.768 kHz clock crystal
    and a **CR2032 backup cell**, plus the 433 MHz receiver.
  - **Mini 2** (OEM Shenzhen Siweiyi, board HXY-DA37): Espressif module with
    `TXD/RXD/IO8/IO9/EN` pads, plus a separate programming header and a `TXU/RXU`
    serial header.
  - **Air 2 LE** (OEM "Scent factory", Guangzhou): Espressif chip with
    `TXD/RXD/IO8/IO9/3V3/GND` pads (IO8 and IO9 are ESP32-C3 boot pins, so likely an
    ESP32-C3), plus a separate MCU.
  - Chip part numbers can't be read from the photos.
- So the Espressif chip probably handles WiFi and Bluetooth only, and relays commands
  over serial to the MCU that drives the pump, LEDs and clock (likely). That fits the
  `55 AA` frames.
- Four different OEMs build these models, so the protocol may differ per model.
- Not Tuya: the Air 2 uses custom GATT UUIDs, none of Tuya's (likely).

### The `55 AA` status frame

- The last byte is the XOR of bytes 0–14. In the one published sample, that gives
  `0x6A` (*verified*). One sample is too few to call this settled.

### WiFi setup and cloud

- Undocumented. The Pro 2 manual only says to make an account, tap "Connect My
  Device" and follow the prompts. The units advertise a BLE name containing their MAC
  (issue #7), so setup over Bluetooth is a guess.
- Nothing public on which hosts the devices contact, the protocol, or TLS. No local
  network API is known.

### Schedules

- **Air 2**: setting a schedule in the app produced no BLE traffic (issue #7), so
  schedules travel over WiFi. Whether the unit keeps running a schedule offline is
  unknown.
- **Pro 2**: a clock crystal, a backup cell and an MCU point to an **on-board clock
  and schedule** (likely). Reviews mention units running outside their schedule after it
  was deleted, which hints at a schedule stored on the device (guess).

### Oil level

- The Air 2 app shows 3 bars. The Mini 2 app shows nothing; only LEDs on the unit do.
  The product pages promise only "estimated bottle life". Reviews report false empty
  readings on the Pro 2, so it may be an estimate rather than a sensor (unknown).

### Firmware

- No published OTA mechanism, versions or changelogs. Nobody has published opening a
  unit or flashing ESPHome or Tasmota.
- Serial and boot pads are visible on the Mini 2 and Air 2 LE boards.

### App status

- The US/CA App Store listing (id 6478093653) returned 404 on 2026-10-19 (not checked by
  hand). The listing mirror shows a 1.2/5 rating, version 3.1.14, and complaints about
  login hangs, WiFi setup failures and broken schedules (MWM).
- Only one other open-source project was found: Scent Assistant.

### Still open (as of 2026-10-19)

- How WiFi provisioning works, and which hosts the devices contact.
- The Pro 2's BLE name, GATT layout and protocol. No reports exist anywhere.
- Exact ESP and MCU part numbers. Whether the chips are locked (secure boot / flash
  encryption).
- Whether oil level comes from a sensor.
- FCC filings for the mains-powered Air 2 and the HVAC 2.

## 2026-10-20: The newer "Scentiment 2.0" app (web)

Scentiment replaced its first app ("Scentiment", App Store id 6478093653, Android
`com.scentiment.app`) with a new one.

| | iOS | Android |
|---|---|---|
| Listing | https://apps.apple.com/us/app/scentiment-2-0/id6752409911 | https://play.google.com/store/apps/details?id=com.root.scentiment |
| Publisher | Scentiment, LLC | Scentiment (contact domain scentiment.com) |
| Latest | 1.1.1, Jun 4 | updated Jul 15, 2026 |
| Rating | 1.5 / 5 (157 ratings) | 1.5–1.6 / 5 (~210 reviews), 10K+ installs |

- The App Store listing names the Mini, Air 2, Mini 2 LE and Pro 2 as supported (from
  reviews). The HVAC 2 isn't mentioned.
- The iOS privacy label lists precise location "for app functionality". Apps need
  location permission to read the phone's current WiFi name, which is typical of apps
  that hand WiFi details to a device (inference).
- Version notes mention "Wi-Fi configuration", "Wi-Fi connection status on the device
  control page", "Bluetooth reliability", "clearer oil-level status indicators" and
  "smart scheduling".
- **How setup works, from an Android review (Feb 21, 2026, Air 2):** Bluetooth
  connects fine, and "sharing network info with my device" succeeds. Then "finish
  setup" fails with "wifi connection failed". So **the WiFi name and password go to
  the device over Bluetooth**, and the failure comes afterwards, when the device or the
  app checks in with Scentiment's cloud (likely).
- A Sep 24 iOS review: accounts can't be created ("credentials are not valid").
- The Android package name `com.root.scentiment` suggests an outside developer
  ("root") built it (guess).

### Where the implementation details would be

- The **Android app package (APK)** contains the app's code. It would show the
  Bluetooth command that sends WiFi details, any other commands (schedules, oil
  level), and which cloud hosts and protocol the device and app use. iOS app files are
  encrypted and can't be read this way.
- No trustworthy mirror of `com.root.scentiment` turned up. The clean way to get it is
  from Google Play, installed on an Android phone and copied off with `adb`.

### Home Assistant community

- No Scentiment-specific threads. The main scent-diffuser thread
  (https://community.home-assistant.io/t/whole-home-cold-air-scent-diffuser-automation-is-possible-without-crappy-apps/541137)
  centers on Aroma-Link devices and Scent Assistant.

## 2026-10-20: Scentiment 2.0 Android app, decompiled (app code)

Source: `com.root.scentiment` 1.2.9 (versionCode 94), copied with `adb` from our own
Pixel 6a and decompiled with jadx 1.5.6. The app's own code lives under
`com.scentiment.app`. Class names below point into it. Only the protocol is recorded
here. No app code is copied.

The app embeds credentials for Scentiment's MQTT brokers, its REST API and analytics.
**They are not recorded here and must never be used.** Using them would mean
contacting Scentiment's servers, which CLAUDE.md forbids. The same credentials let
anyone publish to any device topic on those brokers, which is one more reason to move
our diffusers off them.

Labels: *confirmed* = seen in code. *Inferred* = read from surrounding code or names.

### BLE (confirmed)

- Service `0x0180`, write `0xDEAD`, notify `0xFEF3` (CCCD enabled), read `0xFEF4`. Same
  as Scent Assistant.
- Commands: UTF-8 `{"action":"X","payload":{...}}` + `-|`. `payload` is left out when
  there isn't one.
- Write with response. The app asks for MTU 180 and writes in chunks of MTU − 3, waiting
  for each write to finish (2.5 s timeout).
- No bonding, PIN, handshake, token or encryption.
- The app prefers BLE when connected and falls back to MQTT otherwise.

### Device info (`0xFEF4` read, confirmed)

`manufacturerId|modelName|variantName|deviceId|firmwareVersion|isWifiConfigured|wifiStatus`
(`BluetoothGattManager`). `wifiStatus` is 0–4. On firmware > 2.0, 2 or 3 means connected.

### BLE names and models (confirmed, `BluetoothDiscovery`)

- The app accepts any device whose name contains `scentiment` (any case) and whose
  RSSI is at least −90.
- Name format: `<prefix>-<ModelCode> <VariantCode> [suffix]`.
- Model codes: `D` Diffuser Air 2, **`H` Hvac**, `P` Pro, `M` Mini, `T` Mini 2 LE,
  `W` The Plug-In, `R` Diffuser_4.
- Variant codes: `B` Black, `C` Graphite Gold, `G` Grey, `W` White, `S` Silver, `R` Red.
- Every model goes through the same command code. The HVAC only differs in its default
  intensity (1) and in having no oil reading.

### Commands the app sends (confirmed, `DeviceCommand`, `DeviceConnectionManager`)

| Action | Payload | Notes |
|---|---|---|
| `TURN_ON` | `{"on":1}` / `{"on":0}` | Used for both on and off |
| `SET_LEVEL` | `{"intensity":1..3}` | 1 low, 2 medium, 3 high |
| `SET_RGB_LEVEL` | `{"red","green","blue"}` (0–255) | |
| `SET_RGB_LED` | `{"on":0\|1}` | |
| `WIFI_CONNECT` | `{"ssid","password"}` | The only fields. No cloud endpoint or credentials are sent |
| `SYNC_TIME` | `{"serverTime": <UTC epoch seconds>}` | Sent on firmware > 1.3.8 |
| `UPDATE_PROFILE` | profile with schedule table (below) | Schedules, and power/level over MQTT |
| `INITIAL_LOAD` | same shape, factory entry only | Sent after setup |
| `PING` | none over BLE; `{"userid":…}` over MQTT | The device replies with `acknowledgement` |
| `GET_STATUS` | `{}` | MQTT |
| `RESET` | `{"mode":"soft"\|"hard"}` | MQTT |
| `SECRET_COMMAND` | `{"command":"restart"}` = soft restart | Over BLE. A hard-reset variant uses a fixed token (not recorded) |
| `UPDATE_FIRMWARE` | `{"url":…}` | No version, checksum or signature sent; see Firmware |
| `SAVE_MQTT` | `{"broker","username","password"}` | **Changes which MQTT broker the device uses** |

Defined but never sent: `TURN_OFF`, `SET_INTENSITY`, `SET_SCHEDULE`, `RESET {type}`,
`U_FW`.

### `SAVE_MQTT`: pointing a device at another broker (confirmed in app, untested on device)

The app moved devices between Scentiment's brokers with `SAVE_MQTT` (broker URL,
username, password). The device answers `Credentials saved`, then the app sends
`RESET {"mode":"soft"}` and a `PING` through the new broker
(`DeviceConnectionManager.migrateToEmqxToAws*`). The broker URLs the app hands over
are `wss://…/mqtt` (WebSocket over TLS) with username and password.

The app only ever sent it over MQTT. Whether the firmware also accepts it over BLE
is **untested**. The BLE path does pass any action through unchanged.

### WiFi setup sequence (partly confirmed)

The main setup method didn't decompile fully. The order is rebuilt from the code
around it.

1. Scan and connect over BLE. Read `0xFEF4`.
2. Send `WIFI_CONNECT {"ssid","password"}` (confirmed).
3. Poll `0xFEF4` until `isWifiConfigured` is true (inferred).
4. Wait up to 90 s for the device to come online on MQTT (PING and acknowledgement,
   then broker and backend checks) (confirmed).
5. When it's online, send `SYNC_TIME`, then `INITIAL_LOAD`, then a soft reset
   (confirmed).
6. Make REST calls that link the device to the account (confirmed). This "finish
   setup" step is the one that fails now.

The device's cloud endpoint is **built into the firmware**. The app doesn't send it
during setup, but `SAVE_MQTT` can change it afterwards.

### MQTT (confirmed)

- Topics:
  - app → device: `scentiment-mqtt-prod/device/<deviceId>`
  - device → app: `scentiment-mqtt-prod/user/<userId>`
  - The device probably learns the user ID from `PING {"userid"}` (inferred).
- QoS 1, not retained.
- Scentiment's brokers: AWS IoT (`…-ats.iot.us-west-2.amazonaws.com`, WSS, custom
  authorizer) and two EMQX Cloud brokers. Firmware ≤ 1.3.5 used a legacy EMQX broker.
- Device → app actions:
  - `update_status`: `{deviceId, data:"{\"data\":[…]}"}`
  - `acknowledgement`: `{deviceId, msg}`. `msg` can be `Credentials saved`,
    `BATTERY_MODE`, `PLUGIN_MODE`, `OTA_SUCCESS`/`OTA_FAIL` or `DEVICE_UNLINK`, a JSON
    status, or `{"result":"ALREADY_ADDED"}`.
  - `device_connected`: `{deviceId, fw, model, variant, manufacturer}`
  - `initial_load` and `update_profile`: the schedule table
  - `notification`: `level:N` / `enable:0|1`

### Status array (confirmed)

`{"data":[…]}` arrives on `0xFEF3`, in `update_status`, or inside an `acknowledgement`.
The app ignores indices 0–3 and reads:

| Index | Meaning |
|---|---|
| 4 | power |
| 5 | charging (1 or 2 = charging) |
| 6 | battery % |
| **7** | **oil % (0–100, 255 = unknown)** |
| 8 | fan |
| 9 | intensity |
| 10 | hours worked |
| 11 | gas pump |
| 12 | light switch |
| 13 | factory-restore status |

Scent Assistant's 16-number frame `[85,170,11,255,0,2,200,100,255,0,0,255,255,0,48,106]`
fits this layout:
- indices 0–3 = header `55 AA 0B FF`;
- charging = 2 (on external power), battery = 200 (matches `Bat:200`), oil = 100;
- the last two numbers are extra, and the last is an XOR checksum.

**Unresolved:** at the time of that frame, the app showed "Medium" oil, but oil = 100
should bucket as High. The device's oil reading may be unreliable, or the app showed a
stale value from the backend.

The text line `S:1,L:3,C:1,Bat:100` is power, intensity, **charging state (0/1/2)** and
battery.

### Oil level (confirmed, `OilLevelMapper`)

- Diffuser and Pro models bucket index 7: 0 = Empty, 1–33 = Low, 34–66 = Medium,
  67–100 = High.
- The Plug-In reports present/absent.
- **The HVAC has no oil mapping**: the app shows no oil level for it.
- The app doesn't estimate oil.

### Schedules: stored on the device (format confirmed, on-device running inferred)

The app sends the whole table with `UPDATE_PROFILE`:

```json
{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[
  {"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],
   "workingState":{"level":2,"fan":0,"led":0,"timer":0},"enable":true,"isFactory":true},
  {"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],
   "workingState":{"level":3,"fan":0,"led":0,"timer":0},"enable":true}]},
 "timezone":-7.0,"schedule_flag":1}}
```

- Times are `HH:mm` (24-hour). Weekdays are 0 = Sunday … 6 = Saturday. `timezone` is the
  UTC offset in hours.
- Entry 1 is a fixed "factory" entry that can't be edited or deleted.
- Limits: 7 entries on firmware ≤ 2.21, otherwise 100.
- After changing a schedule, the app sends `SYNC_TIME` and backs the table up to its
  backend.
- Over MQTT, power and intensity also use `UPDATE_PROFILE` (factory entry only,
  `schedule_flag: 0`).
- Nothing in the app or backend fires commands at schedule times, so **the device
  runs its own schedule** (inferred, strong).
- In the app, schedules go over MQTT only. Whether the device accepts `UPDATE_PROFILE`
  over BLE is **untested**.

### Firmware updates (confirmed)

- `UPDATE_FIRMWARE {"url"}` goes over BLE or MQTT. The URL comes from Scentiment's
  backend.
- The app sends no checksum or signature. Whether the device verifies the image
  itself is unknown.
- Success is reported as `OTA_SUCCESS`/`OTA_SUCCEED`.

### What still had to be tested (as of the app-code review; see "Phase 0 results" for outcomes)

1. `0xFEF4` on both units: the HVAC 2's model code and both firmware versions.
2. Whether `SAVE_MQTT` works over BLE, which broker URL schemes it accepts
   (`wss://`, `mqtts://`, `mqtt://`), and whether it checks TLS certificates.
3. Whether `UPDATE_PROFILE` and `SYNC_TIME` work over BLE, and whether schedules keep
   running after a power cut.
4. Whether the Air 2's oil value (index 7) tracks real refills.

## 2026-10-20: Phase 1 desktop app, acceptance session (in progress) and follow-ups

### Owner acceptance session [observed]

The owner ran `docs/phase-1-owner-session.md` against the bundled app, with both diffusers
nearby.

- **Window and Settings: pass.** The title bar is unified with the content, the window
  drags, ⌘, opens Settings in its own window, and the broker connects once the password
  has been saved there. The password is then in the login Keychain under service
  `scent-control`.
- **App icon: pass.** The Dock shows the indigo diffuser icon from PR #5.
- **Scan.** At first, Found Nearby said "No new diffusers found nearby". After a rebuild
  and relaunch it listed both diffusers. Two possible causes:
  - Every ad hoc re-sign can make macOS ask for Bluetooth permission again, and a scan
    without the permission comes back empty.
  - The app may have checked whether Bluetooth was ready before CoreBluetooth had
    reported its first state.

  Either way, the app showed a failed scan as "nothing found". A fix is in progress on
  branch `desktop-env-and-scan`.
- **Adding a diffuser: not yet confirmed.** Both diffusers were listed from the broker,
  named and given rooms. `desktop.json` showed no Bluetooth read for either (no `info`,
  no `ble_id`), so neither add has run end to end yet.

### Oil level on the Air 2 [observed]

This is the follow-up to the app-code item "whether the Air 2's oil value tracks real
refills". Over 5 minutes, the app watched the Air 2 (firmware 2.30) over the broker,
asking for status every 20 s. Status index 7 read:

| Time | State | Index 7 |
|---|---|---|
| 22:44:39 | on, intensity 1 | 100 |
| 22:45:04 | switched off | 0 |
| 22:45:32 | on again, intensity 2 | 50 |
| 22:45:36 | on, intensity 1 | 50 |

So index 7 follows whether the diffuser is running and its spray/pause phase. It is
**not an oil level** on the Air 2, and the HVAC 2 has no such field. The app shows no oil
level for either model.

Also seen during the same watch:
- Over MQTT, `GET_STATUS` was answered only with `acknowledgement` messages, with no
  status array. Status arrays arrived only on a change.
- The Air 2 was on battery at the time (charging 0, battery 98).

### Scentiment app: schedule read-back, lights, names and rooms (app code)

- **Reading back a stored schedule.**
  - No app → device command asks a diffuser for its schedule. The only read-like request
    is `GET_STATUS` (over MQTT, firmware > 2.12), and its reply holds no schedule.
  - The app shows schedules only from its own backend.
  - Incoming device messages carrying a schedule are barely used:
    - `update_profile`: only entry 0's enable and level are read.
    - `initial_load`: when its schedule is empty, the app pushes a default.
  - Schedule limits: at most 7 entries on firmware ≤ 2.21 and 100 above. Entry 1 (the
    factory entry) holds the master power and intensity.
  - Scent Control therefore keeps the last schedule it sent, and shares it through the
    broker (interim; branch `desktop-shared-schedules`).
- **Lights.**
  - Commands: `SET_RGB_LED {on}` and `SET_RGB_LEVEL {red,green,blue}`. The app's "warm"
    light is preset RGB values.
  - The app shows light controls only for the Wall Plug / Plug-In model. Nothing enables
    them for the Air 2 (model `D`) or the HVAC (`H`). That matches Phase 0, where the Air
    2 ignored both commands.
  - The status array's light switch (index 12) is only logged.
  - The schedule entries' `workingState.led` is never set from a control.
- **Names and rooms.**
  - Names and rooms are kept only in Scentiment's backend, as Houses with Spaces.
    Nothing is written to the device.
  - The advertised Bluetooth name is set by the firmware.
  - Scent Control keeps names and rooms locally (PR #7).

### Not yet observed

- A large `UPDATE_PROFILE`: 99 entries is about 13.5 KB. The app and broker accept it, but
  it hasn't been sent to real firmware.
- Whether the diffusers connect with `clean_session` set. Read the `c` flag from the
  broker's connect log line.
- The add flow's Bluetooth wording for "Credentials saved", and whether the `SECRET_COMMAND`
  restart write is acknowledged before the reboot.
