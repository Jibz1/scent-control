# Phase 1: Desktop App v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native macOS app in Rust that finds, sets up and controls the owner's Scentiment diffusers, and edits their schedules. It sends over Bluetooth when a diffuser is in range and over WiFi through the Phase 0 test broker otherwise, and shows honest, aged status.

**Architecture:** The work is split between two crates.
- **`packages/scent-core`** is pure Rust, with no I/O: typed commands (the type *is* the allow-list), tolerant message parsing, schedules and expected state, and online/staleness rules. It's tested against `fixtures/scentiment/`.
- **`apps/desktop`** is an `eframe`/`egui` app. A background `tokio` runtime runs a Bluetooth worker (`btleplug`) and an MQTT worker (`rumqttc` over `wss://`), and the UI exchanges messages with them over channels.

Bluetooth only works from the bundled `.app` (`scripts/bundle-macos.sh`).

**Tech Stack:** Rust ≥ 1.95 (edition 2024), eframe/egui 0.36.2, btleplug 0.13.4, rumqttc 0.25.1 (`websocket`, `use-rustls`), keyring 4.2.0, tokio 1.53.2, futures 0.3.34, uuid 1.27.0, serde 1.0.229, serde_json 1.0.151, dirs 7.0.0.

**Spec:** `docs/superpowers/specs/2026-10-20-scent-control-design.md`: the "Addendum: a Rust desktop app first" (approved), plus "Appendix: Phase 0 results". Device facts: `docs/findings.md` → "Phase 0 results". The reference behavior is the Phase 0 Python tools and test bench (`tools/scentiment_ble.py`, `tools/safety.py`, `tools/control_server.py`, `tools/control.html`).

**Monorepo (owner, 2026-10-20):** the repo is a Bun + Turborepo monorepo (Task 3a): `packages/scent-core`, `apps/desktop` and `tools/` are workspaces whose `package.json` scripts wrap `cargo`/`pytest`, and the Cargo workspace stays at the root. From the root, `bunx turbo run build test lint fmt:check` runs everything. Each later task adds its new Rust dependencies to its crate's `Cargo.toml` as before.

