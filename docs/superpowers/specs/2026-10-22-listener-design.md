# The listener: design

Status: draft for the owner's review, 2026-10-22.

The listener is the first piece of `apps/server`. It runs all the time on Railway, hears
everything the diffusers say on our broker, keeps each diffuser's latest state, pings each
one to know whether it is online, asks for its state when that isn't known, and shares all
of that through the broker. The Mac app
reads it from the start, and the iPhone app is built on top of it (its revised design comes
next).

Parent design: `docs/superpowers/specs/2026-10-20-scent-control-design.md` (§5 Stack,
§6.3 Topics and logins, §6.5 status, and the addendum's "Shared schedules"). Device facts:
`docs/findings.md`.

## 1. Why

The owner's rule (2026-10-22): an app must never say a diffuser is on, or at an
intensity, when it isn't.

Near a diffuser that already holds: over Bluetooth the Air 2 sends its power and intensity
when a client connects, and answers every `TURN_ON` and `SET_LEVEL` with the new state.
Away from home it can't hold today, because of the fixed firmware (`docs/findings.md`,
Phase 0 summary and the 2026-10-20 oil-level watch):

- A diffuser reports **only on a change**.
- `GET_STATUS` over MQTT is answered with an `acknowledgement` that carries no state.
- Its messages are QoS 1 and **not retained**, so a client that wasn't connected when one
  went by never sees it.
- `SET_LEVEL` over WiFi got no reply in Phase 0.
- With keepalive 1200 s and no will message, the broker can take about 30 minutes to notice
  a diffuser has gone. Only pings find out quickly.

So an app that has just opened has missed the last change and can't ask for it. The Mac
app has the same gap: it only knows what happened while it was open. Something has to be
listening all the time, and that is the listener.

## 2. Goals

1. Any app that connects to the broker gets each known diffuser's latest reported state,
   and the facts it needs to say whether the diffuser is online, at once, without having
   been connected when the state changed.
2. Every value carries the time it was reported. Nothing is presented as current that a
   diffuser didn't report.
3. An app can tell when the listener itself isn't running, so it never presents the shared
   state as live when it may be out of date.
4. The Mac app uses it, so the Mac shows the right state after a relaunch.
5. It lets us settle two open questions (§8): does a diffuser report after `SET_LEVEL` over
   WiFi, and does it report when its own schedule turns it on or off?

## 3. Non-goals

- Sending any command other than `PING` and `GET_STATUS`, which only ask. The apps still
  send their own commands, and the listener hears the result.
- The rest of the server: accounts, homes, invites, an HTTP API, a database, per-diffuser
  broker logins, the master copy of the schedules, time zone and daylight-saving re-sends,
  and the desired-state reconciler (parent design §6.6).
- Any change to the broker's setup. Step 0 in §7 only brings the running broker up to the
  repo's current `infra/test-broker` and gives it its storage volume.
- The iPhone app.
- Sharing what an app reads over Bluetooth. The parent design wants Bluetooth changes
  reported to the server (§6.5); that comes later.
- Removing a diffuser that is gone for good.

## 4. Anti-goals: failures even if it works

- Writing a value into the shared state that the diffuser didn't report, such as the value
  an app *asked* for.
- Publishing a retained message anywhere under `scentiment-mqtt-prod/`. The broker would
  replay it to the diffusers on every connect.
- Sending a diffuser anything but `PING` and `GET_STATUS`. Never `UPDATE_FIRMWARE`,
  `U_FW`, `RESET`, `SECRET_COMMAND` or any other command.
- Pinging with a `userid` other than the route ID the diffusers already hold. Its effect on
  a diffuser is unknown (Phase 0 only saw `ADDED` and `ALREADY_ADDED`), and it could change
  where a diffuser reports.
- Restarting or redeploying the broker as part of deploying the listener.
- A broker password in the repo, in logs or on a command line. Real device IDs in tracked
  files.

## 5. Decided

**By the owner (2026-10-22):**

- The listener comes first, and the phone app is built on top of it.
- What it does, as played back and confirmed ("That's right"):
  - it hears everything the diffusers say;
  - it keeps each diffuser's latest readings, with the time each was heard;
  - it pings each diffuser every 30 seconds;
  - it shares through retained messages on our own prefix;
  - it sends nothing but pings. This was later widened to `GET_STATUS` as well (below).
- Also confirmed in that playback:
  - it uses the broker's one shared login for now;
  - the Mac app reads the same state;
  - accounts, schedules, time zones and the reconciler wait.
