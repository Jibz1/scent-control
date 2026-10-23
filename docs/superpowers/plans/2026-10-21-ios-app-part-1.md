# Scent Control for iPhone, part 1: the spike and recognizing diffusers by name

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that the Mac's Rust code can drive Bluetooth and the broker from the owner's
iPhone, and make the Mac recognize diffusers it already knows by the device ID in their
Bluetooth name.

**Architecture:**
- **The spike** is throwaway, on its own branch, and never merged. A standalone Rust crate,
  exported to Swift with UniFFI and packaged as an XCFramework, sits under a bare SwiftUI app
  that runs on the phone. Only its findings land, in `docs/findings.md`.
- **The Mac change** is one TDD change to how `AppState` folds a scan. It's in `apps/desktop`,
  as its own pull request.

**Tech Stack:** Rust 1.99.1, btleplug, rumqttc (wss + rustls), tokio, UniFFI, Swift 6, SwiftUI,
XcodeGen, Xcode 27.1 (iOS 27.1 SDK).

**Spec:** `docs/superpowers/specs/2026-10-21-ios-app-design.md` (approved 2026-10-21). This plan
covers its §7 steps 1–3. Steps 4–7 are part 2, written after the spike, because the spike
decides how Bluetooth and TLS work on the phone (§7, §8).

## Global Constraints

- Never contact Scentiment's servers. Never use, store or print credentials found in
  Scentiment's app.
- Never send `UPDATE_FIRMWARE`, `U_FW`, `RESET` other than `{"mode":"soft"}`, or
  `SECRET_COMMAND` other than `{"command":"restart"}`. The spike sends a diffuser no command
  at all: its only Bluetooth operation is reading `0xFEF4`.
- WiFi and broker passwords never go in the repo, logs, captures, fixtures, stdout or
  command-line arguments. In the spike, the broker password is typed into a `SecureField` on
  the phone and held only in memory.
- No real device IDs, Apple team IDs, device UDIDs or personal names in tracked files, commit
  messages or PR bodies.
  - Tests use synthetic IDs: `TESTDEVICE01` style, or `0A1B2C3D4E5F` and `0A1B2C3D4E60` where
    a 12-hex-digit ID is needed. Only hex IDs are found in Bluetooth names.
  - The spike prints whether a name carries a device ID, never the ID itself.
- Record observed device behavior in `docs/findings.md` with a date and the **[observed]** tag.
- Never delete, recreate, restart or redeploy the Railway `test-broker` service.
- The spike's broker messages go only to `scent-control/v1/probe/ios/<8 random hex digits>`,
  at QoS 1, never retained. Nothing is published under `scentiment-mqtt-prod/`.
- **Versions, exact:**
  - `btleplug = "=0.13.4"`;
  - `rumqttc = { version = "=0.25.1", default-features = false, features = ["websocket", "use-rustls"] }`;
  - `tokio = "=1.53.2"`, `futures = "=0.3.34"`, `uuid = "=1.27.0"`;
  - `uniffi = "=0.32.2"`;
  - XcodeGen 2.46.0, pinned in a `Tools/Package.swift` and run with
    `swift run --package-path Tools xcodegen`, as HCCNA's `apps/mobile-native/ios` does;
  - Swift 6 language mode, iOS deployment target 18.0.
- **Bundle IDs** start `dev.scentcontrol.` (the spike is `dev.scentcontrol.spike`). The team
  ID goes only in a gitignored `Local.xcconfig`.
- **Bluetooth allows one connection at a time.** During the spike, the Mac app is quit and
  Scentiment's app is closed.
- **Commits** end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. **PR
  bodies** end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

## Review Focus

1. **Two scan hits advertising the same device ID** (a stray or spoofed advertisement): link
   neither, so a guess never sends Bluetooth commands to the wrong peripheral. Covered by Task 3,
   `two_hits_with_the_same_id_link_neither`.
2. **A peripheral linked to diffuser A now advertises diffuser B's ID:** the link moves to B and
   A loses it, so two diffusers never share one peripheral. Covered by Task 3,
   `a_bluetooth_id_moves_to_the_diffuser_its_name_names`.
