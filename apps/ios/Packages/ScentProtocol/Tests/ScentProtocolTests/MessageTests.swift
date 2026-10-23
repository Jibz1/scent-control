// Ported from the tests in `packages/scent-core/src/message.rs`.

import Foundation
import Testing

@testable import ScentProtocol

private let userTopic = "scentiment-mqtt-prod/user/route-test-1"

@Suite struct NotificationTests {
  // Cases ported from tools/tests/test_scentiment_ble.py

  @Test func statusTextReadsTheFourKeys() {
    let n = parseNotification(data("S:1,L:3,C:2,Bat:200-|"))
    #expect(n.kind == .statusText)
    #expect(n.raw == "S:1,L:3,C:2,Bat:200")
    #expect(n.status == Status(power: 1, intensity: 3, charging: 2, battery: 200))
  }

  @Test func statusArrayFromScentAssistantIssue42() {
    let n = parseNotification(
      data(#"{"data":[85,170,11,255,0,2,200,100,255,0,0,255,255,0,48,106]}"#))
    #expect(n.kind == .statusArray)
    #expect(
      n.status
        == Status(
          power: 0, intensity: 0, charging: 2, battery: 200, fan: 255, gasPump: 255, light: 255))
  }

  @Test func statusIndex7IsNotReadAsOil() {
    let n = parseNotification(
      data(#"{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]}"#))
    #expect(
      n.status
        == Status(
          power: 1, intensity: 2, charging: 2, battery: 200, fan: 255, gasPump: 255, light: 255))
  }

  @Test func aShortArrayReadsOnlyTheIndicesPresent() {
    let n = parseNotification(data(#"{"data":[85,170,11,255,1,0,90]}"#))
    #expect(n.kind == .statusArray)
    #expect(n.status == Status(power: 1, charging: 0, battery: 90))
  }

  @Test func arrayEntriesThatAreNotIntegersAreSkipped() {
    let n = parseNotification(data(#"{"data":[85,170,11,255,"x",1.5,90]}"#))
    #expect(n.kind == .statusArray)
    #expect(n.status == Status(battery: 90))
    // As in serde_json, `2.0`, `1e2` and `-0` are not integers, and neither is a number
    // too large for 64 bits.
    let floats = parseNotification(data(#"{"data":[0,0,0,0,2.0,1e2,-0,9223372036854775808,true]}"#))
    #expect(floats.status == Status())
    let edges = parseNotification(
      data(#"{"data":[0,0,0,0,9223372036854775807,-9223372036854775808]}"#))
    #expect(edges.status == Status(power: Int.max, charging: Int.min))
  }

  @Test func singleValuesMapToStatusFields() {
    let power = parseNotification(data("enable:1"))
    #expect(power.kind == .value)
    #expect(power.status.power == 1)
    #expect(parseNotification(data("level:2")).status.intensity == 2)
    #expect(parseNotification(data(" battery:80 ")).status.battery == 80)
    #expect(parseNotification(data("ENABLE:0")).status.power == 0)
  }

  @Test func valuesWithoutAStatusFieldAreStillValues() {
    let n = parseNotification(data("led:50"))
    #expect(n.kind == .value)
    #expect(n.status == Status())
    #expect(parseNotification(data("enable:soon")).status == Status())
  }

  @Test func jsonWithoutADataArrayIsJSON() {
    let n = parseNotification(data(#"{"other":1}"#))
    #expect(n.kind == .json && n.status == Status())
    #expect(parseNotification(data("[1,2]")).kind == .json)
    #expect(parseNotification(data(#"{"data":"x"}"#)).kind == .json)
  }

  @Test func parseNotificationNeverCrashes() {
    let cases: [([UInt8], NoteKind)] = [
      ([0xFF, 0xFE], .unknown),
      ([], .unknown),
      (utf8("-|"), .unknown),
      (utf8("hello"), .unknown),
      (utf8("{not json"), .unknown),
      (utf8("S:1,"), .value),
      (utf8(":5"), .unknown),
    ]
    for (bytes, kind) in cases {
      #expect(parseNotification(Data(bytes)).kind == kind, "\(bytes)")
    }
    #expect(parseNotification(data("hello")).raw == "hello")
    // Bytes that aren't UTF-8 become U+FFFD, one per bad sequence, as in Rust.
    #expect(parseNotification(Data([0xFF, 0xFE])).raw == "\u{FFFD}\u{FFFD}")
  }

  /// Swift only: Rust's `trim` and integer parsing, not Swift's.
  @Test func numbersAndSpacesAreReadAsRustReadsThem() {
    #expect(parseNotification(data("enable:+1")).status.power == 1)
    #expect(parseNotification(data("enable:-0")).status.power == 0)
    #expect(parseNotification(data("enable:9223372036854775808")).status.power == nil)
    #expect(parseNotification(data("enable:٣")).status.power == nil)
    // Unicode spaces are trimmed, as Rust's `trim` does.
    #expect(parseNotification(data("\u{2003}enable:1\u{3000}-|\u{A0}")).raw == "enable:1")
  }
}

@Suite struct InfoTests {
  @Test func parseInfoReadsSevenFields() throws {
    #expect(
      try parseInfo("1|Diffuser Air 2|Black|TESTDEVICE01|2.3.1|true|3")
        == DeviceInfo(
          manufacturerID: "1", modelName: "Diffuser Air 2", variantName: "Black",
          deviceID: "TESTDEVICE01", firmware: "2.3.1", wifiConfigured: true, wifiStatus: 3))
  }

  @Test func parseInfoTrimsFieldsAndIgnoresExtras() throws {
    let info = try parseInfo(" 03 | Hvac |Black| TESTDEVICE02 |2.30| False |2|extra\r\n")
    #expect(info.modelName == "Hvac")
    #expect(info.deviceID == "TESTDEVICE02")
    #expect(!info.wifiConfigured)
    #expect(info.wifiStatus == 2)
    #expect(try parseInfo("01|Diffuser|Black|X|2.30|TRUE|0").wifiConfigured)
  }

  @Test func parseInfoRejectsShortTextAndNamesTheCount() {
    let error = #expect(throws: InfoError.self) { try parseInfo("1|Diffuser Air 2|Black") }
    #expect(
      error?.message
        == #"expected 7 '|'-separated fields in the 0xFEF4 info string, got 3: "1|Diffuser Air 2|Black""#
    )
    #expect(throws: InfoError.self) { try parseInfo("") }
  }

  @Test func parseInfoRejectsAStatusThatIsNotASmallNumber() throws {
    let error = #expect(throws: InfoError.self) {
      try parseInfo("01|Diffuser|Black|TESTDEVICE01|2.30|true|ok")
    }
    #expect(error?.message == #"the WiFi status field is not a number from 0 to 255: "ok""#)
    #expect(throws: InfoError.self) {
      try parseInfo("01|Diffuser|Black|TESTDEVICE01|2.30|true|300")
    }
    // As in Rust, a `+` is allowed and a `-` is not, even on zero.
    #expect(try parseInfo("01|Diffuser|Black|TESTDEVICE01|2.30|true|+7").wifiStatus == 7)
    #expect(throws: InfoError.self) { try parseInfo("01|Diffuser|Black|TESTDEVICE01|2.30|true|-0") }
  }

  /// Swift only: the text in the message is shown as Rust's `{:?}` shows it.
  @Test func theErrorShowsTheTextAsRustDebugDoes() {
    let error = #expect(throws: InfoError.self) { try parseInfo("a\"b\\c\n\t\0\u{7F}\u{301}é|x") }
    #expect(
      error?.message
        == #"expected 7 '|'-separated fields in the 0xFEF4 info string, got 2: "a\"b\\c\n\t\0\u{7f}\u{301}é|x""#
    )
    #expect(error?.localizedDescription == error?.message)
  }
}

@Suite struct MqttTests {
  // Cases ported from tools/tests/test_control_server.py

  @Test func repairsTheUnquotedNotificationData() throws {
    // Exactly what an Air 2 (firmware 2.30) sent on 2026-10-20 in reply to GET_STATUS.
    let raw =
      #"{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:0,"rssi":-46}}"#
    let m = parseMqtt(topic: userTopic, body: data(raw))
    #expect(m.action == "notification")
    #expect(m.deviceID == "TESTDEVICE01")
    #expect(!m.validJSON)
    #expect(m.rssi == -46)
    #expect(try #require(m.status).power == 0)
    #expect(utf8(m.raw) == utf8(raw))
  }

  @Test func readsAStatusArrayInsideUpdateStatus() throws {
    let raw =
      #"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":"{\"data\":[85,170,11,255,1,2,200,100,255,3,3,255,255,0,41,106]}"}}"#
    let m = parseMqtt(topic: userTopic, body: data(raw))
    #expect(m.validJSON)
    let status = try #require(m.status)
    #expect(status.power == 1 && status.intensity == 3)
  }

  @Test func readsAStatusArrayGivenAsAnObject() throws {
    // Firmware 2.30 over MQTT sends the array as an object, not a string (observed 2026-10-20).
    let raw =
      #"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}"#
    let m = parseMqtt(topic: userTopic, body: data(raw))
    #expect(m.validJSON)
    let status = try #require(m.status)
    #expect(status.power == 1 && status.intensity == 2)
    #expect(m.rssi == -66)
  }

  @Test func takesTheDeviceIDFromADeviceTopic() {
    let m = parseMqtt(
      topic: "scentiment-mqtt-prod/device/TESTDEVICE01",
      body: data(#"{"action":"PING","payload":{"userid":"route-test-1"}}"#))
    #expect(m.direction == .toDevice)
    #expect(m.deviceID == "TESTDEVICE01")
    #expect(m.action == "PING")
    #expect(m.status == nil)
  }

  @Test func thePayloadDeviceIDWinsOverTheTopic() {
    let m = parseMqtt(
      topic: "scentiment-mqtt-prod/device/TESTDEVICE01",
      body: data(#"{"action":"X","payload":{"deviceId":"TESTDEVICE02"}}"#))
    #expect(m.deviceID == "TESTDEVICE02")
    let empty = parseMqtt(
      topic: "scentiment-mqtt-prod/device/TESTDEVICE01",
      body: data(#"{"action":"X","payload":{"deviceId":""}}"#))
    #expect(empty.deviceID == "TESTDEVICE01")
  }

  @Test func readsDeviceConnected() {
    let m = parseMqtt(
      topic: "scentiment-mqtt-prod/server-notifications/device",
      body: data(
        #"{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}"#
      ))
    #expect(m.direction == .fromDevice)
    #expect(m.deviceID == "TESTDEVICE01")
    #expect(
      m.connected == Connected(fw: "2.30", model: "Diffuser", variant: "Black", manufacturer: "01"))
  }

  @Test func readsAcknowledgementResults() {
    func ack(_ result: String) -> Ack? {
      let msg = JSON.object(["user_id": .string("route-test-1"), "result": .string(result)]).text
      let body = JSON.object([
        "action": .string("acknowledgement"),
        "payload": .object(["deviceId": .string("TESTDEVICE01"), "msg": .string(msg)]),
      ])
      return parseMqtt(topic: userTopic, body: Data(body.bytes)).ack
    }
    #expect(ack("ADDED") == .added)
    #expect(ack("ALREADY_ADDED") == .alreadyAdded)
    #expect(ack("NOPE") == .other("NOPE"))

    for payload in [
      #"{"msg":"not json"}"#,
      #"{"msg":"{\"user_id\":\"x\"}"}"#,
      #"{"msg":5}"#,
      "{}",
    ] {
      let body = #"{"action":"acknowledgement","payload":"# + payload + "}"
      #expect(parseMqtt(topic: userTopic, body: data(body)).ack == nil, "\(payload)")
    }
    let elsewhere = parseMqtt(
      topic: userTopic,
      body: data(#"{"action":"other","payload":{"msg":"{\"result\":\"ADDED\"}"}}"#))
    #expect(elsewhere.ack == nil)
  }

  @Test func parseMqttNeverCrashes() {
    let bodies: [[UInt8]] = [
      [0xFF, 0xFE],
      [],
      utf8("{"),
      utf8("null"),
      utf8(#"{"action":5}"#),
      utf8(#"{"payload":{"data":{"data":"x"}}}"#),
    ]
    for body in bodies {
      let m = parseMqtt(topic: "scentiment-mqtt-prod/user/route-test-1", body: Data(body))
      #expect(!m.raw.isEmpty || body.isEmpty)
    }
  }

  @Test func bodiesThatAreNotMessagesGiveAnEmptyResult() {
    let m = parseMqtt(topic: "anything", body: Data([0xFF] + utf8(" not json at all")))
    #expect(m.action == nil && m.status == nil && m.deviceID == nil)
    #expect(!m.validJSON && !m.raw.isEmpty)

    let null = parseMqtt(topic: "anything", body: data("null"))
    #expect(null.validJSON && null.action == nil)
    #expect(!parseMqtt(topic: "anything", body: Data()).validJSON)
    // A non-string action is not an action.
    #expect(parseMqtt(topic: "anything", body: data(#"{"action":5}"#)).action == nil)
  }

  @Test func theRepairQuotesOnlyValuesThatAreNotJSONValues() {
    #expect(
      quoteUnquotedData(#"{"a":{"data":enable:0,"rssi":-1}}"#)
        == #"{"a":{"data":"enable:0","rssi":-1}}"#)
    #expect(quoteUnquotedData(#"{"data":S:1}"#) == #"{"data":"S:1"}"#)
    // The value runs to the end of the text when nothing closes it.
    #expect(quoteUnquotedData(#"{"data":abc"#) == #"{"data":"abc""#)
    for untouched in [
      #"{"data":"x"}"#, #"{"data":{"data":[1]}}"#, #"{"data":[1,2]}"#, "no data key", "é",
    ] {
      #expect(utf8(quoteUnquotedData(untouched)) == utf8(untouched))
    }
    #expect(quoteUnquotedData(#"{"data":a,"x":{"data":b}}"#) == #"{"data":"a","x":{"data":"b"}}"#)
    // Quotes inside the value are escaped, not left to break the JSON.
    #expect(quoteUnquotedData(#"{"data":a"b}"#) == #"{"data":"a\"b"}"#)
  }

  /// Swift only: the topic prefix is compared byte for byte, as Rust does.
  @Test func theDeviceTopicPrefixIsComparedByteForByte() {
    let combined = parseMqtt(topic: "scentiment-mqtt-prod/device/\u{301}X", body: Data())
    #expect(combined.direction == .toDevice)
    #expect(combined.deviceID == "\u{301}X")
    let bare = parseMqtt(topic: "scentiment-mqtt-prod/device/", body: Data())
    #expect(bare.direction == .toDevice && bare.deviceID == "")
    #expect(parseMqtt(topic: "scentiment-mqtt-prod/device", body: Data()).deviceID == nil)
  }
}