**Branch:** `phase-1-desktop-app`, cut from `phase-0-device-verification` (rebase onto `main` once PR #1 is squash-merged).

## Global Constraints

- macOS only. `apps/desktop` must not compile code for other platforms. `scent-core` stays platform-independent and does no I/O.
- Rust ≥ 1.95, edition 2024. Pin the versions in Tech Stack exactly in `Cargo.toml` (`=x.y.z`).
- Never contact Scentiment's servers. Never use credentials from Scentiment's app.
- **The allow-list is the `Command` enum.** It has no variant for `UPDATE_FIRMWARE`, `U_FW`, hard or factory reset, light commands, or free-form JSON. Nothing can send a raw frame.
- BLE: service `00000180-0000-1000-8000-00805f9b34fb`, write `0000dead-…`, notify `0000fef3-…`, info `0000fef4-…`. Frame = compact JSON + `-|`, written **with response**, in chunks of **177 bytes** (Scentiment's app requests MTU 180; btleplug doesn't expose the MTU).
- MQTT: default broker `wss://test-broker-production.up.railway.app/mqtt`, username `scent-test`, route ID `route-test-1`. The app subscribes to `scentiment-mqtt-prod/#` and resubscribes on every ConnAck. Commands go to `scentiment-mqtt-prod/device/<deviceId>` at QoS 1. Device IDs match `^[0-9A-Z]{12}$`.
- Secrets:
  - The broker password lives only in the macOS Keychain: service `scent-control`, account `<username>@<broker host>`.
  - WiFi passwords are typed into a masked field, sent once, and never stored or logged.
  - Every log line uses `Command::redacted`.
- Schedules go over **WiFi only** in v1: `UPDATE_PROFILE` (`schedule_flag` 1), then `SYNC_TIME` **5 s** later. Bluetooth schedule setting is unconfirmed (findings).
- Default intensity **1**. The always-on (factory) entry is **off** by default in the editor.
- Status rules:
  - ping every **30 s**;
  - "waiting" for **15 s** after a ping;
  - "online" if heard within **75 s**;
  - a reading is stale after **300 s**.
- Bluetooth: connect timeout **10 s**, listen **6 s** after a send, disconnect after **10 s** idle. One link per diffuser.
- Local state: `~/Library/Application Support/ScentControl/desktop.json` (known diffusers, aliases, last readings). It holds no secrets.
- Bluetooth runs only from the bundle: `scripts/bundle-macos.sh` builds `target/Scent Control.app` (Info.plist with `NSBluetoothAlwaysUsageDescription`), signed ad hoc and launched with `open`.
- Tests use synthetic IDs (`TESTDEVICE01`, `TESTDEVICE02`) only. No real device IDs anywhere in the repo.
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **The Bluetooth link stays open.** A held link blocks Scentiment's app and seemed to delay WiFi commands in Phase 0. The app should drop it 10 s after last use. Pinned by `idle_link_is_dropped_after_10_s` (Task 7).
2. **A wrong or missing broker password** should make the app say "Broker login refused" and stop, not retry silently forever. Pinned by `connack_refusal_is_reported_once` (Task 6) and `login_refused_shows_in_state` (Task 5).
3. **A diffuser out of Bluetooth range in Auto mode** should fall back to WiFi within the 10 s connect timeout, with a note, and never hang. Pinned by `auto_falls_back_to_wifi_after_ble_failure` (Task 8).
4. **The day differs between UTC and the diffuser's time zone.** For example, 03:00 UTC Wednesday is 22:00 Tuesday at UTC−5. Expected state must use the diffuser's local weekday. Pinned by `expected_state_uses_the_local_weekday_across_midnight_utc` (Task 3).
5. **Invalid or unknown firmware messages** should be logged raw and never crash or drop the connection. Pinned by `parse_mqtt_never_panics` and the fixture contract test (Task 2).

---

### Task 1: Workspace, toolchain and typed commands

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`, `packages/scent-core/Cargo.toml`, `packages/scent-core/src/lib.rs`, `packages/scent-core/src/command.rs`, `packages/scent-core/src/level.rs`
- Modify: `.gitignore` (add `/target`)
- Test: `packages/scent-core/src/command.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `scent_core::{Command, Via, Secret, Level, BLE_CHUNK, chunks}`
  - `Level::new(u8) -> Option<Level>`, valid for 1..=3, with `Level::get(&self) -> u8`
  - `Secret::new(String)`. `Debug` and `Display` print `***`, and `Secret::expose(&self) -> &str`.
  - `enum Via { Bluetooth, Wifi }`
  - `enum Command`, with variants:
    - `TurnOn { on: bool }`
    - `SetLevel(Level)`
    - `Ping { userid: Option<String> }`
    - `GetStatus`
    - `SyncTime { server_time: u64 }`
    - `UpdateProfile(Profile)` (Profile from Task 3; add this variant in Task 3)
    - `SaveMqtt { broker: String, username: String, password: Secret }`
    - `WifiConnect { ssid: String, password: Secret }`
    - `SoftRestart`
  - `Command::action(&self, via: Via) -> &'static str`
  - `Command::payload(&self, via: Via) -> Option<serde_json::Value>`
  - `Command::ble_frame(&self) -> Vec<u8>`
  - `Command::mqtt_body(&self) -> Vec<u8>`
  - `Command::redacted(&self, via: Via) -> String`
  - `pub const BLE_CHUNK: usize = 177`, and `pub fn chunks(frame: &[u8]) -> std::slice::Chunks<'_, u8>`

- [ ] **Step 1: Update the toolchain and create the workspace.**
  - Run `rustup update stable`. Then `rustc --version` must show 1.95 or newer.
  - `rust-toolchain.toml`: `channel = "stable"`.
  - Root `Cargo.toml`:
    - `[workspace] resolver = "3"`, `members = ["packages/scent-core", "apps/desktop"]`;
    - `[workspace.package] edition = "2024"`, `rust-version = "1.95"`, `license = "MIT"`;
    - `[workspace.dependencies]` pinning `serde = { version = "=1.0.229", features = ["derive"] }` and `serde_json = "=1.0.151"`.
  - Create `apps/desktop` as an empty binary crate (`fn main() {}`) so the workspace resolves. Task 4 fills it in.

- [ ] **Step 2: Write the failing tests** in `command.rs`:

```rust
#[test] fn ble_frames_are_compact_json_with_terminator() {
    assert_eq!(Command::SetLevel(Level::new(2).unwrap()).ble_frame(), br#"{"action":"SET_LEVEL","payload":{"intensity":2}}-|"#.to_vec());
    assert_eq!(Command::Ping { userid: None }.ble_frame(), br#"{"action":"PING"}-|"#.to_vec());
    assert_eq!(Command::TurnOn { on: false }.mqtt_body(), br#"{"action":"TURN_ON","payload":{"on":0}}"#.to_vec());
}
#[test] fn soft_restart_differs_per_transport() {
    assert_eq!(Command::SoftRestart.ble_frame(), br#"{"action":"SECRET_COMMAND","payload":{"command":"restart"}}-|"#.to_vec());
    assert_eq!(Command::SoftRestart.mqtt_body(), br#"{"action":"RESET","payload":{"mode":"soft"}}"#.to_vec());
}
#[test] fn secrets_never_appear_in_redacted_or_debug_output() {
    let c = Command::WifiConnect { ssid: "ExampleWiFi".into(), password: Secret::new("hunter2!".into()) };
    assert!(!c.redacted(Via::Bluetooth).contains("hunter2!") && c.redacted(Via::Bluetooth).contains(r#""password":"***""#));
    assert!(!format!("{c:?}").contains("hunter2!"));
    assert!(String::from_utf8(c.ble_frame()).unwrap().contains("hunter2!")); // the device does need it
}
#[test] fn level_is_one_to_three() { assert!(Level::new(0).is_none() && Level::new(4).is_none() && Level::new(1).is_some()); }
#[test] fn long_frames_split_into_177_byte_chunks_that_rejoin() {
    let c = Command::SaveMqtt { broker: format!("wss://{}/mqtt", "a".repeat(300)), username: "u".into(), password: Secret::new("p".into()) };
    let frame = c.ble_frame();
    let parts: Vec<&[u8]> = chunks(&frame).collect();
    assert!(parts.iter().all(|p| p.len() <= 177) && parts.concat() == frame && parts.len() == frame.len().div_ceil(177));
}
```

- [ ] **Step 3: Run the tests to verify they fail.**

  Run: `cargo test -p scent-core`
  Expected: compile errors, because `Command` and friends aren't defined.

- [ ] **Step 4: Implement `level.rs` and `command.rs`** to the interfaces above.
  - Serialize with `serde_json::to_vec`, which is compact.
  - Booleans go out as `0` or `1`, matching the firmware (`{"on":0}`).
  - `redacted` builds the same JSON as `payload`, with `"***"` for every `password`.

- [ ] **Step 5: Run the tests to verify they pass.**

  Run: `cargo test -p scent-core && cargo clippy --workspace -- -D warnings`
  Expected: 5 passed, no warnings.

- [ ] **Step 6: Commit.**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore crates apps/desktop
git commit -m "Add Rust workspace and scent-core typed commands

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Message parsing and the fixture contract

**Files:**
- Create: `packages/scent-core/src/message.rs`, `packages/scent-core/src/names.rs`, `packages/scent-core/src/topics.rs`, `packages/scent-core/tests/fixtures.rs`
- Modify: `packages/scent-core/src/lib.rs`

**Interfaces:**
- Produces:
  - **Status:** `struct Status { power, intensity, charging, battery, fan, gas_pump, light: Option<i64> }`, deriving `Default` and `PartialEq`.
  - **Bluetooth notifications:** `enum NoteKind { StatusText, StatusArray, Value, Json, Unknown }` and `struct Notification { raw: String, kind: NoteKind, status: Status }`. `parse_notification(bytes: &[u8]) -> Notification` never panics. Status array indices: 4 power, 5 charging, 6 battery, 7 *ignored (not oil)*, 8 fan, 9 intensity, 10 *ignored*, 11 gas_pump, 12 light, 13 *ignored*. Only the indices present are read. Text keys: `S`→power, `L`→intensity, `C`→charging, `Bat`→battery, `enable`→power, `level`→intensity, `battery`→battery.
  - **Device info:** `struct DeviceInfo { manufacturer_id, model_name, variant_name, device_id, firmware: String, wifi_configured: bool, wifi_status: u8 }`. `parse_info(&str) -> Result<DeviceInfo, String>` fails with an error naming the field count when there are fewer than 7 `|` fields.
  - **MQTT messages:** `enum Direction { ToDevice, FromDevice }`, `enum Ack { Added, AlreadyAdded, Other(String) }`, `struct Connected { fw, model, variant, manufacturer: String }`, and `struct MqttMessage { direction, action: Option<String>, device_id: Option<String>, status: Option<Status>, rssi: Option<i64>, ack: Option<Ack>, connected: Option<Connected>, valid_json: bool, raw: String }`. `parse_mqtt(topic: &str, body: &[u8]) -> MqttMessage` repairs `"data":enable:0` (unquoted) and reads `data` given as a string *or* an object.
  - **Names:** `is_scentiment_name(&str) -> bool`, `parse_name(&str) -> NameInfo { model_code, model, variant_code, variant: Option<String> }`, and `device_id_from_name(&str) -> Option<String>`. Codes and names are as in `tools/scentiment_ble.py`.
  - **Topics:** `DEVICE_PREFIX = "scentiment-mqtt-prod/device/"`, `SUBSCRIBE_ALL = "scentiment-mqtt-prod/#"`, `device_topic(&str) -> String`, and `valid_device_id(&str) -> bool`.

- [ ] **Step 1: Write the failing tests.** Put unit tests in `message.rs`, ported from `tools/tests/test_scentiment_ble.py`'s `parse_notification`/`parse_info` cases and `tools/tests/test_control_server.py`'s `parse_device_message` cases, with the same bytes and the same expected fields. Add:

```rust
#[test] fn parse_mqtt_never_panics() {
    for body in [&b"\xff\xfe"[..], b"", b"{", b"null", br#"{"action":5}"#, br#"{"payload":{"data":{"data":"x"}}}"#] {
        let m = parse_mqtt("scentiment-mqtt-prod/user/route-test-1", body);
        assert!(!m.raw.is_empty() || body.is_empty());
    }
}
#[test] fn status_index_7_is_not_read_as_oil() {
    let n = parse_notification(br#"{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]}"#);
    assert_eq!(n.status, Status { power: Some(1), charging: Some(2), battery: Some(200), fan: Some(255), intensity: Some(2), gas_pump: Some(255), light: Some(255) });
}
```

  `tests/fixtures.rs`: load every `fixtures/scentiment/*.json` file (path from `env!("CARGO_MANIFEST_DIR")/../../fixtures/scentiment`). Assert:
  - `ble-info` fixtures pass `parse_info`, with ID `TESTDEVICE01` or `TESTDEVICE02`;
  - `ble-notify` fixtures don't parse as `Unknown`;
  - `mqtt` fixtures parse with an action and a synthetic device ID;
  - `air2-mqtt-notification-invalid-json` has `valid_json == false` and `status.power == Some(0)`;
  - `air2-mqtt-ack-added` has `ack == Some(Ack::Added)`.

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-core`
  Expected: compile errors, because the parsing functions aren't defined.

- [ ] **Step 3: Implement `message.rs`, `names.rs` and `topics.rs`.** The repair is a regex-free scan: for `"data":` followed by a character that isn't `"`, `{` or `[`, wrap the value up to the next `,` or `}` in quotes, then parse again.

- [ ] **Step 4: Run all core tests.**

  Run: `cargo test -p scent-core`
  Expected: all pass, including `fixtures` over the 16 fixture files.

- [ ] **Step 5: Commit.**

```bash
git add packages/scent-core
git commit -m "scent-core: tolerant message parsing, names, topics, fixture contract

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Schedules, expected state, online and staleness

**Files:**
- Create: `packages/scent-core/src/schedule.rs`, `packages/scent-core/src/status.rs`
- Modify: `packages/scent-core/src/command.rs` (add `UpdateProfile(Profile)`), `packages/scent-core/src/lib.rs`

**Interfaces:**
- Consumes: `Level`
- Produces:
  - **Clock time:** `struct HhMm { hour: u8, minute: u8 }`, with `HhMm::parse(&str) -> Option<HhMm>` and `Display` as `"HH:MM"`.
  - **Entries:** `struct Entry { start: HhMm, end: HhMm, weekdays: Vec<u8> /* 0=Sun..6=Sat */, level: Level, enable: bool }` and `struct AlwaysOn { enable: bool, level: Level }`.
  - **Profiles:** `enum ScheduleFlag { AlwaysOnOnly = 0, Replace = 1 }` and `struct Profile { always_on: AlwaysOn, entries: Vec<Entry>, timezone_hours: f64, flag: ScheduleFlag }`. `Profile::to_payload(&self) -> serde_json::Value` produces exactly the `UPDATE_PROFILE` payload in `docs/findings.md`. The factory entry comes first (`index` 1, `00:00`–`23:59`, all weekdays, `workingState {level, fan 0, led 0, timer 0}`, `enable`, `isFactory: true`), followed by the entries, which have no `index` and no `isFactory`. `AlwaysOnOnly` sends just the factory entry.
  - **Expected state:** `enum Source { Entry { start: HhMm, end: HhMm }, AlwaysOn }` and `struct Expected { power: bool, level: Option<Level>, source: Source, until: Option<HhMm> }`. `expected_state(&Profile, now_utc_secs: i64) -> Expected`. Supporting functions: `local_weekday_and_time(now_utc_secs: i64, tz_hours: f64) -> (u8, HhMm)` and `trial_entry(weekday: u8, now: HhMm, start_in_min: u16, duration_min: u16, level: Level) -> Result<Entry, String>`, which fails with an error mentioning "midnight".
  - **Status rules:**
    - `enum Online { Online, Waiting, Offline, Unknown }`
    - `online_status(ping_at: Option<i64>, heard_at: Option<i64>, now: i64) -> Online`
    - `PING_EVERY_S = 30`, `ANSWER_WITHIN_S = 15`, `ONLINE_FOR_S = 75`, `STALE_AFTER_S = 300`
    - `is_stale(at: i64, now: i64) -> bool`

- [ ] **Step 1: Write the failing tests.** Port these from `tools/tests/test_control_server.py`: `expected_state` during and after a window, flag-0 behavior, and `online_status` (all four cases). Port `trial_schedule_entry` from `tools/tests/test_scentiment_ble.py`. Add:

```rust
#[test] fn expected_state_uses_the_local_weekday_across_midnight_utc() {
    // Wed 2026-10-21 03:00 UTC is Tue 22:00 at UTC-5
    let p = Profile { always_on: AlwaysOn { enable: false, level: Level::new(1).unwrap() }, timezone_hours: -5.0, flag: ScheduleFlag::Replace,
        entries: vec![Entry { start: HhMm::parse("21:30").unwrap(), end: HhMm::parse("22:30").unwrap(), weekdays: vec![2], level: Level::new(1).unwrap(), enable: true }] };
    let e = expected_state(&p, 1_792_551_600); // 2026-10-21T03:00:00Z
    assert!(e.power && e.until == HhMm::parse("22:30"));
}
#[test] fn payload_shape_matches_the_firmware() {
    let p = Profile { always_on: AlwaysOn { enable: false, level: Level::new(1).unwrap() }, timezone_hours: -5.0, flag: ScheduleFlag::Replace,
        entries: vec![Entry { start: HhMm::parse("07:00").unwrap(), end: HhMm::parse("09:00").unwrap(), weekdays: vec![1,2,3,4,5], level: Level::new(1).unwrap(), enable: true }] };
    assert_eq!(p.to_payload(), serde_json::json!({"profile":{"schedule":[
        {"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},
        {"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":true}]},
        "timezone":-5.0,"schedule_flag":1}));
}
```

  Verify the epoch constant in the first test with `date -u -r 1792551600` before trusting it.

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-core`
  Expected: compile errors, because the schedule and status types aren't defined.

