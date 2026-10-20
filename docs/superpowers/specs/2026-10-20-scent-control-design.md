# Scent Control: design

Date: 2026-10-20. Status: draft for approval.

Decisions are marked **(owner)** when the project owner made them and **(designer's call)**
when the designer made them. Anything marked "builder's call" is left to whoever
plans and builds this. Device facts come from [docs/findings.md](../../findings.md),
which has the sources.

## 1. Why this exists

Scentiment's diffusers depend on Scentiment's cloud. The official app (old and new,
"Scentiment 2.0") can't finish setting up a diffuser, and accounts can't be created,
because the cloud no longer works. The diffusers still run from their physical
buttons, but app control, WiFi and schedules are gone.

Scent Control replaces the app and the cloud. It is an app plus a server that set
up Scentiment diffusers on the owner's WiFi, control them from anywhere, and run
their schedules. It never depends on Scentiment.

It starts with the owner's two diffusers, then grows to invited testers, and will
eventually be open source and support other brands.

## 2. Goals for the first version

1. **Add a diffuser from the app (owner).** Scan Bluetooth and list nearby
   diffusers with their model. The user picks one, enters their WiFi password, and the
   app connects the diffuser to their WiFi and to our server. The device doesn't
   get opened.
2. **WiFi control from anywhere (owner).** Power, intensity (1–3) and LED color
   go through our server, not Bluetooth, so the phone doesn't need to be nearby.
3. **Schedules (owner).** Create, edit, enable/disable and delete schedules in the
   app. The diffuser stores and runs them itself.
4. **Status.** Online/offline, power, intensity, battery and charging where the
   model reports them, and **oil level only on models that report it (owner)**.
5. **Accounts (owner).** People sign up and sign in. Each person's diffusers are
   private to their home.
6. **Testers (owner).** iPhone builds go out through TestFlight, Android builds as
   an APK.
7. **Both Bluetooth and WiFi (owner, 2026-10-20).** Like Scentiment's own app,
   the app talks to a diffuser over Bluetooth whenever the phone is in range and the
   link connects. Otherwise it goes through the server over WiFi. This replaces the
   earlier designer's call of Bluetooth only as a fallback.

### Definition of done for v1 (designer's call)

- The owner's Air 2 and HVAC 2 were both added through our app and run through our
  server.
- A schedule set in the app runs on the diffuser while the phone is off.
- After a power cut, a diffuser reconnects and gets back its schedule and the time
  without anyone touching it.
- A second person installs a TestFlight or APK build, signs up, and either adds a
  diffuser to their own home or joins the owner's home by invitation.

## 3. Non-goals for v1

- Other brands. The code is shaped for them (§6.4), but no other drivers ship.
- Firmware updates or custom firmware.
- A web app. The server has an API, but the only client is the mobile app.
- Public App Store and Play Store listings.
- Push notifications, for example a low-oil alert (designer's call: later).
- Home Assistant integration.
- A fragrance shop, or anything else from Scentiment's app beyond device control.

## 4. Anti-goals: failures even if everything else works

- **Contacting Scentiment's servers, or using any credential found in Scentiment's
  app.** Their app embeds broker and API credentials. We never use them, store them,
  or publish them.
- **Storing users' WiFi passwords on our server.** The password goes from the phone
  to the diffuser over Bluetooth, and nowhere else.
- **Bricking a diffuser.** Never send `UPDATE_FIRMWARE`, and never send the hard-reset
  `SECRET_COMMAND` (its token isn't recorded anywhere in this repo). A soft restart is
  fine.
- **One diffuser or user reaching another's.** Each diffuser's broker login can
  only use that diffuser's topics. Adding a diffuser requires being next to it.
- **Copying Scentiment's app code.** We implement the protocol from the documented
  facts in docs/findings.md, not from decompiled code.
- **Committing secrets, device captures or vendor app files.** See `.gitignore` and
  `SECURITY.md`.

## 5. Constraints

### Devices

- **Scentiment Diffuser Air 2**, and **Scentiment Diffuser HVAC 2** (touchscreen,
  500 ml). The HVAC 2 is not the Scent Diffuser Pro 2. Both are from the "2" series,
  built around an Espressif chip, and **must not be opened (owner)**.
- WiFi is 2.4 GHz only.
- Bluetooth allows one connection at a time. Scentiment's app must be closed during
  setup.
- **The firmware is fixed, so the server must speak its protocol exactly.**
  - BLE: service `0x0180`, write `0xDEAD` (JSON + `-|`, with response, chunked to
    MTU − 3), notify `0xFEF3`, info `0xFEF4`.
  - MQTT topics: the device subscribes to `scentiment-mqtt-prod/device/<deviceId>`
    and publishes to `scentiment-mqtt-prod/user/<userId>`. Envelope
    `{"action","payload"}`, QoS 1.
  - Command and message formats are in docs/findings.md.
- **Behavior varies by firmware version** (docs/findings.md):
  - firmware ≤ 1.3.5 used a legacy broker;
  - `SYNC_TIME` exists on > 1.3.8;
  - 7 schedule entries on ≤ 2.21, otherwise 100;
  - `wifiStatus` 2 or 3 means connected on > 2.0.
  The codec must take the firmware version into account.
- The HVAC reports no oil level. The Air 2 reports oil as 0–100 (index 7 of the
  status array), but one report suggests it may be unreliable.
- **If the HVAC 2 turns out to speak a different protocol, v1 ships with the Air 2
  only**, and the HVAC 2 gets its own follow-up (designer's call, for the owner to
  confirm). The v1 definition of done then counts the Air 2 alone.
- **During setup, the stock firmware may briefly try Scentiment's built-in broker**
  if `SAVE_MQTT` only works after `WIFI_CONNECT`. That short window is accepted
  (designer's call, for the owner to confirm): our code never contacts Scentiment,
  and the attempt is the device's own built-in behavior. Phase 0 tries the order that
  avoids it first (§7).

### Stack (owner: base it on the HCCNA monorepo)

- Bun 1.4 workspaces, Turborepo, a single `bun.lock`, and a shared version catalog.
- **Effect v4 only** (see HCCNA `AGENTS.md`). Follow its conventions: services as
  `Context.Service`, errors as `Schema.TaggedError`, and so on.
- Server: Bun, Hono, Effect, Drizzle, Postgres.
- App: Expo SDK 57 / React Native with a custom development build, since Bluetooth
  needs native code.
- Tooling: oxlint with the anti-slop rules, plus typecheck/lint/test through turbo.
- Hosting: Railway.
- Copy HCCNA's conventions where they fit. Don't copy its domain code (GraphQL/Relay,
  Strapi, CCB, Firebase).

### Repo

- `github.com/Jibz1/scent-control`, **private for now, open source later (owner)**.
  Follow open-source practice from the start: MIT license, `SECURITY.md`,
  `CONTRIBUTING.md`, no secrets or personal data in the repo.

### Accounts and addresses

- **Broker and API on Railway-generated addresses for now (owner).** Each diffuser
  stores the broker address permanently, so:
  - **Never delete or recreate the Railway service, or its generated domain,
    once a diffuser points at it.** If that address disappears, every diffuser has
    to be set up again in person.
  - To move to a custom domain later, add it to the same service, send `SAVE_MQTT`
    with the new address to each connected diffuser over our broker, and keep the
    old address working until every diffuser has moved.
- **Email:** sign-in codes need a sending domain before strangers can sign up. In
  Phases 1–2 only the owner signs in, so the email provider's test sender (which
  only delivers to the account owner's own address) is enough. **A domain is a
  prerequisite for Phase 3.**
- **Apple:** the owner has a personal paid Apple Developer membership (owner), so
  TestFlight and the WiFi-name entitlement are available.
- **Railway workspace:** to be picked by the owner when Phase 1 sets up hosting.

## 6. How it works

### 6.1 Parts

```
Mobile app ──HTTPS──▶ Server (API, accounts, homes, schedules, reconciler)
    │                     │
    │ Bluetooth           │ MQTT (server's own login)
    │ (setup + fallback)  ▼
    └──────────────▶ MQTT broker ◀──wss (one login per diffuser)── Diffusers on home WiFi
```

- **Mobile app** (`apps/mobile`): Bluetooth for setup and fallback control, HTTPS to
  the server for everything else, and a live connection to the server for status
  updates (builder's call: SSE or WebSocket).
- **Server** (`apps/server`): the API, accounts and homes, the device registry,
  schedules, issuing per-device broker logins, and the reconciler (§6.6). It
  connects to the broker as a privileged client.
- **MQTT broker** (designer's call: on Railway): diffusers connect to it over
  **`wss://` on a Railway HTTPS domain**, so they see a normal public TLS certificate.
  The broker must have:
  - one username/password per client;
  - per-client topic rules managed by the server;
  - QoS 1 and persistent sessions.
  Which broker software is the builder's call (Mosquitto with dynamic security,
  EMQX open source, or similar).
- **Protocol package** (`packages/…`, name is the builder's call): pure functions,
  bytes or JSON in and typed data out. Shared by app and server.

### 6.2 Adding a diffuser

1. The user taps "Add diffuser". The app scans for Bluetooth names containing
   `scentiment` with RSSI ≥ −90, decodes the model and color from the name
   (`D` = Air 2, `H` = HVAC, …), and lists them.
2. The user picks one. The app connects and reads `0xFEF4`: manufacturer, model,
   variant, `deviceId`, firmware version, WiFi status.
3. The app asks the server to start adding this `deviceId` to the user's home. The
   server returns a broker URL, a fresh per-device username and password, and the
   `userId` value the device should publish under (§6.3).
4. The user enters their WiFi password. The app fills in the WiFi name when the OS
   allows it; otherwise the user types it.
5. Over Bluetooth, the app sends `WIFI_CONNECT {ssid,password}`, then
   `SAVE_MQTT {broker,username,password}`, then a soft restart. It then polls
   `0xFEF4` until WiFi is configured.
6. The server sees the device connect with its new login. It then sends `PING
   {userid}`, `SYNC_TIME` and the device's starting profile, and marks the device
   online in the user's home.
7. If the device doesn't come online within about 90 s, the app explains the
   likely causes (wrong password, 5 GHz network, out of range) and offers to retry.

**Ownership:** being next to a diffuser proves you may add it. If a diffuser already
belongs to another home, adding it moves it to the new home and revokes its old broker
login (for example, after it's sold or passed on). The move only happens once the
device actually connects with its new login, so a setup that fails partway leaves
it where it was.

**Order of the setup commands:** whether `SAVE_MQTT` goes before or after
`WIFI_CONNECT`, and whether a restart is needed between them, gets settled in
Phase 0.

### 6.3 Topics and logins

- We keep Scentiment's topic names, because the firmware has them built in.
- Each diffuser gets one broker user, which may only:
  - subscribe to `scentiment-mqtt-prod/device/<its deviceId>`;
  - publish to `scentiment-mqtt-prod/user/<its route id>`.
- The **route id** is a random value the server assigns per diffuser and gives the
  device as `userid` in `PING`. Using a per-device value rather than a real user ID
  keeps the topic rules strict and reveals nothing about accounts (designer's call).
  Whether the device remembers it across restarts or needs a `PING` each time
  gets settled in Phase 0.
- The server subscribes to `scentiment-mqtt-prod/user/+` and publishes on device
  topics. No other client gets wide topic access.

### 6.4 Drivers and capabilities (ready for other brands)

- Each device family is a **driver**: a list of capabilities plus operations over one
  or more transports. The Scentiment driver runs over BLE and over MQTT.
- Capabilities, for example: power, intensity range, RGB LED, battery, charging, oil
  level, on-device schedules (with an entry limit), fan.
- The app shows only what the device supports. That's the owner's rule for oil level,
  applied to every feature.
- Capabilities depend on the model and firmware version. The HVAC has no oil level.
  The schedule limit depends on firmware.

### 6.5 Control, schedules and status

- **Manual control over WiFi.** Builder's call, settled in Phase 0: either the
  commands Scentiment's app sends over MQTT (`UPDATE_PROFILE` with the factory entry
  and `schedule_flag: 0`) or the direct actions (`TURN_ON`, `SET_LEVEL`,
  `SET_RGB_*`) if the device accepts them over MQTT.
- **Over Bluetooth:** `TURN_ON`, `SET_LEVEL`, `SET_RGB_LEVEL` and `SET_RGB_LED`, as
  documented.
- **Default intensity is 1 (owner, 2026-10-20)** wherever the app or server has to
  pick one: switching on, new schedule times and the always-on entry. Scentiment's app
  uses 2 for "Diffuser" models.
- **Schedules:**
  - The server keeps the master schedule table for each device and sends the
    whole table with `UPDATE_PROFILE` (`schedule_flag: 1`), followed by
    `SYNC_TIME`.
  - Entry 1 is the device's fixed "factory" entry. The app never lets anyone
    edit or delete it.
  - Show the per-firmware entry limit.
- **Time zones:** each home stores an IANA time zone. The device only takes a
  decimal UTC offset, so the server re-sends the schedule table when the offset
  changes (daylight saving) and whenever a home's time zone changes.
- **Status:** parse the 14-number status array and the text formats into one typed
  state. Store the latest state per device and stream changes to the app. The raw
  oil value is 0–100. The app shows Empty/Low/Medium/High with the same thresholds
  as Scentiment's app.
- **Bluetooth first when nearby, WiFi otherwise (owner, 2026-10-20).** When the
  phone can reach the diffuser over Bluetooth, the app sends commands that way.
  Bluetooth also has the fullest command support: intensity and replies are
  immediate, per Phase 0. Otherwise commands go through the server and broker. The
  app still drops idle Bluetooth connections soon after use, because a held
  connection blocks others, drains battery models, and seemed to delay WiFi
  commands in Phase 0. Anything changed over Bluetooth is reported to the server, so
  the stored state stays current.

### 6.6 Desired state (from CLAUDE.md, adapted)

- The server stores what each device *should* have: its schedule table, its manual
  power/intensity/LED state, and its time zone offset.
- **The person's most recent change wins, wherever it was made (designer's call,
  for the owner to confirm).** Pressing a button on the diffuser or editing a schedule
  on the HVAC's touchscreen counts as much as a change in the app. When the device
  reports such a change, the server adopts it into the stored state. A power cut,
  reboot or reconnect is **not** a change, and must never undo one.
- When a device connects (a `device_connected` message or a broker connect event),
  the server:
  1. sends `SYNC_TIME`, then reads the device's state and profile (`GET_STATUS`, plus
     the `initial_load`/`update_profile` it reports);
  2. restores only what the device lost (for example an empty schedule table, or a
     time zone offset that changed while it was offline);
  3. leaves alone anything the person changed on the device.
  A device that was offline at 7 am therefore catches up when it's back, and runs its
  own schedule afterwards.
- Phase 0 has to observe what the device keeps across a power cut (schedule, power,
  intensity) before the reconciler's rules can be pinned down. Settling the exact
  rules is the builder's job, within the principle above.
- Commands sent while a device is offline aren't queued for later. The app shows the
  device as offline.

### 6.7 Accounts and homes

- **Sign-in:** email and a one-time code. No passwords (designer's call). Which
  auth library and email provider to use is the builder's call.
- **Homes:**
  - A home is created at sign-up. The owner can invite people by email.
  - Members control everything in the home. Only the owner can remove members or
    the home (designer's call).
  - One home per user is enough for v1.
- Every API call and live update is limited to the caller's home.
- **Stored data:** email, home, devices (`deviceId`, model, variant, firmware, name,
  room), schedules, the latest state, and per-device broker login hashes.
- **Not stored:** WiFi passwords. Store only the WiFi name, and only if the app
  needs it.

### 6.8 Distribution

- iPhone: TestFlight. External testers need Apple's beta review, so the app needs
  Bluetooth and location usage strings and a privacy policy. No third-party social
  login means Sign in with Apple isn't required.
- Android: a signed APK shared directly.
- Builder's call: EAS or local builds. HCCNA builds its store builds locally.

## 7. Phases (riskiest first)

**Phase 0: prove it on the real devices (gate).** Use throwaway scripts under
`tools/`. This is CLAUDE.md's first task, extended.

1. Scan both diffusers and dump their GATT. Read `0xFEF4`: model code, firmware and
   `deviceId` format.
2. Confirm the BLE commands: power, level, LED.
3. Send `SAVE_MQTT` over BLE to a test broker, and confirm the device connects,
   which topics it uses, and how `PING {userid}` behaves after a restart.
   - **Command order:** try `SAVE_MQTT` *before* `WIFI_CONNECT` first, so the
     device never gets a chance to reach Scentiment's built-in broker (designer's
     call).
   - **Connection type:** try `wss://` on a test Railway broker with a public
     certificate first. Any encrypted connection with a publicly trusted
     certificate on our own domain is acceptable without asking, including
     `mqtts://` through Railway's TCP proxy (designer's call).
   - **Plain unencrypted MQTT, or moving the broker off Railway:** stop and ask the
     owner.
4. Confirm `UPDATE_PROFILE` + `SYNC_TIME` run a schedule with the phone away. Check
   what happens after a power cut, and whether the HVAC's touchscreen schedules
   conflict with ours.
5. Record everything in docs/findings.md with dates.

**If `SAVE_MQTT` doesn't work over BLE**, stop and bring the options to the owner.
The fallbacks are:

- (a) Bluetooth-only control, with schedules on the device if `UPDATE_PROFILE`
  works over BLE.
- (b) A small always-on bridge at home (Raspberry Pi or ESP32) that does Bluetooth
  and connects to our server.

**If schedules don't run on the device** (step 4), also stop and ask. The fallback
is (c): the server sends commands at schedule times. Those schedules then stop
whenever the internet or the server is down (designer's call to treat this as a
gate).

Opening devices stays off the table.

**Phase 1: one diffuser end to end, for the owner.** The protocol package with
fixtures, the server and broker on Railway, and app setup and manual control.
Android first, since the owner has a Pixel 6a for testing (designer's call).

**Phase 2: schedules, status and resilience.** The schedule editor, the desired-state
reconciler, status and oil level, the Bluetooth fallback, and the HVAC 2.

**Phase 3: other people.** Sign-up, homes and invites, TestFlight and APK builds,
and the privacy policy. Needs a domain for sign-in email first (§5).

## 8. Left to the builder

- Package names and internal layout beyond `apps/mobile`, `apps/server` and a shared
  protocol package.
- API style (REST is enough) and the live-update mechanism.
- Broker software (must meet §6.1), auth library and email provider.
- The React Native Bluetooth library.
- UI and visual design.
- Updating `CLAUDE.md`: it still describes the earlier Python/PWA plan and must be
  rewritten to match this design before Phase 1 code lands.

## 9. Before going public (owner decisions, not v1 work)

- Consider telling Scentiment privately that their app ships credentials that allow
  control of any device, before publishing anything that describes it.
- Decide how much of the decompiled-app findings to publish.
- Confirm the MIT license and the use of the "Scentiment" name (README disclaimer).

## Appendix: Phase 0 results (2026-10-20)

Phase 0 gate: **go.** Details and evidence are in `docs/findings.md`, under "Phase 0
results". These are the places where reality differs from this design, and what
follows for later phases:

1. **Oil level isn't available on either unit.** Goal 4's oil display has nothing to
   show for now. The capability stays in the driver model and is off for both models.
2. **Light control doesn't work** over Bluetooth or WiFi on firmware 2.30. The LED
   capability is off for both models until shown otherwise.
3. **The schedule must be followed by `SYNC_TIME`**: send `UPDATE_PROFILE`
   (`schedule_flag 1`), then `SYNC_TIME`. Sending the clock first didn't run the
   schedule.
4. **Over WiFi, intensity and power changes through the always-on entry get no
   reply.** The server treats them as sent-but-unconfirmed until a later report.
5. **Online status comes from active pings**, not the broker connection: the device
   uses keepalive 1200 s and no will message. The server pings each diffuser regularly,
   as Scentiment's app does, and a missed answer means offline.
6. **The device reports state only on change**, and nothing triggers a full report
   without changing something. The app shows each reading with its source and age,
   alongside what the schedule says the device should be doing. This is the owner's
   power-reporting rethink, prototyped in the test bench.
7. **Parsing must tolerate the firmware's invalid JSON** (`"data":enable:0`) and the
   object-shaped `update_status` data.
8. **`WIFI_CONNECT` is only needed for diffusers not yet on WiFi.** Setup checks
   `0xFEF4` first, and if WiFi is already configured it sends only `SAVE_MQTT` and a
   soft restart.
9. **Topics:** the server also handles `scentiment-mqtt-prod/server-notifications/device`
   (a device announcing itself before it has a user ID). Per-device broker rules must
   allow subscribing to `scentiment-mqtt-prod/announcements` and publishing there.
10. **One WiFi home per diffuser.** A diffuser on our broker can't use Scentiment's
    cloud at the same time. Bluetooth works for both apps, one at a time.
11. **Not yet tested, moved to Phase 2:** button and touchscreen change reporting, a true
    power cut (the Air 2 has a battery), and HVAC 2 schedules.
12. **macOS desktop clients must be app bundles that declare Bluetooth use**, which
    matters for the desktop app (see the Phase 1 design addition).