- The stack, from `CLAUDE.md`: Bun 1.4, TypeScript, Effect v4 only, Railway, following the
  HCCNA monorepo's conventions.
- **After a diffuser reconnects, its earlier readings are not known** until it reports
  again. Meanwhile the listener asks with `GET_STATUS`, at most every 5 minutes, as the Home
  Assistant adapter does. The Mac follows the same rule. (Decided after the builder check.)

**My calls (the owner can overrule any of them):**

- **Two Railway services, the broker and the listener, not one.** The owner asked whether
  they could be combined. My reasons for keeping them apart:
  - Combined, every server deploy restarts the broker, which drops both diffusers and
    loses anything sent at that moment. The server will change often; the broker
    shouldn't.
  - Kept apart, a crash in our code can't take the diffusers off the broker.
  - Truly combining them means the server *is* the broker, through a TypeScript broker
    library. That replaces Mosquitto, which is proven with this firmware, with something
    that has never met it.
- **No HTTP API yet.** The apps already talk to the broker, and the Mac shares schedules
  through retained messages the same way today.
- **The same online rule as `scent-core`** (`status.rs`): ping every 30 s, wait 15 s for an
  answer, and call a diffuser online for 75 s after hearing from it. The listener publishes
  the facts (`heard_at`, `ping_at`), and each app applies the rule.
- **The MQTT client is MQTT.js** (the `mqtt` package), not one we write. The phone gets
  its own small client in its design. On the server, a mature library is the easier thing to
  read and maintain.
- **The protocol code lives in `apps/server`** for now (`src/protocol/`), not in its own
  package, because the server is its only TypeScript user.
- **The Mac keeps its own pings.** While both run, each diffuser gets two pings every 30
  seconds. Phase 0's bench pinged every 30 s with no trouble. The Mac can stop pinging
  later, while the listener is running.

## 6. How it works

### 6.1 The broker connection

- **Settings**, all from environment variables with the same names the Mac's build and the
  Phase 0 tools use (`.env.example`):
  - `SCENT_BROKER_URL`: the broker's `wss://` address;
  - `SCENT_BROKER_USERNAME` and `SCENT_BROKER_PASSWORD`: the shared login;
  - `SCENT_ROUTE_ID`: the route ID the diffusers already hold (§4).

  A missing value stops the listener at start-up, with a message that names the variable
  but never shows its value.
- **Client ID:** `scent-control-listener-` plus 8 random hex characters, new on every start,
  with a clean session. During a Railway deploy two listeners can run for a moment; distinct
  IDs keep them from knocking each other off.
- **On every ConnAck:**
  - subscribe at QoS 1 to `scentiment-mqtt-prod/#` and `scent-control/v1/#`, the same two
    filters as the Mac (`SUBSCRIBE_ALL`, `SUBSCRIBE_APP`);
  - publish the heartbeat (§6.5);
  - republish every state record it holds, in case the broker restarted and lost its
    retained messages.
- **Reconnects** use a back-off. MQTT.js's built-in reconnect will do.
- **Incoming packets** of up to 256 KiB are accepted, matching the broker's
  `max_packet_size` and the Mac's `MAX_PACKET_SIZE`. A diffuser's acknowledgement can echo a
  full schedule.

### 6.2 What it hears

Each message goes through the TypeScript port of `scent-core`'s `parse_mqtt` (§6.7).

- **Ignored:**
  - messages on `scentiment-mqtt-prod/device/…` (commands going *to* a diffuser);
  - messages with no device ID, or one that fails `valid_device_id` (12 characters, A–Z
    and 0–9).
- **Heard:** any other message from a diffuser, including `acknowledgement` and
  `device_connected`. It sets that diffuser's `heard_at` to the listener's clock (Unix
  seconds).
- **Reported:** every status field the message carries is stored with the same time:
  - power, intensity, charging, battery, fan, gas pump and light, from `Status`;
  - the WiFi signal, `rssi`.
- **Connected:** a `device_connected` message also stores its firmware, model, variant and
  manufacturer.
- **Known diffusers:** a diffuser becomes known the first time any of these happens:
  - the listener hears it;
  - a retained `scent-control/v1/state/<id>` arrives (its own earlier record);
  - a retained `scent-control/v1/schedule/<id>` arrives (the Mac stored a schedule for it).

  On a first-ever start the listener knows nothing. A diffuser joins as soon as it says
  anything, for example its reply to the Mac app's next ping.

### 6.3 Pinging and asking

- Every 30 s, while connected, it pings every known diffuser. Each ping is
  `{"action":"PING","payload":{"userid":"<SCENT_ROUTE_ID>"}}`, published at QoS 1, not
  retained, to `scentiment-mqtt-prod/device/<id>`. It sets `ping_at`.