- [ ] **Step 3: Implement `schedule.rs` and `status.rs`.** Use plain arithmetic, with no date crate:

```rust
let local = now_utc_secs + (tz_hours * 3600.0).round() as i64;
let days = local.div_euclid(86_400);
let weekday = ((days + 4).rem_euclid(7)) as u8; // 1970-01-01 was a Thursday; 0 = Sunday
let secs = local.rem_euclid(86_400);
```

  An entry is active when `enable`, the weekday is in `weekdays`, and `start <= now < end`. The first active entry wins; otherwise the always-on entry decides.

- [ ] **Step 4: Run all core tests.**

  Run: `cargo test -p scent-core && cargo clippy --workspace -- -D warnings`
  Expected: all pass, no warnings.

- [ ] **Step 5: Commit.**

```bash
git add packages/scent-core
git commit -m "scent-core: schedules, expected state, online and staleness rules

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: App shell and macOS bundle

**Files:**
- Create: `apps/desktop/Cargo.toml`, `apps/desktop/src/main.rs`, `apps/desktop/src/app.rs`, `apps/desktop/macos/Info.plist`, `scripts/bundle-macos.sh`
- Modify: `README.md` (a "Desktop app" section)

**Interfaces:**
- Produces:
  - `struct ScentApp`, implementing `eframe::App`, with `ScentApp::new(cc: &eframe::CreationContext) -> Self`.
  - The bundle path `target/Scent Control.app`, with bundle identifier `dev.scent-control.desktop`.
  - The binary `scent-control`.

- [ ] **Step 1: Write `apps/desktop/Cargo.toml` and the app shell.**
  - `[package] name = "scent-desktop"`, `[[bin]] name = "scent-control"`.
  - Dependencies: `eframe = { version = "=0.36.2", default-features = false, features = ["accesskit", "default_fonts", "glow"] }` and `scent-core = { path = "../../packages/scent-core" }`.
  - Gate the crate with `#![cfg(target_os = "macos")]` in `main.rs`, and give other targets a `compile_error!`.
  - The window is titled "Scent Control", 1100×720 by default. It has an empty left panel ("Diffusers") and an empty central panel.
  - **`muda` is deferred:** winit already gives a macOS app its standard app menu (About, Hide, Quit). The addendum's "real menu bar" comes when the app has menu commands of its own. Record this as a ruling.

