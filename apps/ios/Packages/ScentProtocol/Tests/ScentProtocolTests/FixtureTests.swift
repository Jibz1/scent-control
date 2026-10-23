// The fixture contract, ported from `packages/scent-core/tests/fixtures.rs`: every
// captured message in `fixtures/scentiment/` must parse the way the Phase 0 tools read it.

import Foundation
import Testing

@testable import ScentProtocol

private let syntheticIDs = ["TESTDEVICE01", "TESTDEVICE02"]

private struct Fixture: Decodable {
  var name = ""
  let device: String
  let channel: String
  let topic: String?
  let text: String

  private enum CodingKeys: String, CodingKey {
    case device, channel, topic, text
  }
}

/// Every fixture, read with Foundation's decoder so the files don't depend on our reader.
private func fixtures() throws -> [Fixture] {
  let folder = repoRoot.appendingPathComponent("fixtures/scentiment")
  var found: [Fixture] = []
  for url in try FileManager.default.contentsOfDirectory(
    at: folder, includingPropertiesForKeys: nil)
  where url.pathExtension == "json" {
    var fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: url))
    fixture.name = url.deletingPathExtension().lastPathComponent
    found.append(fixture)
  }
  return found.sorted { $0.name < $1.name }
}

private func fixture(_ name: String) throws -> Fixture {
  try #require(try fixtures().first { $0.name == name }, "no fixture named \(name)")
}

private func onChannel(_ channel: String) throws -> [Fixture] {
  try fixtures().filter { $0.channel == channel }
}

private func parseMqttFixture(_ f: Fixture) throws -> MqttMessage {
  parseMqtt(topic: try #require(f.topic, "\(f.name): no topic"), body: data(f.text))
}

@Suite struct FixtureTests {
  @Test func theCapturedFixturesAreAllFound() throws {
    // Phase 0 captured 16. More may be added; fewer means a file went missing.
    let all = try fixtures()
    #expect(all.count >= 16, "\(all.map(\.name))")
    #expect(try onChannel("ble-info").count >= 2)
    #expect(try onChannel("ble-notify").count >= 7)
    #expect(try onChannel("mqtt").count >= 7)
  }

  @Test func infoFixturesParseWithASyntheticDeviceID() throws {
    for f in try onChannel("ble-info") {
      let info = try parseInfo(f.text)
      #expect(syntheticIDs.contains(info.deviceID), "\(f.name)")
      #expect(info.deviceID == (f.device == "air2" ? "TESTDEVICE01" : "TESTDEVICE02"), "\(f.name)")
      #expect(info.wifiConfigured && info.firmware == "2.30", "\(f.name)")
    }
  }

  @Test func notifyFixturesAreRecognised() throws {
    for f in try onChannel("ble-notify") {
      let kind = parseNotification(data(f.text)).kind
      #expect(kind != .unknown, "\(f.name)")
      let expected: NoteKind =
        f.name.contains("status-array")
        ? .statusArray : f.name.contains("status-text") ? .statusText : .value
      #expect(kind == expected, "\(f.name)")
    }
  }

  @Test func mqttFixturesParseWithAnActionAndASyntheticDeviceID() throws {
    for f in try onChannel("mqtt") {
      let m = try parseMqttFixture(f)
      #expect(m.action != nil, "\(f.name)")
      let id = try #require(m.deviceID, "\(f.name): no device id")
      #expect(syntheticIDs.contains(id) && Topics.validDeviceID(id), "\(f.name)")
      #expect(m.direction == .fromDevice, "\(f.name)")
      #expect(utf8(m.raw) == utf8(f.text), "\(f.name)")
      // The device's own invalid JSON is the only message that needs repair.
      #expect(m.validJSON == (f.name != "air2-mqtt-notification-invalid-json"), "\(f.name)")
    }
  }

  @Test func theInvalidJSONFixtureIsRepaired() throws {
    let m = try parseMqttFixture(fixture("air2-mqtt-notification-invalid-json"))
    #expect(!m.validJSON)
    #expect(m.action == "notification")
    #expect(try #require(m.status, "repaired status").power == 0)
    #expect(m.rssi == -46)
  }

  @Test func theStatusObjectFixtureGivesAStatus() throws {
    let m = try parseMqttFixture(fixture("air2-mqtt-update-status-object"))
    let status = try #require(m.status)
    #expect(status.power == 1 && status.intensity == 2 && status.battery == 200)
    #expect(m.rssi == -66)
  }

  @Test func theAckFixturesCarryTheirResult() throws {
    #expect(try parseMqttFixture(fixture("air2-mqtt-ack-added")).ack == .added)
    #expect(try parseMqttFixture(fixture("air2-mqtt-ack-already-added")).ack == .alreadyAdded)
  }

  @Test func theDeviceConnectedFixturesCarryModelInfo() throws {
    for name in [
      "air2-mqtt-device-connected-server-notifications",
      "air2-mqtt-device-connected-user",
      "hvac-mqtt-device-connected-user",
    ] {
      let connected = try #require(try parseMqttFixture(fixture(name)).connected, "\(name)")
      #expect(connected.fw == "2.30", "\(name)")
      #expect(connected.variant == "Black", "\(name)")
    }
    let hvac = try parseMqttFixture(fixture("hvac-mqtt-device-connected-user"))
    #expect(hvac.connected?.model == "Hvac")
  }
}
