// Ported from the tests in `packages/scent-core/src/command.rs`.

import Foundation
import Testing

@testable import ScentProtocol

@Suite struct CommandTests {
  @Test func bleFramesAreCompactJSONWithTerminator() {
    #expect(
      text(Command.setLevel(level(2)).bleFrame())
        == #"{"action":"SET_LEVEL","payload":{"intensity":2}}-|"#)
    #expect(text(Command.ping(userid: nil).bleFrame()) == #"{"action":"PING"}-|"#)
    #expect(text(Command.turnOn(false).mqttBody()) == #"{"action":"TURN_ON","payload":{"on":0}}"#)
  }

  @Test func softRestartDiffersPerTransport() {
    #expect(
      text(Command.softRestart.bleFrame())
        == #"{"action":"SECRET_COMMAND","payload":{"command":"restart"}}-|"#)
    #expect(
      text(Command.softRestart.mqttBody()) == #"{"action":"RESET","payload":{"mode":"soft"}}"#)
  }

  @Test func secretsNeverAppearInRedactedOrDebugOutput() {
    let c = Command.wifiConnect(ssid: "ExampleWiFi", password: Secret("hunter2!"))
    #expect(!c.redacted(via: .bluetooth).contains("hunter2!"))
    #expect(c.redacted(via: .bluetooth).contains(#""password":"***""#))
    #expect(!String(describing: c).contains("hunter2!"))
    #expect(!String(reflecting: c).contains("hunter2!"))
    var dumped = ""
    dump(c, to: &dumped)
    #expect(!dumped.contains("hunter2!"))
    #expect(text(c.bleFrame()).contains("hunter2!"))  // the device does need it
  }

  @Test func levelIsOneToThree() {
    #expect(Level(0) == nil && Level(4) == nil && Level(1) != nil)
  }

  @Test func longFramesSplitInto177ByteChunksThatRejoin() {
    let c = Command.saveMqtt(
      broker: "wss://\(String(repeating: "a", count: 300))/mqtt", username: "u",
      password: Secret("p"))
    let frame = c.bleFrame()
    let parts = chunks(frame)
    #expect(parts.allSatisfy { $0.count <= 177 })
    #expect(parts.reduce(Data(), +) == frame)
    #expect(parts.count == (frame.count + 176) / 177)
  }

