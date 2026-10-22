# Scent Control for iPhone: design

Date: 2026-10-21. Status: approved by the owner, 2026-10-21.

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
  the Mac does.
- **Native Swift (owner).** Not Expo or React Native, which the main design planned.
- **iPhone first (owner).** Android, if it comes, is a later and separate app.
- **The server comes after this (owner).** See §9 for what the owner said it must do.

## 2. Goals

The iPhone app does what the Mac app on `main` does (`apps/desktop`):

1. **Find diffusers.** Scan over Bluetooth and list nearby Scentiment diffusers with their
   model and color, as the Mac's sidebar does under Found Nearby.
   - **A diffuser the app already knows is recognized by its name (owner, 2026-10-21).** A
     diffuser's advertised name ends with its device ID ([observed], `docs/findings.md`;
     `scent_core::device_id_from_name`).
     - **What happens:** when a scan finds a diffuser whose device ID matches one the app
       knows, the app links that Bluetooth peripheral to it. It sends the diffuser nothing.
       From then on Bluetooth control works, and the diffuser isn't listed under Found Nearby.
     - **Why:** this covers diffusers added by the other app, and ones known only from the
       broker. Without it, every app would have to set up every diffuser again, restarting it
       each time.
     - **The Mac gets this first (§7).** It also fixes the two diffusers the Mac shows today
       with no Bluetooth link.
2. **Add a diffuser.** The same flow as the Mac's Add sheet (`hub/add.rs`, `add_flow.rs`):
   - read `0xFEF4` first;
   - if the diffuser is already on WiFi, send only `SAVE_MQTT` to our broker, wait the full
     25 s, then restart it over Bluetooth;
   - otherwise ask for the WiFi name and password and send `WIFI_CONNECT` first;
   - wait for it to connect (90 s), then `PING` with the route ID;
   - Set Up Again for a diffuser that didn't finish.
