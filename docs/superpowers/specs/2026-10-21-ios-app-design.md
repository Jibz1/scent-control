# Scent Control for iPhone: design

Date: 2026-10-21. Status: approved by the owner, 2026-10-21. Revised 2026-10-23: the
phone app is all Swift (owner, 2026-10-22), it can copy a schedule from one diffuser to
another (owner, 2026-10-23), and the screens were approved from screenshots (owner,
2026-10-23). The revision replaces the shared Rust engine of the first version (§6, §7).

Decisions are marked **(owner)** when the project owner made them and **(designer's call)**
when the designer made them. Anything marked "builder's call" is left to whoever plans and
builds this. The main design is
[2026-10-20-scent-control-design.md](2026-10-20-scent-control-design.md); this document
changes its phases and its mobile stack, and nothing else. Device facts are in
[docs/findings.md](../../findings.md).

## 1. Why

The Mac app works, but it stays on the Mac. The owner wants the same control from their
phone, and chose to build the phone app next, before the server (owner, 2026-10-21).

- **Same features as the Mac app first (owner).** Nothing new on the phone until it does what
  the Mac does, apart from copying schedules (§2).
- **Native Swift, and only Swift on the phone (owner, 2026-10-22).** Not Expo or React Native,
  which the main design planned, and not the Mac's Rust code through UniFFI, which the first
  version of this design planned. The spike showed the Rust route works; the owner chose Swift.
- **Control from anywhere (owner, 2026-10-23).** Away from home WiFi, the phone reaches the
  diffusers through the broker, as the Mac does.
- **See the current schedule (owner, 2026-10-23).** A diffuser can't be asked for its schedule,
  so "current" means the last one sent from Scent Control, by the phone or the Mac, shared
  through the broker.
- **iPhone first (owner).** Android, if it comes, is a later and separate app.
- **The server comes after this (owner).** See §9 for what the owner said it must do.

## 2. Goals

The iPhone app does what the Mac app on `main` does (`apps/desktop`):

1. **Find diffusers.** Scan over Bluetooth and list nearby Scentiment diffusers with their
   model and color, as the Mac's sidebar does under Found Nearby.
   - **A diffuser the app already knows is recognized by its name (owner, 2026-10-21).** A
     diffuser's advertised name ends with its device ID ([observed], `docs/findings.md`).
     When a scan finds a diffuser whose device ID matches one the app knows, the app links
     that Bluetooth peripheral to it and sends it nothing. From then on Bluetooth control
     works, and it isn't listed under Found Nearby. This covers diffusers added by the Mac
     and ones known only from the broker.
2. **Add a diffuser.** The same flow as the Mac's Add sheet (`hub/add.rs`, `add_flow.rs`):
   - read `0xFEF4` first;
   - if the diffuser is already on WiFi, send only `SAVE_MQTT` to our broker, wait the full
     25 s, then restart it over Bluetooth;
   - otherwise ask for the WiFi name and password and send `WIFI_CONNECT` after `SAVE_MQTT`;
   - wait for it to connect (90 s), then `PING` with the route ID;
   - try again after a failure.
3. **Control.** Power and intensity 1–3 (default 1), over Bluetooth when the phone reaches the
   diffuser and over WiFi otherwise, with the Mac's rules: Bluetooth first, the 120 s
   cool-down only when WiFi is available, commands kept in order per diffuser, and a WiFi
   change shown as "sent, waiting for the diffuser" until a report confirms it. Send Over
   (Auto, Bluetooth or WiFi) and Read Status come too.
4. **Schedules.** The Mac's editor and week preview. A schedule goes out over WiFi as
   `UPDATE_PROFILE` (`schedule_flag 1`), then `SYNC_TIME` 5 s later. Once the broker accepts
   it, the app stores the shared copy (`scent-control/v1/schedule/<deviceId>`, retained), and
   it adopts a newer copy from another app. So the phone and the Mac agree on the last
   schedule sent, whichever sent it.
   - **Copy a schedule to other diffusers (owner, 2026-10-23).** From a diffuser's schedule,
     Copy to Another Diffuser picks one or more targets; each gets the whole table, exactly
     as Send Schedule sends it (replace, then the time), in its own queue, and its shared copy
     is updated. The times keep the source's time zone. Each target says whether its copy went
     through (designer's call).
5. **Status.** Each reading shows its source and age, and greys out after 5 minutes. Online
   means answering pings, sent every 30 s while the app is open. What the schedule says it
   should be doing now, the Today strip and the week appear alongside.
6. **Names and rooms**, as in the Mac's Details.
7. **Settings:** the broker (address, username, route ID, password), appearance (System,
   Light or Dark), and About with the app's version.
8. **The log**, written as sentences, with the raw (masked) message behind each line.

### Definition of done (designer's call)

- On the owner's iPhone 16 Pro Max, both diffusers can be found, added (or set up again),
  switched on and off, set to each intensity, and given a schedule, over Bluetooth and over
  WiFi, and from away from home over the broker.
- A schedule sent from the phone shows on the Mac, one sent from the Mac shows on the phone,
  and one copied from one diffuser runs on the other.
- The Mac app behaves exactly as before, and all its tests pass.
- The owner has run a phone version of `docs/phase-1-owner-session.md`, and the results are in
  `docs/findings.md`.

## 3. Non-goals

- The server, accounts, sharing with people, Home Assistant logins, and one broker login per
  diffuser. They come next (§9). The phone uses the test broker's shared login, as the Mac
  does.
- Android and iPad. The app is iPhone only, portrait (designer's call).
- Lights and oil level: these models don't support them on firmware 2.30.
- Working in the background. iOS suspends the app when it isn't open. The diffusers keep
  running their schedules regardless.
- Widgets, Live Activities, Siri and Control Center (designer's call: later, if wanted).
- TestFlight. The first builds install from Xcode onto the owner's phone.
- Syncing names and rooms between the Mac and the phone. Each app keeps its own until the
  server holds them (designer's call).
- Filling in the WiFi name automatically. It needs the Access WiFi Information entitlement
  and location permission; the person types it for now (builder's call, 2026-10-23).

## 4. Anti-goals: failures even if everything else works

- Everything in the main design's §4 and `CLAUDE.md`'s Rules still holds. In particular:
  - nothing contacts Scentiment;
  - no forbidden command can be built: the Swift `Command` enum is the same allow-list as
    `scent_core::Command`;
  - passwords never reach logs, files, the repo or command-line arguments;
  - there are no real device IDs in tracked files.
- **The Swift and Rust protocol code drifting apart.** With two codecs, the guard is shared
  fixtures (designer's call, 2026-10-23): both are tested against every captured message in
  `fixtures/scentiment/`, and against `fixtures/commands/commands.json`, the exact frames of
  every command, generated from the Rust code and checked by a Rust test, so a change on
  either side that alters a frame fails a test. The engine's behavior (routing, the add flow,
  pings, shared schedules, the log's wording) is ported from the Mac's code, file by file,
  with each Swift file naming its Rust source.
- **Breaking the Mac app.** Nothing in `apps/desktop` changes for the phone.
- **Disturbing the diffusers.** No change to topics, the broker, or what the diffusers are sent.

## 5. Constraints

- **Devices:** as in the main design §5. Bluetooth allows one connection at a time, so the
  phone and the Mac can't both hold a link to the same diffuser. Both drop idle links after
  10 s, which keeps that rare.
- **This Mac:** Xcode 27.1 with the iOS 27.1 SDK, Swift 6.4.
- **Test phone:** the owner's iPhone 16 Pro Max on iOS 27, already paired with this Mac. The
  owner's paid Apple Developer membership signs builds for it.
- **No third-party Swift packages in the app (designer's call).** Core Bluetooth for
  Bluetooth, URLSession's WebSocket and the system's TLS for the broker, the Security
  framework for the Keychain. Only the build tool XcodeGen is pinned, in `apps/ios/Tools`.
- **The repo stays shaped for open source:** no team ID, signing identity or personal data in
  tracked files.

## 6. How it works

### 6.1 Parts (designer's call)

```
packages/scent-core            unchanged; its frames are pinned in fixtures/commands
apps/ios/Packages/ScentProtocol  scent-core in Swift: codec, parsing, allow-list, schedules, online rules
apps/ios/Packages/ScentMQTT      MQTT 3.1.1 over wss://, no dependencies
apps/ios/Packages/ScentBluetooth Core Bluetooth: scan, read 0xFEF4 and 0xFEF3, write 0xDEAD in 177-byte chunks
apps/ios/Packages/ScentKit       ScentEngine: the Mac's hub, state, store, add flow and shared schedules, in Swift
apps/ios/Packages/ScentUI        the screens, in SwiftUI
apps/ios/App                     the thin app target
```

Each package builds and tests on its own. Everything but ScentUI also builds for the Mac,
so `swift test` runs those tests without a simulator.

### 6.2 The engine

- **Ported, not redesigned.** `ScentEngine` follows `apps/desktop/src`: `route.rs` (Route),
  `add_flow.rs` and `hub/add.rs` (AddFlow, the add in LiveEngine), `shared_schedule.rs`
  (SharedSchedule), `state.rs` (Ledger), `store.rs` (Store), `hub/mod.rs` (LiveEngine) and
  `ui/log_summary.rs` (LogWords).
- **One queue per diffuser**, so its commands run in order while different diffusers run side
  by side; the add runs as one job in its diffuser's queue.
- **The saved file** keeps the Mac's `desktop.json` shape (`state.json` in the app's
  Application Support folder), plus the phone's Send Over choice per diffuser.
- **The radios** (Bluetooth and the broker) sit behind one interface, so the engine's tests
  run against a fake, as the hub's do on the Mac.
- **The screens' model** (`AppModel`) is observable and on the main actor. The screens send
  it `Action`s; the engine carries them out and updates it. A sample engine plays the same
  actions on made-up data, for screenshots and previews.
- **The Keychain** holds the broker password (service `scent-control`, account
  `<username>@<host>`, as on the Mac), readable only while the phone is unlocked and never
  synced.
- **App lifecycle.** In the background the engine drops Bluetooth links and stops pinging;
  back in front, it reconnects to the broker if needed and pings again.

### 6.3 Screens (approved from screenshots, owner 2026-10-23)

The Mac app's look, in iPhone conventions (Apple's iOS Human Interface Guidelines, iOS 26's
Liquid Glass controls): system indigo accent, SF Pro Rounded for headlines and state words,
SF Pro for the rest, Dynamic Type, light and dark.

- **The room (designer's call).** A diffuser's screen opens on its room: a lavender dawn in
  light mode and a deep indigo night in dark mode while it diffuses, drained to grey when it's
  off. Mist rises from the diffuser's glyph, thicker for higher intensity. The power button
  and the Low/Medium/High control sit at its foot on glass. It is the one bold place; around
  it the screen is a plain grouped list.
- **Diffusers (the first screen).** Tiles grouped by room, two to a row; a diffusing one is
  lit with its room's dusk, and its power button switches it without opening it. Each says
  what it is doing and what its schedule does next. Find Nearby adds a Found Nearby section.
- **A diffuser.** The room, then Schedule (today's strip, the week, Edit, Copy to Another
  Diffuser), Status (power source, WiFi signal, last report, each with its age and source),
  Connection (Send Over, Read Status) and Details (name, room, model, firmware, device ID,
  Rename).
- **Sheets:** the schedule editor with a live week preview; Copy Schedule; Add Diffuser with
  the Mac's steps; Settings; the Activity log.
- **Motion (owner, 2026-10-23: "add animations"; designer's calls).** Only where it shows
  what changed: the zoom from a tile into its room; the mist thickening, thinning or rising
  away as the intensity or power changes; the room crossfading to grey; the intensity pill
  sliding; the power button's bounce with a haptic; "Sending…" while a change is on its way;
  check marks drawing themselves in as add and copy steps finish; the broker wait's ring
  draining. The mist is the only motion that runs on its own, and only while diffusing. With
  Reduce Motion on, the mist holds still and the room changes without a crossfade.
- **The icon.** The Mac's icon redrawn full-bleed for iOS, with dark and tinted versions
  (`apps/ios/scripts/app-icon.swift`).

### 6.4 Project and build (designer's call)

- **The Xcode project.** `apps/ios/project.yml` (XcodeGen, pinned in `apps/ios/Tools`)
  generates `ScentControl.xcodeproj` and `App/Info.plist`, both gitignored.
- **Language and tests.** Swift 6 language mode, iOS 26 minimum, Swift Testing.
- **Signing.** `Config/Base.xcconfig` is tracked. A gitignored `Config/Local.xcconfig` holds
  `DEVELOPMENT_TEAM`, with a tracked `Local.xcconfig.example`. The bundle ID is
  `dev.scentcontrol.app`.
- **Scripts.** `apps/ios/package.json` names its scripts `ios:*` (generate, build, test, fmt,
  screens), as HCCNA does. The repo's turbo tasks never include them.
- **Screens for review.** `scripts/screens.sh` renders each screen with sample data in light
  and dark on the simulator, through the app's `-screen` launch argument.

### 6.5 Testing

- `ScentProtocol` against the shared fixtures, and `scent-core` against
  `fixtures/commands/commands.json` (§4).
- `ScentMQTT`'s codec and client state machine against a fake WebSocket.
- `ScentBluetooth`'s pure parts (chunking, the scan filter, the link timer, queues, wording).
- `ScentEngine` against fake radios: routing and fallback, the cool-down, schedules and their
  shared copy, copying, adopting a newer copy, recognizing by name, the add, pings, and the
  saved file.
- The real proof is on the phone, with the diffusers: a phone version of
  `docs/phase-1-owner-session.md`.

## 7. Order (revised 2026-10-23)

1. ~~Spike on the phone~~ (done 2026-10-22; see `docs/findings.md`, "iPhone spike").
2. **The Mac recognizes known diffusers by name,** as its own pull request
   (`desktop-recognize-by-name`, built; the phone does the same).
3. **The screens, on sample data, for the owner's review** (done 2026-10-23).
4. **The protocol, the broker client, Bluetooth and the engine**, then the app on the real
   engine.
5. **The owner's session on the phone,** with the results in `docs/findings.md`.

## 8. Risks

- **Two codecs.** Covered by the shared fixtures (§4).
- **Our own MQTT client.** MQTT 3.1.1 at QoS 1 is small, but it is new code; its codec and
  state machine are tested, and the owner's session proves it against the broker.
- **The phone and the Mac both near a diffuser.** One Bluetooth link at a time means that one
  app's connection can fail while the other holds a link. The engine falls back to WiFi when
  Bluetooth fails.

## 9. After this: the server (owner, 2026-10-21; for its own design)

The owner described the server this way. It's recorded here so that this app doesn't paint
it into a corner.

- **Accounts.** The owner signs in on several devices.
- **Sharing.** The owner shares access with people and with Home Assistant
  (`apps/home-assistant`).
- **Broker logins.** They come from accounts: each signed-in app gets a login limited to its
  home's diffusers, and each diffuser keeps one login limited to its own topics. The current
  diffusers move to theirs over the broker, without a visit.
- **Still open:**
  - whether the server is written in Rust, sharing `scent-core`, or in the TypeScript stack of
    the main design;
  - how someone who is invited first gets in.