  // Expected bytes below were produced by tools/scentiment_ble.py (encode_command and redact_frame).
  @Test func framesMatchThePythonToolsByteForByte() {
    let wifi = Command.wifiConnect(ssid: "ExampleWiFi", password: Secret("hunter2!"))
    #expect(
      text(wifi.bleFrame())
        == #"{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"hunter2!"}}-|"#)
    let mqtt = Command.saveMqtt(
      broker: "wss://b.example.test/mqtt", username: "dev1", password: Secret("s3cret"))
    #expect(
      text(mqtt.bleFrame())
        == #"{"action":"SAVE_MQTT","payload":{"broker":"wss://b.example.test/mqtt","username":"dev1","password":"s3cret"}}-|"#
    )
    #expect(text(Command.turnOn(true).bleFrame()) == #"{"action":"TURN_ON","payload":{"on":1}}-|"#)
    #expect(
      text(Command.ping(userid: "example-route").bleFrame())
        == #"{"action":"PING","payload":{"userid":"example-route"}}-|"#)
    #expect(text(Command.getStatus.bleFrame()) == #"{"action":"GET_STATUS","payload":{}}-|"#)
    #expect(
      text(Command.syncTime(serverTime: 1_790_000_000).bleFrame())
        == #"{"action":"SYNC_TIME","payload":{"serverTime":1790000000}}-|"#)
    let quoted = Command.wifiConnect(ssid: "Caf\u{e9} \"5G\"", password: Secret("p\\w"))
    #expect(
      Array(quoted.bleFrame())
        == utf8(
          "{\"action\":\"WIFI_CONNECT\",\"payload\":{\"ssid\":\"Caf\u{e9} \\\"5G\\\"\",\"password\":\"p\\\\w\"}}-|"
        ))
  }

  @Test func redactedTextMatchesThePythonRedactFrame() {
    let mqtt = Command.saveMqtt(
      broker: "wss://b.example.test/mqtt", username: "dev1", password: Secret("s3cret"))
    let shown =
      #"{"action":"SAVE_MQTT","payload":{"broker":"wss://b.example.test/mqtt","username":"dev1","password":"***"}}"#
    #expect(mqtt.redacted(via: .bluetooth) == shown + "-|")
    #expect(mqtt.redacted(via: .wifi) == shown)
    #expect(Command.ping(userid: nil).redacted(via: .wifi) == #"{"action":"PING"}"#)
  }

  // Expected bytes below were produced by tools/scentiment_ble.py (build_profile and encode_command).
  @Test func updateProfileFramesMatchThePythonToolsOnBothTransports() {
    let timed = Profile(
      alwaysOn: AlwaysOn(enable: false, level: level(1)),
      entries: [
        Entry(
          start: hhmm("07:00"), end: hhmm("09:00"), weekdays: [1, 2, 3, 4, 5], level: level(3),
          enable: true)
      ],
      timezoneHours: -5.0, flag: .replace)
    let body =
      #"{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},{"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":3,"fan":0,"led":0,"timer":0},"enable":true}]},"timezone":-5.0,"schedule_flag":1}}"#
    let c = Command.updateProfile(timed)
    #expect(text(c.bleFrame()) == body + "-|")
    #expect(text(c.mqttBody()) == body)

    let alwaysOn = Command.updateProfile(
      Profile(
        alwaysOn: AlwaysOn(enable: true, level: level(1)), entries: [], timezoneHours: 5.5,
        flag: .alwaysOnOnly))
    #expect(
      text(alwaysOn.bleFrame())
        == #"{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":true,"isFactory":true}]},"timezone":5.5,"schedule_flag":0}}-|"#
    )
  }

  @Test func updateProfileHasNothingToRedact() {
    let c = Command.updateProfile(
      Profile(
        alwaysOn: AlwaysOn(enable: false, level: level(2)), entries: [], timezoneHours: 0.0,
        flag: .replace))
    for via in [Via.bluetooth, .wifi] {
      #expect(c.action(via: via) == "UPDATE_PROFILE")
      let frame = text(via == .bluetooth ? c.bleFrame() : c.mqttBody())
      #expect(c.redacted(via: via) == frame)
    }
  }

  @Test func secretPrintsStarsAndExposesOnlyOnRequest() {
    let s = Secret("hunter2!")
    #expect("\(s)" == "***")
    #expect(String(reflecting: s) == "***")
    var dumped = ""
    dump(s, to: &dumped)
    #expect(!dumped.contains("hunter2!"))
    #expect(s.expose() == "hunter2!")
  }

  @Test func levelRoundTripsAndChunksCoverTheBoundary() {
    #expect((1...3).map { Level($0)!.value } == [1, 2, 3])
    #expect(chunks(Data(count: 177)).count == 1)
    #expect(chunks(Data(count: 178)).map(\.count) == [177, 1])
    #expect(chunks(Data()).isEmpty)
  }

  // Swift only: the parts of the API the Rust tests reach through `payload` and `action`.

  @Test func chunksOfASliceAreIndexedFromZero() {
    let frame = Data(count: 400)[10...]
    let parts = chunks(frame)
    #expect(parts.map(\.count) == [177, 177, 36])
    #expect(parts.allSatisfy { $0.startIndex == 0 })
  }

  @Test func payloadJSONIsThePayloadAloneWithSecrets() {
    #expect(Command.ping(userid: nil).payloadJSON(via: .wifi) == nil)
    #expect(Command.setLevel(level(3)).payloadJSON(via: .bluetooth) == #"{"intensity":3}"#)
    #expect(Command.getStatus.payloadJSON(via: .wifi) == "{}")
    #expect(Command.softRestart.payloadJSON(via: .bluetooth) == #"{"command":"restart"}"#)
    #expect(Command.softRestart.payloadJSON(via: .wifi) == #"{"mode":"soft"}"#)
    let save = Command.saveMqtt(
      broker: "wss://broker.example.test/mqtt", username: "example-user",
      password: Secret("example-password"))
    #expect(
      save.payloadJSON(via: .wifi)
        == #"{"broker":"wss://broker.example.test/mqtt","username":"example-user","password":"example-password"}"#
    )
    let profile = Profile(
      alwaysOn: AlwaysOn(enable: true, level: level(2)), entries: [], timezoneHours: -3.5,
      flag: .alwaysOnOnly)
    #expect(Command.updateProfile(profile).payloadJSON(via: .wifi) == profile.payloadJSON())
  }

  @Test func actionsAreTheFirmwareNames() {
    let all: [(Command, String, String)] = [
      (.turnOn(true), "TURN_ON", "TURN_ON"),
      (.setLevel(level(1)), "SET_LEVEL", "SET_LEVEL"),
      (.ping(userid: nil), "PING", "PING"),
      (.getStatus, "GET_STATUS", "GET_STATUS"),
      (.syncTime(serverTime: 0), "SYNC_TIME", "SYNC_TIME"),
      (.saveMqtt(broker: "b", username: "u", password: Secret("p")), "SAVE_MQTT", "SAVE_MQTT"),
      (.wifiConnect(ssid: "s", password: Secret("p")), "WIFI_CONNECT", "WIFI_CONNECT"),
      (.softRestart, "SECRET_COMMAND", "RESET"),
      (
        .updateProfile(
          Profile(alwaysOn: AlwaysOn(enable: true, level: level(1)), entries: [], timezoneHours: 0)
        ), "UPDATE_PROFILE", "UPDATE_PROFILE"
      ),
    ]
    for (command, bluetooth, wifi) in all {
      #expect(command.action(via: .bluetooth) == bluetooth)
      #expect(command.action(via: .wifi) == wifi)
    }
  }
}