- [ ] **Step 2: Write `apps/desktop/macos/Info.plist`** with these keys:
  - `CFBundleName` "Scent Control"
  - `CFBundleIdentifier` `dev.scent-control.desktop`
  - `CFBundleExecutable` `scent-control`
  - `CFBundlePackageType` `APPL`
  - `LSMinimumSystemVersion` `13.0`
  - `NSHighResolutionCapable` true
  - `NSBluetoothAlwaysUsageDescription`: "Scent Control finds your diffusers and talks to them over Bluetooth."

- [ ] **Step 3: Write `scripts/bundle-macos.sh`.** It:
  1. runs `cargo build --release -p scent-desktop`;
  2. assembles `target/Scent Control.app/Contents/{MacOS/scent-control, Info.plist}`;
  3. runs `codesign --force --sign - "target/Scent Control.app"`;
  4. prints the path.

  Use `set -euo pipefail`. Its usage comment says to launch with `open "target/Scent Control.app"`, never by running the binary directly: macOS kills Bluetooth access from processes without the plist declaration.

- [ ] **Step 4: Verify the bundle.**

  Run: `scripts/bundle-macos.sh && codesign --verify "target/Scent Control.app" && open "target/Scent Control.app" && sleep 3 && pgrep -x scent-control`
  Expected: a valid signature, and a PID printed. The owner confirms the window titled "Scent Control" is visible. Then quit it.

- [ ] **Step 5: Commit.**