- **Asking for status**, the Home Assistant adapter's rule (`docs/home-assistant.md`,
  "State and outages"):
  - A diffuser that is reachable (heard within the last 75 s) gets
    `{"action":"GET_STATUS","payload":{}}`, at QoS 1 and not retained, in either case:
    - its power or intensity is not known (never reported, or reported before its last
      reconnect; see §6.4);
    - its newest power or intensity report is more than 5 minutes (`STALE_AFTER_S`) old.
  - It gets at most one every 5 minutes.
  - Firmware 2.30 answers only sometimes. An answer without a status array changes
    nothing.
- One function builds the only things the listener ever publishes under
  `scentiment-mqtt-prod/`, and it can build only this `PING` and `GET_STATUS`. A test pins
  that.

### 6.4 The state message

- **Topic:** `scent-control/v1/state/<deviceId>`, for valid IDs only. Add `state_topic` and
  `state_topic_device` to `scent-core`'s `topics.rs` next to the schedule ones, and mirror
  them in TypeScript.
- **Payload, version 1.** Fields that were never reported are left out:

  ```json
  {
    "v": 1,
    "updated_at": 1792510230,
    "heard_at": 1792510229,
    "ping_at": 1792510200,
    "fields": {
      "power":     { "value": 1,   "at": 1792510100 },
      "intensity": { "value": 2,   "at": 1792510100 },
      "charging":  { "value": 2,   "at": 1792510100 },
      "battery":   { "value": 200, "at": 1792510100 },
      "rssi":      { "value": -66, "at": 1792510100 }
    },
    "connected": { "fw": "2.30", "model": "D", "variant": "B", "manufacturer": "01", "at": 1792509000 }
  }
  ```

  - The keys of `fields` are `power`, `intensity`, `charging`, `battery`, `fan`, `gas_pump`,
    `light` and `rssi`. The values are the raw integers, exactly as the diffuser sent them.
  - `updated_at` is when the listener wrote the message.
- **Publishing:** retained at QoS 1, whenever the record changes. The builder may merge
  changes that come close together, as long as no change waits more than a second.
- **One gate for retained messages:** a single function makes every retained publish, and
  it refuses any topic not under `scent-control/v1/`, as the Mac's `retained_publish` does,
  with the same tests.
- **Merging (newer wins):**
  - each field keeps whichever copy has the later `at`;
  - `heard_at` and `ping_at` keep the later value;
  - `connected` keeps the later `at`.

  This is how the listener restores itself from its own retained records on start. It also
  makes a second listener, briefly running during a deploy, harmless.
- **Rules for readers:**
  - ignore any `v` other than 1;
  - ignore any time more than a day ahead of the reader's own clock, as for shared
    schedules;
  - ignore an empty retained message: that is how a record is taken off the broker;
  - **a field reported before the diffuser's last reconnect is not known** (owner,
    2026-10-22):
    - if its `at` is earlier than `connected.at`, readers show it as not known since
      `connected.at`;
    - a field reported in the same second still counts;
    - the old value stays in the record, because newer-wins merging can't delete a field,
      so this is a rule applied when reading.

### 6.5 The heartbeat

- **Topic:** `scent-control/v1/listener`.
- **Payload:** `{"v":1,"running":true,"at":<unix s>,"started_at":<unix s>}`, retained at
  QoS 1, published on every ConnAck and then every 30 s.
- **Will:** `{"v":1,"running":false}`, retained at QoS 1. If the listener drops off without
  disconnecting, the broker publishes this at once.
- **Shutting down:** on SIGTERM it disconnects cleanly and publishes nothing. A listener
  replacing it during a deploy has already published its own heartbeat, and a "not running"
  from the old one would overwrite it.
- **Readers** treat the listener as running only when `running` is true and `at` is within
  75 s of their clock. Otherwise the shared state may be out of date. An app then says so
  and relies on what it hears itself.

### 6.6 Logs

- Logged: connections, disconnects and reconnects; how many diffusers it knows; the kinds
  of errors.
- Never logged: passwords, settings values, or message bodies.
- Device IDs appear in logs only as their last 4 characters. This follows the Home
  Assistant adapter's rule against printing raw device traffic.

### 6.7 Code