3. **Control.** Power and intensity 1–3 (default 1), over Bluetooth when the phone reaches the
   diffuser and over WiFi otherwise, with the Mac's rules: Bluetooth first, the 120 s
   cool-down only when WiFi is available, commands kept in order per diffuser, and a WiFi
   change shown as "sent, waiting for the diffuser" until a report confirms it. The Mac's
   Send Over choice (Auto, Bluetooth or WiFi) and Read Status come too (designer's call:
   the owner's session uses them).
4. **Schedules.** The Mac's editor and week preview. A schedule goes out over WiFi as
   `UPDATE_PROFILE` (`schedule_flag 1`), then `SYNC_TIME` 5 s later. Once the broker accepts
   it, the app stores the shared copy (`scent-control/v1/schedule/<deviceId>`, retained), and
   it adopts a newer copy from another app. So the phone and the Mac agree on the last
   schedule sent, whichever sent it.
5. **Status.** Each reading shows its source and age, and greys out after 5 minutes. Online
   means answering pings, sent every 30 s while the app is open. "What the schedule says it
   should be doing now" and the Today strip appear alongside.
6. **Names and rooms**, as in the Mac's Details.
7. **Settings:** the broker (address, username, route ID, password), appearance (System,
   Light or Dark), and About with the app's version.
8. **The log**, written as sentences, with the raw message behind each line.

### Definition of done (designer's call)

- On the owner's iPhone 16 Pro Max, both diffusers can be found, added (or set up again),
  switched on and off, set to each intensity, and given a schedule, over Bluetooth and over
  WiFi.
- A schedule sent from the phone shows on the Mac, and one sent from the Mac shows on the
  phone.
- The Mac app behaves exactly as before, apart from recognizing known diffusers by name, and
  all its tests pass.
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
- TestFlight. The first builds install from Xcode onto the owner's phone. TestFlight comes when
  someone else tests it.
- Syncing names and rooms between the Mac and the phone. Each app keeps its own until the
  server holds them (designer's call).

## 4. Anti-goals: failures even if everything else works

- Everything in the main design's §4 and `CLAUDE.md`'s Rules still holds. In particular:
  - nothing contacts Scentiment;
  - no forbidden command can be built;
  - passwords never reach logs, files, the repo or command-line arguments;
  - there are no real device IDs in tracked files.
- **A second copy of the diffuser logic in Swift.** The protocol, the add flow, routing,
  pings, schedule sending and the screens' wording all come from the same Rust code the Mac
  uses. Swift draws the screens and owns what only iOS can do (§6.4). If the two apps can
  drift apart, the design has failed.
- **Breaking the Mac app.** Moving its engine into a shared library must not change what it
  does. Its existing tests move with the code and keep passing. The one change to its
  behavior is recognizing known diffusers by name (§2), made on its own beforehand (§7).
- **Disturbing the diffusers.** No change to topics, the broker, or what the diffusers are sent.

## 5. Constraints

- **Devices:** as in the main design §5. Bluetooth allows one connection at a time, so the
  phone and the Mac can't both hold a link to the same diffuser. Both drop idle links after
  10 s, which keeps that rare.
- **This Mac:** Xcode 27.1 with the iOS 27.1 SDK, and Rust 1.99.1. The Rust iOS targets
  (`aarch64-apple-ios`, `aarch64-apple-ios-sim`) aren't installed yet.
- **Test phone:** the owner's iPhone 16 Pro Max, already paired with this Mac. The owner's paid
  Apple Developer membership signs builds for it.
- **The Rust libraries already used support iOS.** `btleplug` 0.13.4 uses Core Bluetooth on
  iOS, the same code as on macOS, though it is less used there. `rumqttc` is plain Rust.
  Neither has been run on this phone (§8).
- **The repo stays shaped for open source:** no team ID, signing identity or personal data in
  tracked files.

## 6. How it works

### 6.1 Parts (designer's call)

```
packages/scent-core     unchanged: codec, parsing, allow-list, schedules, online rules
packages/scent-engine   new: the Mac app's engine and screen logic, moved, not rewritten
packages/scent-ffi      new: the engine as a Swift API, through UniFFI
apps/desktop            egui screens + macOS Keychain, on top of scent-engine
apps/ios                SwiftUI app, on top of scent-ffi
```

### 6.2 `scent-engine`: what moves out of `apps/desktop`

- **The engine:** the hub (`hub/`), the Bluetooth and MQTT workers (`ble.rs`, `mqtt.rs`),
  `route.rs`, `add_flow.rs`, `transport.rs`, `shared_schedule.rs`, `state.rs` and
  `store.rs`.
  - `Appearance` moves with `store.rs` as a plain enum. The egui-specific parts stay in
    `apps/desktop/src/ui/theme.rs`.
  - `store.rs` takes its directory from the app, since `dirs::data_dir()` means nothing on
    iOS. The file format doesn't change. The phone keeps its own `desktop.json` (the name
    can stay) in its Application Support folder.
- **The screen logic that has no egui in it.** It moves so that both apps say the same thing:
  - `ui/format.rs`: ages and reading text;
  - `ui/log_summary.rs`: log sentences, details and failure notes;
  - the pure parts of:
    - `ui/timeline.rs` (`day_plan`, `windows_for_day`, `now_fraction`, `day_summary`);
    - `ui/schedule_editor.rs` (`Draft`, `check`, `preview`, `parse_offset`);
    - `ui/device_panel.rs` (`detail_rows`, `expected_line`, and the hero's failure-note
      rule, which keeps the last failure until a newer one replaces it or an action
      succeeds);
    - `ui/add_sheet.rs` (`step_list`).
  - **The state the Mac keeps in its screen code** moves too (designer's call), so neither
    app reimplements it:
    - the Add sheet's progress;
    - the "sending" and "waiting for the diffuser" state;
    - the scan timers.
  - The builder may move more of this kind of code. Drawing stays in each app.
- **Folding events into state.** `app.rs` currently folds hub events into `AppState` and saves
  the store, and that moves into the engine as one `Engine` type, so neither app
  reimplements it. Today `app.rs` owns the hub, the state, the store, the broker settings and
  the screens. Afterwards it owns the screens and an `Engine`.
- **The Mac app afterwards:** `apps/desktop` keeps `main.rs`, `app.rs`, `ui/` (drawing) and
  `secrets.rs` (the macOS Keychain through `keyring`). Its tests either move with the code they
  test or stay with the drawing code.

### 6.3 `scent-ffi`: the Swift API

- **UniFFI**, with the bindings generated into Swift and the Rust built as a static library for
  the phone and the simulator, packaged as an XCFramework (designer's call). The exact types
  are the builder's call. The shape is:
  - **an `Engine` object.** `start(dataDir, broker settings, password?)`, then `submit(command)`
    for every user action, then `snapshot() -> Snapshot`;
  - **a listener the engine calls after every change.** Swift then reads a fresh snapshot on
    the main actor. This mirrors the Mac's `repaint` callback, and it keeps async Rust out of
    the Swift API;
  - **plain records for everything the screens show:**
    - devices with readings, sources and ages, and the expected line;
    - found diffusers;
    - add progress as the step list;
    - log sentences;
    - broker status;
    - the schedule draft's problems;
    - the Today strip's spans.
- **What never crosses from Rust to Swift:** passwords. Swift reads the broker password from the
  Keychain and hands it to `start` or a reconnect, and the WiFi password goes in with the add
  command. The rest of the code's rule holds: nothing that holds a secret prints it.

### 6.4 What Swift owns on iOS

- **The screens (SwiftUI)** and navigation.
- **The Keychain** (Security framework; designer's call), for the broker password. The Mac keeps
  `keyring`.
- **App lifecycle.**
  - When the app goes to the background, the engine is told to drop Bluetooth links and stop
    pinging.
  - When it returns, the engine reconnects to the broker if needed. Resubscribing on every
    ConnAck already covers that, and pings resume.
  - Exactly what is paused is the builder's call, within "nothing runs while suspended".
- **Permissions and Info.plist:**
  - `NSBluetoothAlwaysUsageDescription` for Bluetooth;
  - the Access WiFi Information entitlement with location permission when in use, so the
    add flow can fill in the WiFi name. If either is refused, the person types the name.
- **The WiFi name.** The engine takes it as input; Swift reads it.

### 6.5 Screens (designer's call)

The Mac app's look, translated into iPhone conventions (Apple's iOS Human Interface
Guidelines): system indigo accent, SF Pro, Dynamic Type, light and dark.

- **Diffusers (the first screen).** A list grouped by room. Each row shows the name, the model's
  glyph, and the status in words, never by color alone.
  - A Find Nearby button scans and shows a Found Nearby section. Tapping a found diffuser
    opens the Add sheet.
  - The toolbar has Settings and the log.
- **A diffuser.**
  - The hero: the diffuser glyph with mist when on, a state sentence, and the last failure.
  - The controls: a power toggle and an intensity control for 1–3.
  - The readings, each with its source and age, and the expected line.
  - The Today strip, and a Schedule row that opens the editor.
  - The name and room, and Set Up Again.
- **The Add sheet.** The step list with marks for done, in progress and failed. WiFi name and
  password fields appear only when the diffuser isn't on WiFi.
- **The schedule editor.** A sheet with the times, days and intensity, the week preview, and
  the same checks and limits as the Mac.
- **The log.** Sentences with the Mac's ↗ ↙ ! marks, and the detail when a line is tapped.
- **Settings.** The broker fields (the password in a secure field, saved to the Keychain),
  appearance, and About.
  - On the first launch with no broker, Settings opens by itself.
  - Afterwards, a notice is the way back to it, as on the Mac.
- **The glyphs** (the diffuser and its mist, the Today strip) are redrawn in SwiftUI from the
  Mac's shapes in `ui/glyphs.rs` and `ui/hero.rs`. SF Symbols are used where one fits.

### 6.6 Project and build (designer's call, following HCCNA's `apps/mobile-native`)

- **The Xcode project.** `apps/ios/project.yml` (XcodeGen) generates
  `apps/ios/ScentControl.xcodeproj` and `App/Info.plist`, and both are gitignored.
  - XcodeGen is pinned in `apps/ios/Tools/Package.swift`.
  - The app target is thin. The feature code lives in a local Swift package
    (`apps/ios/Packages/ScentKit`), one module per concern (names are the builder's call).
- **Language and tests.** Swift 6 language mode, iOS 18 minimum, Swift Testing for tests, and
  `swift format`.
- **Signing.** `Config/Base.xcconfig` is tracked. A gitignored `Config/Local.xcconfig` holds
  `DEVELOPMENT_TEAM`, with a tracked `Local.xcconfig.example`. The bundle ID is the
  builder's call, and must not contain personal data.
- **The Rust core.** A script builds it for `aarch64-apple-ios` and `aarch64-apple-ios-sim`,
  generates the Swift bindings, and assembles the XCFramework into the Swift package. The
  XCFramework is gitignored and rebuilt by that script.
- **Scripts.** `apps/ios/package.json` names its scripts `ios:*` (generate, build, test, fmt,
  core), as HCCNA does. The repo's turbo tasks, which run where Xcode may not exist, never
  include them. `scent-engine` and `scent-ffi` join the Cargo workspace, and their
  `package.json` scripts wrap `cargo`, like the other crates.
- **Screens for review.** A way to render each screen in light and dark on the simulator, as
  the Mac's `render_screens` does, so pull requests can show screenshots (builder's call: an
  XCUITest that exports screenshots, or `ImageRenderer` in a test).

### 6.7 Testing

- The engine's existing tests move with it, and the Mac app's remaining tests still pass:
  `cargo test --workspace`, clippy with `-D warnings`, and fmt.
- `scent-ffi` has Rust tests that drive an `Engine` through its public API against fakes, as the
  hub's tests do now.
- The Swift side has tests for what it adds: Keychain wrapping (in the simulator), mapping
  snapshots to views, and lifecycle handling.
- The real proof is on the phone, with the diffusers: a phone version of
  `docs/phase-1-owner-session.md`.

## 7. Order (owner, 2026-10-21: spike now, and the engine moves only after the Mac's adding is proven)

1. **Spike on the phone, to prove what's untested** (throwaway, not reviewed as product code).
   It starts at once, since it touches nothing in the Mac app.
   A minimal Rust library through UniFFI in a bare SwiftUI app on the iPhone that:
   - scans with `btleplug` and lists the two diffusers;
   - reads `0xFEF4`;
   - connects to the test broker over `wss://` with `rumqttc` and `rustls`, using whichever
     root certificates work on iOS (§8).
   If `btleplug` fails on iOS, Bluetooth moves to Swift: a Core Bluetooth layer behind a
   transport interface the engine calls. The rest of the design stays. Record what's learned
   in `docs/findings.md`.
2. **The Mac recognizes known diffusers by name (§2),** in `apps/desktop`, as its own pull
   request.
3. **The owner finishes the Mac session's adding and control steps** on the real diffusers
   (`docs/phase-1-owner-session.md`), with that change in. Fixes it turns up land in
   `apps/desktop`, before anything moves.
4. **Move the engine into `scent-engine`,** with the Mac app on top and its tests green. This
   is its own pull request.
5. **`scent-ffi`, and the script that builds the XCFramework.**
6. **The iPhone app:**
   - the project, Settings and the Keychain;
   - the list, scanning, and a diffuser's screen with its controls;
   - the Add sheet;
   - schedules;
   - the log, appearance and About.
7. **The owner's session on the phone,** with the results in `docs/findings.md`.

Update `CLAUDE.md` and the main design's phases to match: `apps/ios` in place of `apps/mobile`,
and the new order.

## 8. Risks

- **`btleplug` on iOS** has the least use of anything here. The spike settles it, and §7 says
  what changes if it fails.
- **TLS root certificates on iOS.** The Mac loads the system's roots. Whether the same crate
  works on iOS, or the app should bundle roots or use the platform verifier, is settled in the
  spike (builder's call).
- **UniFFI and async.** The Swift API stays synchronous plus a listener (§6.3), so no async
  crosses the boundary. If UniFFI's version makes that awkward, the builder may choose
  another shape that keeps the same rule.
- **The phone and the Mac both near a diffuser.** One Bluetooth link at a time means that one
  app's connection can fail while the other holds a link. The engine already falls back to
  WiFi when Bluetooth fails.

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
  - whether the server is written in Rust, sharing `scent-engine` and `scent-core`, or in the
    TypeScript stack of the main design;
  - how someone who is invited first gets in.