```bash
git add apps/desktop scripts/bundle-macos.sh README.md
git commit -m "Desktop app shell and macOS bundle with Bluetooth usage description

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: App state, events and local store

**Files:**
- Create: `apps/desktop/src/state.rs`, `apps/desktop/src/store.rs`
- Modify: `apps/desktop/Cargo.toml` (`serde`, `serde_json`, `dirs = "=7.0.0"`), `apps/desktop/src/main.rs`

**Interfaces:**
- Consumes: `scent_core::{parse_mqtt, parse_notification, DeviceInfo, Status, Via, online_status, is_stale, expected_state, Profile}`
- Produces:
  - **Readings and pending commands:**
    - `struct Reading { value: i64, at: i64, via: Via }`
    - `enum Field { Power, Intensity, Charging, Battery, Rssi }`
    - `struct Sent { power: Option<bool>, intensity: Option<u8>, at: i64 }`
  - **Devices:** `struct DeviceView { id: String, label: String, ble_id: Option<String>, info: Option<DeviceInfo>, readings: BTreeMap<Field, Reading>, heard_wifi: Option<i64>, heard_ble: Option<i64>, ping_at: Option<i64>, sent: Option<Sent>, sent_profile: Option<Profile> }`
  - **Events:** `enum Event`, with variants:
    - `Mqtt { topic: String, body: Vec<u8>, at: i64 }`
    - `BleNotification { ble_id: String, bytes: Vec<u8>, at: i64 }`
    - `BleInfo { ble_id: String, info: DeviceInfo, at: i64 }`
    - `Scanned(Vec<ScanHit>)`
    - `Sent { device_id: String, summary: String, via: Via, at: i64, power: Option<bool>, intensity: Option<u8>, profile: Option<Profile> }`
    - `Pinged { device_id: String, at: i64 }`
    - `Failed { device_id: Option<String>, what: String, error: String, at: i64 }`
    - `BrokerConnected`
    - `BrokerLoginRefused(String)`
    - `BrokerDisconnected(String)`

    where `struct ScanHit { ble_id: String, name: String, rssi: Option<i16> }`.
  - **Broker state:** `enum BrokerState { Connecting, Connected, LoginRefused(String), Disconnected(String) }`
  - **App state:** `struct AppState { devices: BTreeMap<String, DeviceView>, scan: Vec<ScanHit>, broker: BrokerState, log: VecDeque<LogEntry> /* cap 500 */ }`, with:
    - `AppState::apply(&mut self, e: Event)`
    - `AppState::online(&self, id: &str, now: i64) -> Online`
    - `struct LogEntry { at: i64, device_id: Option<String>, via: Option<Via>, direction: Direction, action: String, text: String }`
  - **Store:** `Store { dir: PathBuf }`, with:
    - `Store::default_dir() -> PathBuf` (Application Support/ScentControl)
    - `Store::load(&self) -> AppState`, which returns an empty state on a missing or corrupt file and logs the problem
    - `Store::save(&self, s: &AppState) -> std::io::Result<()>`

    The store writes `desktop.json` atomically (temp file plus rename) and persists only `devices` (id, label, ble_id, info, readings).

- [ ] **Step 1: Write the failing tests.** Put the following in `state.rs`; storage tests use a temp dir.
  - `apply_mqtt_keeps_each_reading_with_time_and_source`: the update_status fixture text, then a Bluetooth `enable:0`, reproduces the Python `test_apply_event_keeps_each_reading_with_its_time_and_source`.
  - `sent_is_pending_until_a_later_report`: after `Sent { power: Some(true), .. at: 100 }` and a report `at: 90`, the pending state is still shown; after a report `at: 110`, it isn't.
  - `login_refused_shows_in_state`: `BrokerLoginRefused("Not authorized")` leads to `BrokerState::LoginRefused`, plus one log entry.
  - `sent_profile_flag_0_replaces_only_the_always_on_entry`: the port of the Python test.
  - `store_round_trips_devices_and_survives_a_corrupt_file`: save, load and compare; then write `{` to the file and check `load` returns an empty state.
  - `log_is_capped_at_500`.

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-desktop`
  Expected: compile errors, because `AppState` and `Store` aren't defined.

- [ ] **Step 3: Implement `state.rs` and `store.rs`.** `Mqtt` events with `Direction::ToDevice` and action `UPDATE_PROFILE` aren't parsed back into `sent_profile`: the hub reports profiles through `Event::Sent { profile }`.

- [ ] **Step 4: Run the tests.**

  Run: `cargo test -p scent-desktop && cargo clippy --workspace -- -D warnings`
  Expected: all pass.

- [ ] **Step 5: Commit.**

```bash
git add apps/desktop
git commit -m "Desktop: app state, events and local store

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: MQTT worker

**Files:**
- Create: `apps/desktop/src/mqtt.rs`, `apps/desktop/src/secrets.rs`, `apps/desktop/examples/mqtt_roundtrip.rs`
- Modify: `apps/desktop/Cargo.toml` (`rumqttc = { version = "=0.25.1", default-features = false, features = ["websocket", "use-rustls"] }`, `tokio = { version = "=1.53.2", features = ["rt-multi-thread", "macros", "sync", "time"] }`, `keyring = "=4.2.0"`)

**Interfaces:**
- Consumes: `Event`, `scent_core::{Command, Via, SUBSCRIBE_ALL, device_topic, valid_device_id}`
- Produces:
  - `struct BrokerSettings { url: String /* wss://host/mqtt */, username: String, route_id: String }`, implementing `Default` with the values in Global Constraints.
  - `secrets::broker_password(&BrokerSettings) -> Option<Secret>` and `secrets::set_broker_password(&BrokerSettings, &Secret) -> Result<(), String>`, using Keychain service `scent-control`, account `<username>@<host>`.
  - `struct MqttHandle`, with `MqttHandle::send(&self, device_id: &str, cmd: &Command) -> Result<(), String>`, which refuses invalid device IDs and returns once the message is queued.
  - `mqtt::spawn(settings: BrokerSettings, password: Secret, events: std::sync::mpsc::Sender<Event>, repaint: impl Fn() + Send + 'static) -> MqttHandle`
  - `mqtt::map_connack(code) -> Option<Event>`, which returns `BrokerLoginRefused` for not-authorized or bad-credentials codes.
  - The worker:
    - subscribes to `SUBSCRIBE_ALL` on every ConnAck;
    - turns each publish into `Event::Mqtt`;
    - on a refused login, emits `BrokerLoginRefused` **once** and stops polling (no reconnect loop);
    - on any other error, emits `BrokerDisconnected` and retries every 5 s.

- [ ] **Step 1: Write the failing tests** in `mqtt.rs`:
  - `connack_refusal_is_reported_once`: `map_connack` returns `BrokerLoginRefused` for `NotAuthorized` and `BadUserNamePassword`, and `None` for `Success`. A small pure helper `RetryPolicy::after(&Event) -> Option<Duration>` returns `None` after a refusal and `Some(5s)` after a disconnect.
  - `send_refuses_bad_device_ids`: `"#"`, `"TESTDEVICE01/x"` and `""` all return `Err`.
  - `broker_url_parsing`: `wss://test-broker-production.up.railway.app/mqtt` gives host, port 443 and path `/mqtt`. Anything that isn't `wss://` is an error (encrypted only, as in Phase 0).

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-desktop mqtt`
  Expected: compile errors, because the MQTT functions aren't defined.

- [ ] **Step 3: Implement `mqtt.rs` and `secrets.rs`.** Use `MqttOptions::new("scent-desktop-<8 random hex>", <full wss URL>, 443)`, `set_transport(Transport::wss_with_default_config())`, `set_credentials` and keep-alive 30 s. Publish with `AsyncClient::publish(device_topic(id), QoS::AtLeastOnce, false, cmd.mqtt_body())`.

- [ ] **Step 4: Run the tests.**

  Run: `cargo test -p scent-desktop`
  Expected: all pass.

- [ ] **Step 5: Live check against the test broker.** MQTT needs no Bluetooth, so this runs from a plain shell. `examples/mqtt_roundtrip.rs` reads the password from the Keychain, or from `SCENT_BROKER_PASSWORD` in development, connects, sends `Command::Ping { userid: Some("route-test-1") }` to the Air 2's device topic (the ID is taken from `desktop.json` or an argument, never hardcoded), and prints events for 10 s.

  Run: `cargo run -p scent-desktop --example mqtt_roundtrip -- <deviceId>`
  Expected: `BrokerConnected`, then an `Mqtt` event whose body is an `acknowledgement` with `ALREADY_ADDED`.

- [ ] **Step 6: Commit.**

```bash
git add apps/desktop
git commit -m "Desktop: MQTT worker over wss with resubscribe, Keychain broker password

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Bluetooth worker

**Files:**
- Create: `apps/desktop/src/ble.rs`
- Modify: `apps/desktop/Cargo.toml` (`btleplug = "=0.13.4"`, `uuid = "=1.27.0"`, `futures = "=0.3.34"`)

**Interfaces:**
- Consumes: `Event`, `ScanHit`, `scent_core::{Command, chunks, parse_info, is_scentiment_name}`
- Produces:
  - `struct BleHandle`, with:
    - `scan(&self, secs: u64)`, which emits `Event::Scanned` filtered with `is_scentiment_name`;
    - `read_info(&self, ble_id: &str)`, which emits `BleInfo` or `Failed`;
    - `send(&self, ble_id: &str, cmd: Command)`, which connects, writes each chunk with response, then forwards notifications for 6 s, or emits `Failed`;
    - `read_status(&self, ble_id: &str)`, which reads `0xFEF3` and emits `BleNotification`.
  - `ble::spawn(events: std::sync::mpsc::Sender<Event>, repaint: impl Fn() + Send + 'static) -> BleHandle`
  - `struct LinkTimer`, with `LinkTimer::touch(&mut self, now: Instant)` and `LinkTimer::should_disconnect(&self, now: Instant) -> bool` (10 s idle).
  - `CONNECT_TIMEOUT = 10 s`, `LISTEN_AFTER_SEND = 6 s`.

- [ ] **Step 1: Write the failing tests:**
  - `idle_link_is_dropped_after_10_s`: `LinkTimer` touched at t0; `should_disconnect` is false at t0+9 s and true at t0+10 s; touching again at t0+9 s defers it.
  - `frames_are_written_in_order_in_177_byte_chunks`: a pure `fn writes_for(cmd: &Command) -> Vec<Vec<u8>>`, used by `send`, rejoins to `cmd.ble_frame()` with each part ≤ 177 bytes.

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-desktop ble`
  Expected: compile errors.

- [ ] **Step 3: Implement `ble.rs`** on btleplug's `Manager` → first adapter.
  - Find peripherals by their `PeripheralId` string, which is the stored `ble_id`.
  - On connect: `discover_services` → find the characteristics by UUID → `subscribe(notify)` → spawn a task that forwards `notifications()` as `BleNotification`.
  - One task per connected peripheral holds the `LinkTimer` and disconnects when it fires.

- [ ] **Step 4: Run the tests.**

  Run: `cargo test -p scent-desktop`
  Expected: all pass.

- [ ] **Step 5: Live check from the bundle (owner nearby, phone Bluetooth off).** Temporarily wire a "Scan" button in `app.rs` that calls `BleHandle::scan(10)` and lists the hits. Run `scripts/bundle-macos.sh && open "target/Scent Control.app"`. The owner approves the "Scent Control would like to use Bluetooth" prompt.

  Expected: both `Scentiment-…` diffusers are listed with their parsed model.

- [ ] **Step 6: Commit.**

```bash
git add apps/desktop
git commit -m "Desktop: Bluetooth worker with chunked writes and idle disconnect

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Hub and Bluetooth-first routing

**Files:**
- Create: `apps/desktop/src/route.rs`, `apps/desktop/src/hub.rs`

**Interfaces:**
- Consumes: `BleHandle`, `MqttHandle`, `AppState`, `Event`
- Produces:
  - `enum TransportPref { Auto, Wifi, Bluetooth }` (default `Auto`) and `route(pref: TransportPref, ble_known: bool, on_broker: bool) -> Vec<Via>`. Auto gives `[Bluetooth?, Wifi?]` in that order, keeping only the available ones.
  - `enum UiCommand`, with variants:
    - `Scan`
    - `Send { device_id: String, cmd: Command, pref: TransportPref }`
    - `ReadStatus { device_id: String }`
    - `SendSchedule { device_id: String, profile: Profile }`
    - `Add(AddRequest)` (Task 10)
  - `struct Hub`, with `Hub::start(settings, password: Option<Secret>, repaint) -> (Hub, std::sync::mpsc::Receiver<Event>)` and `Hub::submit(&self, c: UiCommand)`.
  - The hub runs the ping loop: every `PING_EVERY_S` it sends `Ping { userid: route_id }` to each device heard over WiFi, and emits `Pinged`.
  - `SendSchedule` sends `UpdateProfile`, then `SyncTime { server_time: now }` 5 s later, both over WiFi.

- [ ] **Step 1: Write the failing tests** in `route.rs`:

```rust
#[test] fn auto_is_bluetooth_first() { assert_eq!(route(TransportPref::Auto, true, true), vec![Via::Bluetooth, Via::Wifi]); }
#[test] fn auto_without_bluetooth_id_uses_wifi() { assert_eq!(route(TransportPref::Auto, false, true), vec![Via::Wifi]); }
#[test] fn forced_transports_have_no_fallback() { assert_eq!(route(TransportPref::Wifi, true, true), vec![Via::Wifi]); assert_eq!(route(TransportPref::Bluetooth, true, false), vec![Via::Bluetooth]); }
#[test] fn nothing_available_is_an_empty_route() { assert!(route(TransportPref::Auto, false, false).is_empty()); }
```

  Also in `hub.rs`: `auto_falls_back_to_wifi_after_ble_failure`, which drives a pure `next_attempt(route: &[Via], failed: &[Via]) -> Option<Via>`, plus `schedule_sends_profile_then_sync_time_5_s_later`, which drives a pure `schedule_steps(profile, now) -> [(Duration, Command); 2]`.

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-desktop route hub`
  Expected: compile errors.

- [ ] **Step 3: Implement `route.rs` and `hub.rs`.** A Bluetooth failure (connect timeout or write error) followed by a WiFi attempt is logged as one `Failed` ("Bluetooth couldn't reach the diffuser, so this went over WiFi").

- [ ] **Step 4: Run the tests.**

  Run: `cargo test -p scent-desktop && cargo clippy --workspace -- -D warnings`
  Expected: all pass.

- [ ] **Step 5: Commit.**

```bash
git add apps/desktop
git commit -m "Desktop: hub with Bluetooth-first routing, ping loop and schedule sequencing

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Main screens (diffusers, status, controls, settings, log)

**Files:**
- Create: `apps/desktop/src/ui/mod.rs`, `ui/device_list.rs`, `ui/device_panel.rs`, `ui/settings.rs`, `ui/log_view.rs`, `ui/format.rs`
- Modify: `apps/desktop/src/app.rs`

**Interfaces:**
- Consumes: `Hub`, `AppState`, `Store`, `secrets`, `scent_core::{expected_state, online_status, is_stale}`
- Produces:
  - **Formatting helpers** in `ui/format.rs`:
    - `ago(at: i64, now: i64) -> String`: "just now", "N min ago" or "at HH:MM";
    - `reading_text(r: Option<&Reading>, now: i64, format: impl Fn(i64) -> String) -> (String, bool /* stale */)`;
    - `power_source(charging: Option<i64>, battery: Option<i64>) -> &'static str`: "plugged in" when charging is 2 or battery is 200; "charging"; "battery N%"; or "not reported yet";
    - `expected_text(&Expected) -> String`.
  - **Screens:**
    - **Diffuser list (left).** Name, model and online badge for each, plus "Scan for diffusers" and "Add…".
    - **Device panel (center).**
      - Readings: power, intensity, power source and WiFi signal, each with its age and source, greyed when stale. A pending "sent, waiting for the diffuser" value shows in amber.
      - The expected-state line.
      - A "Send over" segmented control (Auto / WiFi / Bluetooth).
      - Power on/off and intensity Low/Medium/High buttons.
      - "Read over Bluetooth".
    - **Settings sheet.** Broker URL, username and route ID, plus a masked password field that saves to the Keychain. A "Broker login refused" banner shows when `BrokerState::LoginRefused`.
    - **Log (bottom, collapsible).** The conversation, newest last, with direction, via, action and redacted text.
  - `ScentApp::update` drains events with `try_recv` into `AppState::apply`, saves the store at most every 10 s, and calls `ctx.request_repaint_after(1 s)` so ages stay current.

- [ ] **Step 1: Write the failing tests** in `ui/format.rs`. They port the bench's wording exactly:
  - `ago_formats`: 0 s → "just now", 180 s → "3 min ago", 7200 s → "at HH:MM".
  - `power_source_wording`: (2, 200) → "plugged in"; (1, 80) → "charging"; (0, 80) → "battery 80%"; (None, None) → "not reported yet".
  - `reading_is_stale_after_300_s`.
  - `expected_text_wording`: "Its schedule says it should be off now." and "Its schedule says it should be on at level 1 until 22:30 (schedule 21:30–22:30)."

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-desktop format`
  Expected: compile errors.

- [ ] **Step 3: Implement the formatting helpers and the screens.** Copy: sentence case, plain verbs, no all-caps labels, and the same words the bench uses.

- [ ] **Step 4: Run the tests and lints.**

  Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings`
  Expected: all pass.

- [ ] **Step 5: Live check from the bundle (owner).** Enter the broker password in Settings once. Then check:
  1. both diffusers show "Answering" within about 30 s;
  2. turn the Air 2 on and off over Auto (Bluetooth), then over WiFi, and confirm what the diffuser does;
  3. readings show their ages and sources.

  Note any differences in `docs/findings.md` as **[observed]**.

- [ ] **Step 6: Commit.**

```bash
git add apps/desktop
git commit -m "Desktop: diffuser list, status and controls, settings, log

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9a: Apple design language (owner, 2026-10-20)

The owner asked for Apple design language in the desktop app. The binding rules are the
spec section "Design language: Apple's macOS guidelines (owner, 2026-10-20)" in
`docs/superpowers/specs/2026-10-20-scent-control-design.md`. It covers color, type,
layout, controls, settings, notices, the log, capitalization and motion. Read it first;
its values are exact. This task restyles the Task 9 screens to match. It changes no
behavior: every command, state rule and wording rule from Task 9 stays, apart from
capitalization, which follows the spec section.

**Files:**
- Create: `apps/desktop/src/ui/theme.rs`. It holds the palette for light and dark, the
  text styles, the spacing, and font loading.
- Create: `apps/desktop/src/ui/widgets.rs`. It holds the switch, segmented control,
  grouped box and row, status dot, notice, and push buttons in default and plain styles.
- Modify:
  - `apps/desktop/src/main.rs`: viewport chrome.
  - `apps/desktop/src/app.rs`.
  - `apps/desktop/src/ui/*.rs`: the screens.
  - `apps/desktop/Cargo.toml`: the snapshot dev-dependency only.
- Test: `ui/theme.rs` and `ui/widgets.rs` unit tests, plus the snapshot renderer below.

**Interfaces:**
- Produces:
  - `theme::Palette` with `Palette::for_dark(dark: bool) -> Palette`. Its fields are
    `accent`, `green`, `orange`, `red`, `gray`, `label`, `secondary_label`, `separator`,
    `window`, `sidebar` and `grouped`, as `egui::Color32`.
  - `theme::install(ctx: &egui::Context, fonts_dir: &Path)`. It sets `ThemePreference::System`,
    the fonts, the text styles and the visuals for both appearances.
  - `theme::font_definitions(fonts_dir: &Path) -> egui::FontDefinitions`. It falls back to
    egui's defaults when `SFNS.ttf` or `SFNSMono.ttf` is missing.
  - `theme::status_color(&Palette, Online) -> Color32`: Answering is green, waiting is
    orange, everything else is gray.
  - Widgets that Task 10's sheets will reuse: `switch(ui, &mut bool, label)`,
    `segmented(ui, &mut T, &[(T, &str)], label)` (the label names its accessibility group), `grouped(ui, title, |ui| …)`,
    `row(ui, label, |ui| …)`, `notice(ui, kind, text)`, `default_button(ui, text)` and
    `plain_button(ui, text)`.

- [ ] **Step 1: Write the failing tests.**
  - `palette_matches_the_spec`: every value in the spec's color table, for both
    appearances. Alpha values: label 85%, secondary 50% light and 55% dark, separator 10%.
  - `status_colors_follow_online`: Answering is green; waiting is orange; unknown and
    silent are gray.
  - `fonts_fall_back_when_missing`: `font_definitions` on an empty temporary directory
    returns egui's default families, with no SF entries, and doesn't panic.
  - `semibold_uses_weight_590`: the semibold SF font data carries `wght` 590 and the
    regular carries 400. Read this from the `FontTweak` coords your code builds, through a
    pure helper.
  - `segmented_selects_the_clicked_segment`: drive the segmented control with
    `egui_kittest`, if you add it, or test a pure `segment_at` helper.

- [ ] **Step 2: Run them to verify they fail.** Run `cargo test -p scent-desktop theme widgets`.
  Expected: compile errors.

- [ ] **Step 3: Implement the theme, the widgets and the window chrome, then restyle every
  Task 9 screen.**
  - Chrome: the viewport uses `with_fullsize_content_view(true)`, `with_title_shown(false)` and
    `with_titlebar_shown(false)`, with a minimum size of about 900×600 pt. Leave room for
    the traffic lights at the top of the sidebar.
  - Settings: a separate window, from `show_viewport_immediate`, opened with ⌘, and from a
    toolbar gear button. If that doesn't work with eframe 0.36's glow backend on macOS,
    use a sheet anchored to the top of the main window. Ledger which one you used.
  - Render code: screens take the state and collect `UiCommand`s, or take a sink, so
    they can be drawn without a live `Hub`.

- [ ] **Step 4: Snapshot renders, so the look can be seen without a screen.** Add
  `egui_kittest` at the exact version that matches egui 0.36.2, as a dev-dependency only,
  with the features that render to an image.
  - Add an `#[ignore]` test, `render_screens`, that draws these from synthetic state
    (`TESTDEVICE01` "Air 2", Answering, with fresh readings, one pending intensity and one
    stale reading; `TESTDEVICE02` "HVAC 2", silent; one Found Nearby hit; a short log):
    - the main window in light and dark, with the first device selected;
    - the Settings window, or sheet;
    - a notice;
  - It writes each render to `target/snapshots/<name>.png`. It never creates a Hub, a
    btleplug Manager or a network connection.
  - Run it with `cargo test -p scent-desktop render_screens -- --ignored`, then view every
    PNG yourself with the Read tool. Compare each against the spec section and fix what
    doesn't match.

- [ ] **Step 5: Run the tests and lints.** Run `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, and
  once `bunx turbo run build test lint fmt:check`. Expected: all pass. The ignored
  snapshot test doesn't run by default.

- [ ] **Step 6: Commit.**

```bash
git add apps/desktop Cargo.lock
git commit -m "Desktop: Apple design language (system colors, SF Pro, source list, grouped boxes)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Add-a-diffuser flow and schedule editor

**Files:**
- Create: `apps/desktop/src/add_flow.rs`, `apps/desktop/src/ui/add_sheet.rs`, `apps/desktop/src/ui/schedule_editor.rs`
- Modify: `apps/desktop/src/hub.rs` (handle `UiCommand::Add`), `apps/desktop/src/app.rs`

**Interfaces:**
- Consumes: `BleHandle`, `MqttHandle`, `Event`, `scent_core::{Command, Profile, Entry, AlwaysOn, trial_entry, local_weekday_and_time}`
- Consumes (as built in Task 9a, `apps/desktop/src/ui/widgets.rs`; see its module index): `switch(ui, &mut bool, label)`, `segmented(ui, &mut T, &[(T, &str)], label) -> Choice<T>`, `grouped(ui, title, |ui| …)`, `grouped_with(ui, title, |ui| accessory, |ui| rows)`, `row(ui, label, |ui| …)`, `secondary(ui, text)`, `footnote(ui, text)`, `notice(ui, NoticeKind, text)` / `notice_with`, `default_button(ui, text)`, `plain_button(ui, text)`; tokens in `ui/theme.rs`; the clock helpers in `ui/clock.rs` (Mac's current UTC offset).
- Produces:
  - `struct AddRequest { ble_id: String, label: String, wifi: Option<(String, Secret)> }`
  - `enum AddStep { ReadInfo, NeedWifi, SaveMqtt, WifiConnect, Restart, WaitConnected, Ping, Done(String /*deviceId*/), Failed(String) }`
  - `add_flow::next(step: &AddStep, info: Option<&DeviceInfo>, wifi_given: bool, connected_device_id: Option<&str>) -> AddStep`, a pure state machine. The order is:
    1. `ReadInfo`;
    2. if WiFi is configured: `SaveMqtt` → `Restart` → `WaitConnected` (up to 90 s, satisfied by `device_connected` on either topic for this device ID) → `Ping` → `Done`;
    3. if not configured and no WiFi given: `NeedWifi`;
    4. if WiFi was given: `SaveMqtt` → `WifiConnect` → `Restart` → `WaitConnected` → …
  - **Schedule editor:**
    - an always-on toggle (default off) with its level (default 1);
    - entry rows with start/end time, weekday toggles S–S, level (default 1), and "runs";
    - "Add a test time starting in 3 minutes", using `trial_entry`;
    - the UTC offset, defaulting to the Mac's current offset;
    - "Send schedule", which confirms "Replace <label>'s stored schedule?" and then submits `UiCommand::SendSchedule`.

- [ ] **Step 1: Write the failing tests** in `add_flow.rs`:
  - `configured_device_skips_wifi`: info with `wifi_configured: true` goes ReadInfo → SaveMqtt → Restart → WaitConnected.
  - `unconfigured_device_asks_for_wifi`: info with false and no WiFi → `NeedWifi`; with WiFi given → SaveMqtt → WifiConnect → Restart.
  - `waits_for_its_own_device_connected`: `WaitConnected` with another device's ID stays put; with its own → `Ping` → `Done(id)`.
  - `times_out_after_90_s`: a pure `wait_outcome(started: i64, now: i64, connected: bool)` gives `Failed` with a message naming WiFi password, 2.4 GHz and range.

- [ ] **Step 2: Run the tests to verify they fail.**

  Run: `cargo test -p scent-desktop add_flow`
  Expected: compile errors.

- [ ] **Step 3: Implement `add_flow.rs`, the hub's `Add` handling, and both sheets.** Over Bluetooth, the soft restart is `Command::SoftRestart` (which becomes `SECRET_COMMAND restart`). Store the new device in `desktop.json` with its label and `ble_id` once it's `Done`.

- [ ] **Step 4: Run the tests and lints.**

  Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings`
  Expected: all pass.

