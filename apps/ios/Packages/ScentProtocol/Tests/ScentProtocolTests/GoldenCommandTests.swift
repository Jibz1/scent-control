// The golden commands: `fixtures/commands/commands.json` holds the exact text the Rust
// crate writes for a fixed set of commands (`packages/scent-core/tests/commands.rs`
// builds the same list and checks the same file). Here the Swift codec must write the
// same bytes.

import Foundation
import Testing

@testable import ScentProtocol

private struct Golden: Decodable {
  struct Case: Decodable {
    let name: String
    let bleFrame: String
    let mqttBody: String
    let redactedBluetooth: String
    let redactedWifi: String

    private enum CodingKeys: String, CodingKey {
      case name
      case bleFrame = "ble_frame"
      case mqttBody = "mqtt_body"
      case redactedBluetooth = "redacted_bluetooth"
      case redactedWifi = "redacted_wifi"
    }
  }

  let commands: [Case]
}

private func entry(_ start: String, _ end: String, _ weekdays: [Int], _ n: Int, _ enable: Bool)
  -> Entry
{
  Entry(start: hhmm(start), end: hhmm(end), weekdays: weekdays, level: level(n), enable: enable)
}

/// An always-on-only profile at `tz` hours: the shortest `UPDATE_PROFILE`, for checking how
/// the time zone is written.
private func zone(_ tz: Double) -> Command {
  .updateProfile(
    Profile(
      alwaysOn: AlwaysOn(enable: true, level: level(1)), entries: [], timezoneHours: tz,
      flag: .alwaysOnOnly))
}

/// The commands in the golden file, in its order: the same list as the Rust test's.
private let commands: [(String, Command)] = {
  let fake = Secret("example-password")
  return [
    ("turn_on", .turnOn(true)),
    ("turn_off", .turnOn(false)),
    ("set_level_1", .setLevel(level(1))),
    ("set_level_2", .setLevel(level(2))),
    ("set_level_3", .setLevel(level(3))),
    ("ping", .ping(userid: nil)),
    ("ping_with_userid", .ping(userid: "route-test-1")),
    ("ping_with_empty_userid", .ping(userid: "")),
    ("get_status", .getStatus),
    ("sync_time", .syncTime(serverTime: 1_792_510_200)),
    ("sync_time_zero", .syncTime(serverTime: 0)),
    ("sync_time_max", .syncTime(serverTime: UInt64.max)),
    (
      "save_mqtt",
      .saveMqtt(broker: "wss://broker.example.test/mqtt", username: "example-user", password: fake)
    ),
    ("wifi_connect", .wifiConnect(ssid: "ExampleWiFi", password: fake)),
    (
      "wifi_connect_escapes",
      .wifiConnect(
        ssid: "Caf\u{e9} \"5G\" / \u{1f33f}\\ \t\n\r\u{8}\u{c}\u{0}\u{1}\u{1f}\u{7f}\u{2028}",
        password: Secret("p\\w\"\u{e4}\u{1}"))
    ),
    ("soft_restart", .softRestart),
    (
      "update_profile_replace",
      .updateProfile(
        Profile(
          alwaysOn: AlwaysOn(enable: false, level: level(1)),
          entries: [
            entry("07:00", "09:00", [1, 2, 3, 4, 5], 3, true),
            entry("18:30", "22:00", [0, 6], 2, true),
            entry("12:00", "12:30", [3], 1, false),
          ],
          timezoneHours: 5.5, flag: .replace))
    ),
    (
      "update_profile_always_on_only",
      .updateProfile(
        Profile(
          alwaysOn: AlwaysOn(enable: true, level: level(2)),
          entries: [entry("07:00", "09:00", [1], 3, true)], timezoneHours: -5.0, flag: .alwaysOnOnly
        ))
    ),
    (
      "update_profile_replace_empty",
      .updateProfile(
        Profile(
          alwaysOn: AlwaysOn(enable: false, level: level(3)), entries: [], timezoneHours: 0.0,
          flag: .replace))
    ),
    ("update_profile_zone_zero", zone(0.0)),
    ("update_profile_zone_negative_zero", zone(-0.0)),
    ("update_profile_zone_quarter", zone(5.75)),
    ("update_profile_zone_negative_half", zone(-9.5)),
    ("update_profile_zone_fourteen", zone(14.0)),
    ("update_profile_zone_tenth", zone(0.1)),
    ("update_profile_zone_third", zone(1.0 / 3.0)),
    // Plain decimals from 1e-5 up to below 1e16, exponents outside that.
    ("update_profile_zone_1e15", zone(1e15)),
    ("update_profile_zone_1e16", zone(1e16)),
    ("update_profile_zone_1e-5", zone(1e-5)),
    ("update_profile_zone_1e-6", zone(1e-6)),
    ("update_profile_zone_small", zone(1.25e-5)),
    ("update_profile_zone_tiny", zone(-4.5e-9)),
    ("update_profile_zone_large", zone(-1.234_567_89e20)),
    ("update_profile_zone_1e23", zone(1e23)),
    ("update_profile_zone_max", zone(Double.greatestFiniteMagnitude)),
    ("update_profile_zone_min_positive", zone(Double.leastNormalMagnitude)),
    ("update_profile_zone_subnormal", zone(5e-324)),
    // serde_json writes a number that isn't finite as null.
    ("update_profile_zone_nan", zone(Double.nan)),
    ("update_profile_zone_infinity", zone(Double.infinity)),
  ]
}()