- **Location:** `apps/server`, package `@scent-control/server`, in the Bun workspace.
  - Its `build`, `test`, `lint` and `fmt:check` scripts let `bunx turbo run build test lint
    fmt:check` cover it.
  - `effect` is pinned to an exact version in a root `catalog`, as in HCCNA (4.0.2 there
    today). Any `@effect/*` package matches it exactly.
  - Effect v4 idioms only: `Context.Service`, `Schema.TaggedError`, `Effect.catch*`.
    `bun test`, oxlint and oxfmt, as HCCNA's server uses. HCCNA's anti-slop oxlint plugin
    comes when there is more TypeScript.
  - One `bun.lock` at the repo root.
- **A pure core and a thin shell**, the same split as `scent-core` and the apps:
  - `src/protocol/`: a port of what the listener needs from `scent-core`:
    - from `topics.rs`: the prefixes, `valid_device_id`, `device_topic`, and the new state
      and listener topics;
    - from `message.rs`: `parse_mqtt` and `parse_notification`, including the repair of
      firmware 2.30's unquoted `"data":enable:0`;
    - from `status.rs`: the timing constants;
    - from `command.rs`: the MQTT bodies of `PING` and `GET_STATUS`.
  - The core (pure): applying a parsed message to a diffuser's record, merging retained
    records, and building the state and heartbeat payloads.
  - The shell (Effect services): the MQTT connection, the clock, the 30 s ping and
    heartbeat schedule, and start-up and shut-down.
- **Tests:**
  - The TypeScript parser must make the same assertions on the same fixtures as
    `packages/scent-core/tests/fixtures.rs`:
    - every `mqtt` fixture parses with an action and a synthetic device ID;
    - the invalid-JSON notification is repaired;
    - the status-object fixture gives a status;
    - the ack fixtures carry `ADDED` and `ALREADY_ADDED`;
    - the `device_connected` fixtures carry their model information.

    The same goes for `parse_notification` on the `ble-notify` fixtures, because MQTT
    `notification` data uses the same text shapes.
  - The core:
    - newer-wins merging;
    - commands to a diffuser being ignored;
    - invalid IDs being ignored;
    - a retained schedule making a diffuser known;
    - a field older than `connected.at` reading as not known;
    - `GET_STATUS` going only to a reachable diffuser whose state is not known or is
      5 minutes old, and at most every 5 minutes.
  - The two gates:
    - the retained-publish gate refuses every topic outside our prefix (the Mac's list of
      foreign topics);
    - the command builder makes only `PING`, with the configured route ID, and
      `GET_STATUS`.
- **Deploying:** a new Railway service, `listener`, in the `scent-control` project, beside
  `test-broker`.
  - The image builds from the repo root with `apps/server/Dockerfile`, so it uses the root
    lockfile. The builder works out the Railway config for that.
  - `SCENT_BROKER_USERNAME` and `SCENT_BROKER_PASSWORD` are Railway reference variables to
    the broker's own (`${{test-broker.TEST_BROKER_USERNAME}}`,
    `${{test-broker.TEST_BROKER_PASSWORD}}`), so nobody types or copies the password.
  - `SCENT_BROKER_URL` and `SCENT_ROUTE_ID` take the values in the repo-root `.env`. The
    route ID isn't a secret, but it stays out of the repo.
  - It connects through the broker's public `wss://` address, the path the diffusers and
    apps already use. Railway's private network is a possible later change.
  - Deploying the listener never touches `test-broker`. Step 0 in §7 is the only broker
    change in this work.

### 6.8 The Mac app

The Mac already subscribes to `scent-control/v1/#`. It also learns to read the two new
topics:

- **A state record:**
  - for each field, the Mac adopts the listener's reading (as a WiFi reading) when it is
    newer than its own;
  - `heard_wifi` and `ping_at` take the later of its own and the listener's;
  - a diffuser it doesn't know yet becomes known, like one first heard on the broker;
  - "sent, waiting for the diffuser" still clears only on a report made after the send, as
    `pending_power` and `pending_intensity` work today.

  The Mac's clock and the listener's may differ by a second or two. The Mac still hears
  every report directly while it is open, so that doesn't hide a confirmation.
- **The heartbeat:** the broker section of Settings gets one line:
  - "Listener: running";
  - "Listener: not running, last seen 5 min ago";
  - "Listener: not seen yet".

  The wording and placement follow the Mac's design language. Nothing else changes when the
  listener isn't running: the Mac carries on with its own pings and readings, as today.
- **The reconnect rule (owner, 2026-10-22):**
  - the Mac applies §6.4's rule to its own readings as well as the listener's;
  - a power or intensity reported before the diffuser's last `device_connected` shows as
    not known, in the list and on its screen, until a newer report;
  - the Mac adopts the listener's `connected.at` when it is newer than its own
    `connected_at`, and the add flow, which waits for a `connected_at` after its restart,
    keeps working.