3. **The ID in the name is lower-case hex:** it still links, because device IDs are stored
   upper case. Covered by Task 3, `a_lower_case_id_in_the_name_still_links`.
4. **A scan finds a diffuser the app doesn't know:** it stays under Found Nearby for Add, and no
   diffuser is created. Covered by Task 3, `a_scan_never_adds_a_diffuser_it_does_not_know` and
   `a_known_diffuser_found_by_name_is_not_found_nearby`.
5. **On iOS, loading the system root certificates panics or finds none:** the spike shows an
   error and doesn't crash, as the Mac's `build_options` already arranges with `catch_unwind`.
   Covered by Task 2, Step 9.

---

### Task 1: Point the docs at the iPhone app

Branch `ios-app-design` (it holds the design and this plan).

**Files:**
- Modify: `CLAUDE.md`
- Modify: `docs/superpowers/specs/2026-10-20-scent-control-design.md` (the end of "Revised
  phases")

- [ ] **Step 1: Edit `CLAUDE.md`**
  - **Current plan:** the bullet becomes this plan's path,
    `docs/superpowers/plans/2026-10-21-ios-app-part-1.md`. Add a bullet for the iPhone design,
    `docs/superpowers/specs/2026-10-21-ios-app-design.md`.
  - **Stack:** replace "Expo SDK 57 (custom dev build for Bluetooth)" with: the iPhone app is
    native SwiftUI over the shared Rust engine through UniFFI (see the iPhone design), and the
    server's language is still open.
  - **Layout:** replace the `apps/mobile` line with `apps/ios  SwiftUI iPhone app (next; see
    the iPhone design)`. Leave `apps/server`.
  - **Phases:** under 1, add one line: "Next: the iPhone app
    (`docs/superpowers/specs/2026-10-21-ios-app-design.md`), then the server."

- [ ] **Step 2: Append to the main design's "Revised phases"**

  Add one bullet: "**2026-10-21 (owner):** the iPhone app comes next, before the server, in
  native Swift over the Mac's engine. See
  [2026-10-21-ios-app-design.md](2026-10-21-ios-app-design.md). It replaces the open
  React Native or SwiftUI question above."

- [ ] **Step 3: Verify**

  Run: `grep -n 'apps/mobile\|Expo' CLAUDE.md; grep -c '2026-10-21-ios-app' CLAUDE.md docs/superpowers/specs/2026-10-20-scent-control-design.md`
  Expected: no `apps/mobile` or `Expo` lines. The count is at least 2 for `CLAUDE.md` and 1 for
  the spec.

- [ ] **Step 4: Commit** ("Docs: point CLAUDE.md and the main design at the iPhone app")

---

### Task 2: The iPhone spike (throwaway)

This branch is never merged. Its code is not reviewed as product code. What gets reviewed is
the findings entry (Step 11), which lands on `ios-app-design`.

**Branch:** `spike/ios-core`, cut from `ios-app-design`.

**Files:**
- Create: `spikes/ios-core/README.md`: what this is, that it's throwaway, and Steps 5–9 as
  commands.
- Create: `spikes/ios-core/.gitignore`: `rust/target/`, `build/`, `ios/*.xcodeproj/`,
  `ios/App/Info.plist`, `ios/Config/Local.xcconfig`, `ios/Tools/.build/`.
- Create: `spikes/ios-core/rust/Cargo.toml`, `rust/src/lib.rs`, `rust/uniffi-bindgen.rs`
- Create: `spikes/ios-core/build-core.sh`
- Create: `spikes/ios-core/ios/project.yml`, `ios/Tools/Package.swift`,
  `ios/Tools/Sources/Tools/Empty.swift`, `ios/Config/Base.xcconfig`,
  `ios/Config/Local.xcconfig.example`, `ios/App/SpikeApp.swift`, `ios/App/ContentView.swift`
- Modify, on `ios-app-design`: `docs/findings.md`

**Interfaces:**
- **Consumes:** `scent_core::{is_scentiment_name, parse_name, device_id_from_name, parse_info}`
  (path dependency `../../../packages/scent-core`). The Mac's patterns, copied rather than
  imported:
  - `apps/desktop/src/ble.rs`'s info characteristic `0000fef4-0000-1000-8000-00805f9b34fb`;
  - `apps/desktop/src/mqtt.rs`'s `build_options`: the URL passed whole as the broker address,
    and `catch_unwind(Transport::wss_with_default_config)`.
- **Produces:** findings only. Part 2 reads two decisions from them:
  - **Bluetooth:** Rust (btleplug) or Swift (Core Bluetooth);
  - **Roots:** the system's (rustls-native-certs) or bundled (webpki-roots).

- [ ] **Step 1: Install the iOS targets.**
  Run: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
  Expected: both listed by `rustup target list --installed`.

- [ ] **Step 2: Write `rust/Cargo.toml`.**
  - The package is `spike-core`, with `[lib] name = "spike_core"` and
    `crate-type = ["staticlib", "cdylib"]`.
  - It has its own empty `[workspace]` table, so the root Cargo workspace ignores it.
  - Dependencies are the Global Constraints' versions, plus
    `uniffi = { version = "=0.32.2", features = ["cli"] }` and the `scent-core` path dependency.
  - Add `webpki-roots`, pinned with `=` to the release that uses the same `rustls-pki-types` as
    the root `Cargo.lock` (whose rustls is 0.23.46).
  - Add `[[bin]] name = "uniffi-bindgen", path = "uniffi-bindgen.rs"`, whose `main` calls
    `uniffi::uniffi_bindgen_main()`.

- [ ] **Step 3: Write `rust/src/lib.rs`: these exports and nothing else.**

  ```rust
  uniffi::setup_scaffolding!();

  #[derive(uniffi::Record)]
  pub struct SpikeHit { pub peripheral: String, pub name: String, pub rssi: Option<i16>, pub has_device_id: bool }

  #[derive(uniffi::Enum)]
  pub enum Roots { System, Bundled }

  #[derive(Debug, uniffi::Error)]
  pub enum SpikeError { Failed { message: String } }   // plus Display and std::error::Error

  #[uniffi::export] pub fn scan(seconds: u32) -> Result<Vec<SpikeHit>, SpikeError>;
  #[uniffi::export] pub fn read_info(peripheral: String) -> Result<String, SpikeError>;
  #[uniffi::export] pub fn broker_round_trip(url: String, username: String, password: String, roots: Roots) -> Result<String, SpikeError>;
  ```

  - **The runtime.** Every export blocks on one shared multi-thread tokio runtime, kept in a
    `OnceLock`. Swift calls them off the main thread.
  - **`scan`:**
    - starts a btleplug scan on the first adapter for `seconds` and keeps hits whose name
      passes `is_scentiment_name`;
    - remembers each `Peripheral` in a `Mutex<HashMap<String, Peripheral>>`. The key is the
      Mac's text for a peripheral ID: copy `ble_id_of` from `apps/desktop/src/ble.rs`
      (about line 125);
    - sets `has_device_id` to `device_id_from_name(&name).is_some()`.
  - **`read_info`:**
    - connects to the remembered peripheral, discovers services, and reads `0xFEF4`;
    - disconnects, waiting at most 3 s, as the Mac's `disconnect_quietly` does;
    - returns `"<model from parse_name>, firmware <firmware>, WiFi configured <bool>, status
      <n>"` from `parse_info`;
    - on a parse error returns `Failed { message: "couldn't parse 0xFEF4" }`, never the raw
      text, which holds the device ID.
  - **`broker_round_trip`:**
    - **Connect.** `MqttOptions::new("scent-spike-<8 hex>", url, 443)`, with keep-alive 30 s
      and the credentials.
      - `Roots::System` uses `catch_unwind(Transport::wss_with_default_config)`, and turns a
        panic into `Failed { message: "couldn't load the system's root certificates" }`.
      - `Roots::Bundled` uses `Transport::wss_with_config` with a rustls `ClientConfig` built
        from `webpki_roots::TLS_SERVER_ROOTS`.
    - **Round trip.** Waits for ConnAck (10 s), subscribes to the probe topic (Global
      Constraints), and publishes `{"probe":"ios"}` once.
    - **Returns** `"connected in <ms> ms, round trip <ms> ms"` when the message comes back
      within 10 s. Otherwise it returns the rumqttc error's text, which never holds the
      password.
  - **Never print** a name, a device ID or the password. `println!` only the same summary
    strings the exports return, so `devicectl --console` shows them.

- [ ] **Step 4: Write `build-core.sh` (`set -eu`, run from `spikes/ios-core`).** In order:
  1. `cargo build --release --target aarch64-apple-ios` in `rust/`;
  2. `cargo run --bin uniffi-bindgen -- generate --library rust/target/aarch64-apple-ios/release/libspike_core.a --language swift --out-dir build/generated`.
     If bindgen rejects the `.a`, build the host `cdylib` with `cargo build --release` and
     generate from `rust/target/release/libspike_core.dylib` instead;
  3. copy `spike_coreFFI.h` to `build/headers/`, and `spike_coreFFI.modulemap` to
     `build/headers/module.modulemap`;
  4. `rm -rf build/SpikeCore.xcframework`, then
     `xcodebuild -create-xcframework -library rust/target/aarch64-apple-ios/release/libspike_core.a -headers build/headers -output build/SpikeCore.xcframework`.

  Run: `sh build-core.sh`
  Expected: `build/SpikeCore.xcframework/Info.plist` and `build/generated/spike_core.swift`
  exist.

- [ ] **Step 5: Write the Xcode project spec and the app.**
  - **`ios/Tools/Package.swift`:** HCCNA's file, verbatim apart from the package's name and
    path, pinning `https://github.com/yonaskolb/XcodeGen.git` `exact: "2.46.0"`. Add an empty
    `Sources/Tools/Empty.swift`.
  - **`ios/project.yml`:**
    - project `ScentSpike`, `deploymentTarget: iOS: "18.0"`, `SWIFT_VERSION: "6.0"`;
    - `configFiles` Debug and Release pointing at `Config/Base.xcconfig`, which ends with
      `#include? "Local.xcconfig"`;
    - one application target `ScentSpike`, whose sources are `App` and
      `../build/generated/spike_core.swift`;
    - dependencies: `framework: ../build/SpikeCore.xcframework` (`embed: false`),
      `sdk: CoreBluetooth.framework` and `sdk: Security.framework`;
    - `PRODUCT_BUNDLE_IDENTIFIER: dev.scentcontrol.spike` and `CODE_SIGN_STYLE: Automatic`;
    - Info.plist properties: `NSBluetoothAlwaysUsageDescription: "Scent Control uses
      Bluetooth to find and talk to your diffusers."`, `UILaunchScreen: {}`, and portrait
      only.
  - **`Config/Local.xcconfig.example`:** says to copy it to `Local.xcconfig` and set
    `DEVELOPMENT_TEAM`.
  - **`ContentView.swift`:** one screen.
    - A Scan button runs `scan(seconds: 5)` and lists the hits (name, RSSI, "has ID"
      yes/no). Tapping a hit runs `readInfo` and shows the result under it.
    - A Broker section has URL and Username text fields and a `SecureField` for the password.
      Its two buttons are "System roots" and "Bundled roots", each running
      `brokerRoundTrip` and showing the result or the error text.
    - Every call runs in `Task.detached`, and its result is shown on the main actor.

- [ ] **Step 6: Generate the project and build it for a device.**
  - **The team ID** goes in `Config/Local.xcconfig` without being printed:

    ```bash
    security find-certificate -c "Apple Development" -p | openssl x509 -noout -subject -nameopt multiline | awk '/organizationalUnitName/ {print "DEVELOPMENT_TEAM = " $3}' > ios/Config/Local.xcconfig
    ```

  - **Generate and build** in `ios/`:

    ```bash
    swift run --package-path Tools xcodegen generate --spec project.yml --quiet
    xcodebuild build -project ScentSpike.xcodeproj -scheme ScentSpike -configuration Debug -destination 'generic/platform=iOS' -derivedDataPath ../build/DerivedData -allowProvisioningUpdates -quiet
    ```

  Expected: `../build/DerivedData/Build/Products/Debug-iphoneos/ScentSpike.app` exists.
  - **A link error naming a missing framework or symbol:** add that framework as another
    `sdk:` dependency, and note it for the findings.

- [ ] **Step 7: Install it on the owner's iPhone and launch it with its console.**
  Get the paired iPhone 16 Pro Max's identifier from `xcrun devicectl list devices` at run
  time, and never write it to a file. Then:

  ```bash
  xcrun devicectl device install app --device <identifier> ../build/DerivedData/Build/Products/Debug-iphoneos/ScentSpike.app
  xcrun devicectl device process launch --device <identifier> --console dev.scentcontrol.spike
  ```

  Expected: the app opens on the phone.
  - **If iOS says the developer isn't trusted, or Developer Mode is off:** stop. Ask the owner
    to allow it on the phone, under Settings › General › VPN & Device Management, or Settings
    › Privacy & Security › Developer Mode. Then launch again.

- [ ] **Step 8: Owner step: Bluetooth.** The owner is near both diffusers, with the Mac app
  quit and Scentiment's app closed.
  1. They tap Scan and allow Bluetooth.
     Expected: two hits, one name starting `Scentiment-D` and one `Scentiment-H`, both with
     "has ID: yes".
  2. They tap each hit.
     Expected: `Air 2, firmware 2.30, …` and `HVAC 2, firmware 2.30, …`, or the error shown.

  Record exactly what the screen and the console show.

- [ ] **Step 9: Owner step: the broker.**
  1. The owner enters the broker address and username, as shown in the Mac app's Settings,
     and types the password.
  2. They tap System roots, then Bundled roots.
     Expected: each shows `connected in … ms, round trip … ms` or an error message. The app
     doesn't crash in either case (Review Focus 5).

  Record both results.

- [ ] **Step 10: Commit the spike on `spike/ios-core`** ("Spike: Rust Bluetooth and broker on
  iPhone through UniFFI (throwaway)").
  - Run `git status` first: `Local.xcconfig`, `build/` and the generated project must not be
    staged.
  - Don't push unless the owner asks.

- [ ] **Step 11: Write the findings on `ios-app-design`.** In `docs/findings.md`, add a section
  "iPhone spike (<date>)". Every bullet is tagged **[observed]** and holds no IDs or UDIDs. It
  covers:
  - the iOS version and the phone model;
  - whether btleplug scanned and read `0xFEF4`, with the messages seen;
  - System roots and Bundled roots: worked, or the error;
  - whether UniFFI 0.32.2 and a static-library XCFramework worked, and any frameworks the
    linker needed;
  - the two decisions, stated as lines Part 2 can quote:
    - "Bluetooth on iPhone: Rust (btleplug)" or "…: Swift (Core Bluetooth)";
    - "Root certificates on iPhone: system" or "…: bundled".

  Commit ("Findings: the iPhone spike").

---

### Task 3: The Mac recognizes known diffusers by name

**Branch:** `desktop-recognize-by-name`, cut from `origin/main`, in its own worktree. It's its
own pull request.

**Files:**
- Modify: `apps/desktop/src/state.rs`:
  - the `Event::Scanned` arm of `apply_at` (about line 406);
  - a new method beside `on_ble_info` (about line 761);
  - tests in its `mod tests`.
- Modify: `apps/desktop/src/ui/device_list.rs`: one test in its `mod tests` (about line 534).
- Modify: `docs/phase-1-owner-session.md`: step 5.

**Interfaces:**
- **Consumes:**
  - `scent_core::device_id_from_name(name: &str) -> Option<String>`, which returns upper case
    and only for 12 hex digits;
  - the existing `ScanHit { ble_id, name, rssi }`, `AppState.devices: BTreeMap<String, DeviceView>`
    and `DeviceView.ble_id: Option<String>`.
- **Produces:** `AppState::link_known_hits(&mut self, hits: &[ScanHit])`, which is private.
  Part 2 moves it into `scent-engine` with the rest of `state.rs`.

- [ ] **Step 1: Write the failing tests in `state.rs`'s `mod tests`.**

  ```rust
  const HEX1: &str = "0A1B2C3D4E5F";
  const HEX2: &str = "0A1B2C3D4E60";

  fn named_hit(ble: &str, id: &str) -> ScanHit {
      ScanHit { ble_id: ble.to_owned(), name: format!("Scentiment-D B n {id}"), rssi: Some(-60) }
  }

  fn knowing(ids: &[&str]) -> AppState {
      let mut s = AppState::default();
      for id in ids {
          s.devices.insert((*id).to_owned(), DeviceView::new(id));
      }
      s
  }

  #[test]
  fn a_scan_links_a_known_diffuser_by_the_id_in_its_name() {
      let mut s = knowing(&[HEX1]);
      s.apply_at(Event::Scanned(vec![named_hit("ble-a", HEX1)]), NOW);
      assert_eq!(s.devices[HEX1].ble_id.as_deref(), Some("ble-a"));
      assert_eq!(s.scan, vec![named_hit("ble-a", HEX1)], "the scan list is still every hit");
  }

  #[test]
  fn a_lower_case_id_in_the_name_still_links() {
      let mut s = knowing(&[HEX1]);
      s.apply_at(Event::Scanned(vec![named_hit("ble-a", &HEX1.to_ascii_lowercase())]), NOW);
      assert_eq!(s.devices[HEX1].ble_id.as_deref(), Some("ble-a"));
  }

  #[test]
  fn a_scan_link_replaces_a_stale_bluetooth_id() {
      let mut s = knowing(&[HEX1]);
      s.devices.get_mut(HEX1).unwrap().ble_id = Some("ble-old".to_owned());
      s.apply_at(Event::Scanned(vec![named_hit("ble-new", HEX1)]), NOW);
      assert_eq!(s.devices[HEX1].ble_id.as_deref(), Some("ble-new"));
  }

  #[test]
  fn a_scan_never_adds_a_diffuser_it_does_not_know() {
      let mut s = knowing(&[HEX1]);
      s.apply_at(Event::Scanned(vec![named_hit("ble-b", HEX2)]), NOW);
      assert!(!s.devices.contains_key(HEX2));
      assert_eq!(s.devices[HEX1].ble_id, None);
  }

  #[test]
  fn two_hits_with_the_same_id_link_neither() {
      let mut s = knowing(&[HEX1]);
      s.devices.get_mut(HEX1).unwrap().ble_id = Some("ble-old".to_owned());
      s.apply_at(Event::Scanned(vec![named_hit("ble-a", HEX1), named_hit("ble-b", HEX1)]), NOW);
      assert_eq!(s.devices[HEX1].ble_id.as_deref(), Some("ble-old"));
  }

  #[test]
  fn a_bluetooth_id_moves_to_the_diffuser_its_name_names() {
      let mut s = knowing(&[HEX1, HEX2]);
      s.devices.get_mut(HEX1).unwrap().ble_id = Some("ble-a".to_owned());
      s.apply_at(Event::Scanned(vec![named_hit("ble-a", HEX2)]), NOW);
      assert_eq!(s.devices[HEX2].ble_id.as_deref(), Some("ble-a"));
      assert_eq!(s.devices[HEX1].ble_id, None);
  }

  #[test]
  fn linking_is_not_hearing_from_the_diffuser() {
      let mut s = knowing(&[HEX1]);
      let before = s.devices[HEX1].clone();
      s.apply_at(Event::Scanned(vec![named_hit("ble-a", HEX1)]), NOW);
      let after = s.devices[HEX1].clone();
      assert_eq!(DeviceView { ble_id: None, ..after }, before, "only the Bluetooth ID changes");
      assert!(s.log.is_empty(), "a scan sends and hears nothing, so it logs nothing");
  }
  ```

- [ ] **Step 2: Write the failing test in `device_list.rs`'s `mod tests`.**

  ```rust
  #[test]
  fn a_known_diffuser_found_by_name_is_not_found_nearby() {
      let mut state = AppState::default();
      state.devices.insert("0A1B2C3D4E5F".to_owned(), DeviceView::new("0A1B2C3D4E5F"));
      let hit = |ble: &str, name: &str| ScanHit { ble_id: ble.to_owned(), name: name.to_owned(), rssi: Some(-60) };
      state.apply_at(
          Event::Scanned(vec![
              hit("ble-known", "Scentiment-D B n 0A1B2C3D4E5F"),
              hit("ble-new", "Scentiment-H B n 0A1B2C3D4E60"),
          ]),
          5_000,
      );
      let names: Vec<&str> = unknown_hits(&state).iter().map(|hit| hit.name.as_str()).collect();
      assert_eq!(names, ["Scentiment-H B n 0A1B2C3D4E60"]);
  }
  ```

  `device_list.rs`'s test module imports `super::*`. Add `use crate::state::Event;` there if
  `Event` isn't already in scope.

- [ ] **Step 3: Run them to see them fail.**
  Run: `cargo test -p scent-desktop -- by_the_id_in_its_name lower_case_id stale_bluetooth_id never_adds_a_diffuser same_id_link_neither moves_to_the_diffuser not_hearing_from found_by_name`
  Expected: the link tests fail, with `ble_id` `None` where `Some("ble-a")` was expected, and
  so on. `a_scan_never_adds…` and `two_hits…` may already pass, since today nothing links.

- [ ] **Step 4: Implement `fn link_known_hits(&mut self, hits: &[ScanHit])` in `state.rs`.**
  - The `Event::Scanned(hits)` arm becomes `{ self.link_known_hits(&hits); self.scan = hits; }`.
  - **The rule:** take the device IDs named by the hits (`device_id_from_name(&hit.name)`).
    For each ID named by exactly one hit and present in `self.devices`:
    - set any *other* device whose `ble_id` equals that hit's `ble_id` to `None`;
    - then set this device's `ble_id` to the hit's `ble_id`.
  - Nothing else changes: no log entry, `heard_ble`, info or readings, and no new devices.
  - Give it a doc comment saying why: a diffuser's advertised name ends with its device ID (see
    `docs/findings.md`), so a diffuser known from the broker or from another app can be
    reached over Bluetooth without adding it again. An ID named twice is ambiguous and links
    nothing.

- [ ] **Step 5: Run them to see them pass, then the whole check.**
  Run: `cargo test -p scent-desktop -- by_the_id_in_its_name lower_case_id stale_bluetooth_id never_adds_a_diffuser same_id_link_neither moves_to_the_diffuser not_hearing_from found_by_name`
  Expected: 8 passed.
  Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check`
  Expected: all pass, with clippy clean and fmt clean.

- [ ] **Step 6: Add to step 5 of `docs/phase-1-owner-session.md`.** Append this sentence:
  "Diffusers the app already knows (from the broker, or added before) aren't listed under Found
  Nearby: the scan recognizes them by the device ID in their Bluetooth name, and Send Over
  Bluetooth reaches them without adding them again."

- [ ] **Step 7: Commit, push and open the PR.**
  - Commit ("Desktop: recognize a known diffuser by the device ID in its Bluetooth name").
  - Push `desktop-recognize-by-name` and open a PR against `main`. The body says what changes,
    the Review Focus cases it covers, the test counts from Step 5, and the owner-session step
    that confirms it.

---

## After part 1: the gate before part 2

1. **The owner runs the Mac session's steps 5–11** (`docs/phase-1-owner-session.md`) with a
   build that includes Task 3: scanning and recognizing, adding or setting up again, and
   control over Bluetooth and WiFi. The results go into `docs/findings.md`. Fixes land in
   `apps/desktop` before anything moves (design §7 step 3).
2. **Then write part 2**, `docs/superpowers/plans/<date>-ios-app-part-2.md`. It covers the
   design's §7 steps 4–7, using the spike's two decisions from Task 2 Step 11.