- [ ] **Step 5: Live check (owner).**
  1. **Add the HVAC 2 again.** It's already on the broker, so expect SaveMqtt → Restart → Done within about 60 s.
  2. **Send the Air 2 a test schedule** with "Add a test time starting in 3 minutes", left untouched.

  Expected: the Air 2 starts and stops on time, and the expected-state line matches it.

- [ ] **Step 6: Commit.**

```bash
git add apps/desktop
git commit -m "Desktop: add-a-diffuser flow and schedule editor

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Acceptance with the owner, and docs

**Files:**
- Modify: `README.md`, `CLAUDE.md` (layout and "current phase"), `tools/README.md` (mark the test bench as superseded by the desktop app for everyday use), `docs/findings.md` (any new **[observed]** behavior)

**Interfaces:**
- Consumes: everything above

- [ ] **Step 1: Run the acceptance checklist with the owner,** using the bundled app only and the phone's Bluetooth off. Record each item as pass or fail with a date in `docs/findings.md`:
  1. Scan lists both diffusers.
  2. Both show "Answering" within 30 s of launch.
  3. Power and intensity work on the Air 2 over Auto (Bluetooth), then over forced WiFi.
  4. With the Mac's Bluetooth turned off, Auto falls back to WiFi with the note, within 10 s.
  5. A test schedule starts and stops on the Air 2, and the expected line matches it.
  6. Quitting and relaunching keeps the diffusers and their last readings, marked with their ages.
  7. A wrong broker password shows "Broker login refused" and stops retrying.
  8. No password appears anywhere in the log view or in `desktop.json`. Check with `grep -c` for the test password: expected 0.

- [ ] **Step 2: Update the docs.**
  - `README.md`: how to build (`scripts/bundle-macos.sh`) and why it must be launched with `open`.
  - `CLAUDE.md`: add `packages/scent-core` and `apps/desktop` to the layout, and make Phase 1 current.
  - `tools/README.md`: a note that everyday control moved to the desktop app, and that the bench stays for protocol experiments.

- [ ] **Step 3: Final checks.**

  Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && tools/.venv/bin/python -m pytest tools -q`
  Expected: all green.

- [ ] **Step 4: Commit.**

```bash
git add README.md CLAUDE.md tools/README.md docs/findings.md
git commit -m "Phase 1 acceptance results and docs for the desktop app

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