- **Tests** go in `state.rs`, next to the shared-schedule ones:
  - a newer listener reading is adopted and an older one is not;
  - a reading from before the last reconnect shows as not known;
  - an unknown diffuser is added;
  - another `v`, or a time more than a day ahead, is ignored;
  - the heartbeat counts as stale after 75 s.

## 7. Order of work

0. **Bring the broker up to date.**
   - **Why:** the running broker was deployed from the first `infra/test-broker` commit,
     which has `persistence false` and no packet limit. The commits that turned storage on
     and capped packets at 256 KiB say nothing was deployed. Until this step, any broker
     restart loses every retained message, and §6.1's packet limit isn't in force.
   - **What:** attach a Railway volume at `/mosquitto/data`, then redeploy the broker from
     the repo (`railway up infra/test-broker --path-as-root --service test-broker`). This
     is the volume, redeploy and persistence check the owner approved earlier. Confirm with
     the owner just before running it.
   - **Do it with both diffusers nearby.** The redeploy drops them, and Phase 0 never
     recorded whether a diffuser reconnects on its own after a broker restart.
     - A diffuser that isn't back within 5 minutes gets a soft restart over Bluetooth
       (`SECRET_COMMAND {"command":"restart"}`). Phase 0 saw that reconnect it.
     - Record in `docs/findings.md` whether each came back by itself, and how long it took.
   - **What's lost:** the retained messages on the broker now, such as the old probe and
     any stored schedules. The Mac keeps its own copy of each schedule.
   - **Then the persistence check:** a retained probe under `scent-control/v1/probe/…`
     must still be there after a broker restart. That restarts the broker a second time,
     so the same care applies.
1. **Prove the client.** In `apps/server`, show MQTT.js under Bun connecting over `wss://`
   to the test broker with the shared login, subscribing, and receiving a probe on
   `scent-control/v1/probe/…`. That is the riskiest unknown. If it fails, stop and bring it
   to the owner.
2. **The protocol port** and its fixture tests.
3. **The core**, with its tests.
4. **The shell**, deployed to Railway.
5. **The Mac change** (a Rust PR).
6. **The owner's check** (§8).

## 8. How we know it works

The owner's checks, with both diffusers powered and the Mac nearby:

1. **It's running.** Railway shows the listener running, and the Mac's Settings says
   "Listener: running".
2. **It knows the diffusers.** Open the Mac app. Within a minute,
   `tools/broker.py watch` shows a `scent-control/v1/state/<id>` for each diffuser.
3. **It catches changes nobody saw.**
   1. Quit the Mac app.
   2. Turn the Air 2 on at intensity 2 with `tools/broker.py publish`.
   3. Turn the Mac's Bluetooth off and open the app.
   4. It shows the Air 2 on, at intensity 2, with the time of the report.
4. **It sees a diffuser go away, and come back.**
   1. Unplug the HVAC 2. Within about 75 s the Mac shows it offline.
   2. Plug it back in. Once it is back, its power and intensity show as not known since it
      reconnected, until it reports or answers a `GET_STATUS`.
   3. `tools/broker.py watch` shows the listener asking within 5 minutes.
5. **The two open questions.** Record both in `docs/findings.md`, dated, as **[observed]**:
   - after a `SET_LEVEL` over WiFi, does an `update_status` arrive?
   - when the Air 2's own schedule starts and stops it, does it report?
6. **Stopping it is visible.** Stop the listener in Railway. Within 75 s the Mac says
   "Listener: not running", and it still works as before.
7. **Redeploying it doesn't touch the diffusers.** Redeploy the listener. The broker's log
   shows the diffusers stayed connected, and the state records are still there.

## 9. Risks

- **MQTT.js under Bun over `wss://`.** Task 1 exists to prove it.
- **Two pingers.** If a diffuser misbehaves with pings every 15 s on average, the Mac stops
  pinging while the listener is running.
- **A diffuser that never changes and never speaks.** It stays unknown until it says
  anything, and a ping reply is enough. Its state comes with its next report, or with an
  answer to `GET_STATUS`, which firmware 2.30 gives only sometimes.
- **A wrong `SCENT_ROUTE_ID`.** See §4. Check it against the Mac's value before the first
  deploy.

## 10. Left to the builder

- The module layout inside the core-and-shell split, and the Effect service boundaries.
- The Dockerfile, the Railway config, and merging publishes that come close together.
- The wording and placement of the Mac's heartbeat line, within its design language.
- Whether to add an integration test against a local Mosquitto.