private func golden() throws -> Golden {
  let url = repoRoot.appendingPathComponent("fixtures/commands/commands.json")
  return try JSONDecoder().decode(Golden.self, from: Data(contentsOf: url))
}

@Suite struct GoldenCommandTests {
  @Test func theListOfCommandsIsTheRustOne() throws {
    #expect(try golden().commands.map(\.name) == commands.map(\.0))
  }

  @Test func commandsMatchTheGoldenFileByteForByte() throws {
    let cases = try golden().commands
    for ((name, command), want) in zip(commands, cases) {
      #expect(
        Array(command.bleFrame()) == utf8(want.bleFrame),
        "\(name) ble_frame: \(text(command.bleFrame()))")
      #expect(
        Array(command.mqttBody()) == utf8(want.mqttBody),
        "\(name) mqtt_body: \(text(command.mqttBody()))")
      #expect(
        utf8(command.redacted(via: .bluetooth)) == utf8(want.redactedBluetooth),
        "\(name) redacted_bluetooth: \(command.redacted(via: .bluetooth))")
      #expect(
        utf8(command.redacted(via: .wifi)) == utf8(want.redactedWifi),
        "\(name) redacted_wifi: \(command.redacted(via: .wifi))")
    }
  }

  @Test func onlyTheFramesCarryTheFakePassword() {
    for (name, command) in commands {
      for via in [Via.bluetooth, .wifi] {
        #expect(!command.redacted(via: via).contains("example-password"), "\(name)")
      }
      let carries = name == "save_mqtt" || name == "wifi_connect"
      #expect(text(command.bleFrame()).contains("example-password") == carries, "\(name)")
      #expect(text(command.mqttBody()).contains("example-password") == carries, "\(name)")
    }
  }

  /// The payload alone is the frame's `payload` value, written the same way.
  @Test func thePayloadIsTheFramesPayload() throws {
    for (name, command) in commands {
      let body = text(command.mqttBody())
      let action = command.action(via: .wifi)
      if let payload = command.payloadJSON(via: .wifi) {
        #expect(body == #"{"action":""# + action + #"","payload":"# + payload + "}", "\(name)")
      } else {
        #expect(body == #"{"action":""# + action + #""}"#, "\(name)")
      }
    }
  }
}
