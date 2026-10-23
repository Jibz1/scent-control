// The listener's records and heartbeat (spec 2026-10-22-listener-design §6.8). The ledger's
// tests are ported from the listener tests in `apps/desktop/src/state.rs`; the engine's run
// on fake radios, as `EngineTests.swift` does.

import Foundation
import ScentProtocol
import Testing

@testable import ScentEngine

private let id1 = "TESTDEVICE01"
private let ble1 = UUID(uuidString: "AAAAAAAA-2222-3333-4444-555555555555")!
/// The clock for events that carry no time of their own.
private let now = 5_000
private let userTopic = "scentiment-mqtt-prod/user/route-test-1"
private let state1Topic = "scent-control/v1/state/TESTDEVICE01"
private let listenerTopic = "scent-control/v1/listener"
private let connected1 =
  #"{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}"#

private func mqtt(_ topic: String, _ body: String, _ at: Int) -> EngineEvent {
  .mqtt(topic: topic, body: Data(body.utf8), at: at)
}

/// An `update_status` message whose `data` is Bluetooth-style text such as `S:1,L:2`.
private func report(_ id: String, _ data: String, _ at: Int) -> EngineEvent {
  mqtt(
    userTopic, #"{"action":"update_status","payload":{"deviceId":"\#(id)","data":"\#(data)"}}"#, at)
}

/// The listener's record of TESTDEVICE01, written at `at`: `fields` and `extra` are JSON
/// members.
private func shared(_ fields: String, _ extra: String = "", _ at: Int) -> EngineEvent {
  mqtt(state1Topic, #"{"v":1,"updated_at":\#(at),"fields":{\#(fields)}\#(extra)}"#, at)
}

private func sent(_ id: String, power: Bool?, at: Int) -> EngineEvent {
  .sent(
    deviceID: id, summary: Command.turnOn(true).redacted(via: .wifi), via: .wifi, at: at,
    power: power,
    intensity: nil, profile: nil)
}

/// A ledger that knows TESTDEVICE01, an Air 2, from its Bluetooth info.
private func knownAir2() -> Ledger {
  var ledger = Ledger()
  let info = DeviceInfo(
    manufacturerID: "01", modelName: "Diffuser", variantName: "Black", deviceID: id1,
    firmware: "2.30",
    wifiConfigured: true, wifiStatus: 3)
  ledger.apply(.bluetoothInfo(bleID: ble1, info: info, at: 1), now: now)
  return ledger
}

@Suite struct ListenerLedger {
  @Test func aNewerListenerReadingIsAdoptedAndAnOlderOneIsNot() {
    var ledger = knownAir2()
    ledger.apply(report(id1, "S:1,L:2", 100), now: now)
    ledger.apply(
      shared(
        #""power":{"value":0,"at":150},"intensity":{"value":3,"at":50},"fan":{"value":1,"at":150}"#,
        "", 160),
      now: now)
    let device = ledger.devices[id1]!
    #expect(device.readings[.power] == Reported(value: 0, at: 150, via: .wifi))
    #expect(device.readings[.intensity] == Reported(value: 2, at: 100, via: .wifi))
    // The fan is the HVAC's; the app has no reading for it.
    #expect(device.readings.count == 2)
  }

  @Test func aDiffuserKnownOnlyToTheListenerIsAdded() {
    var ledger = Ledger()
    ledger.apply(shared(#""power":{"value":1,"at":90}"#, #","heard_at":95"#, 100), now: now)
    let device = ledger.devices[id1]
    #expect(device?.label == id1)
    #expect(device?.readings[.power]?.value == 1)
    #expect(device?.heardWifi == 95)
    #expect(ledger.log.isEmpty)
  }

  @Test func listenerMessagesOfAnotherVersionOrFromTheFutureChangeNothingAndAreNotLogged() {
    for (topic, body) in [
      (state1Topic, #"{"v":2,"updated_at":60,"fields":{}}"#),
      (state1Topic, #"{"v":1,"updated_at":\#(60 + 86_401),"fields":{}}"#),
      (state1Topic, "{ not json"),
      ("scent-control/v1/state/testdevice01", #"{"v":1,"updated_at":60,"fields":{}}"#),
      (listenerTopic, #"{"v":2,"running":true,"at":60}"#),
      (listenerTopic, "{ not json"),
    ] {
      var ledger = knownAir2()
      let before = ledger
      ledger.apply(mqtt(topic, body, 60), now: now)
      #expect(ledger == before, "\(topic) \(body)")
    }
  }

  @Test func theListenersHeardAndPingTimesCountWhenNewer() {
    var ledger = knownAir2()
    ledger.apply(report(id1, "S:1,L:2", 100), now: now)
    ledger.apply(shared("", #","heard_at":120,"ping_at":110"#, 130), now: now)
    #expect(ledger.devices[id1]?.heardWifi == 120 && ledger.devices[id1]?.pingAt == 110)
    ledger.apply(shared("", #","heard_at":90,"ping_at":80"#, 140), now: now)
    #expect(ledger.devices[id1]?.heardWifi == 120 && ledger.devices[id1]?.pingAt == 110)
  }

  @Test func aReadingFromBeforeTheLastReconnectIsNotCurrent() {
    var ledger = knownAir2()
    ledger.apply(report(id1, "S:1,L:2", 100), now: now)
    ledger.apply(mqtt(userTopic, connected1, 200), now: now)
    #expect(ledger.devices[id1]?.current(.power) == nil)
    // Kept, only not current: the diffuser may have come back as it was, or not.
    #expect(ledger.devices[id1]?.readings[.power] != nil)
    // A report in the same second as the reconnect counts.
    ledger.apply(report(id1, "S:0,L:1", 200), now: now)
    #expect(ledger.devices[id1]?.current(.power)?.value == 0)
    // A reconnect the listener heard while this phone wasn't listening counts too.
    ledger.apply(
      shared(
        "",
        #","connected":{"fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01","at":300}"#,
        310),
      now: now)
    #expect(ledger.devices[id1]?.connectedAt == 300)
    #expect(ledger.devices[id1]?.current(.power) == nil)
  }

  @Test func aListenerReportAfterASendClearsItAndOneBeforeDoesNot() {
    var ledger = knownAir2()
    ledger.apply(sent(id1, power: true, at: now), now: now)
    // The listener republishes an old report every 30 s: it confirms nothing.
    ledger.apply(shared(#""power":{"value":0,"at":\#(now - 30)}"#, "", now), now: now)
    #expect(ledger.devices[id1]?.pendingPower == true)
    ledger.apply(shared(#""power":{"value":1,"at":\#(now + 2)}"#, "", now + 3), now: now)
    #expect(ledger.devices[id1]?.pendingPower == nil)
  }

  @Test func theWillKeepsWhenTheListenerWasLastSeen() {
    var ledger = Ledger()
    ledger.apply(
      mqtt(listenerTopic, #"{"v":1,"running":true,"at":100,"started_at":50}"#, 100), now: now)
    #expect(ledger.listener == Heartbeat(running: true, at: 100))
    #expect(ledger.listenerSeenAt == 100)
    ledger.apply(mqtt(listenerTopic, #"{"v":1,"running":false}"#, 150), now: now)
    #expect(ledger.listener == Heartbeat(running: false, at: nil))
    #expect(ledger.listenerSeenAt == 100)
    #expect(ledger.log.isEmpty)
  }
}

@MainActor
@Suite struct ListenerEngine {
  @Test func aFreshPhoneLearnsEachDiffuserAndItsReadingsFromTheListener() async throws {
    let rig = Rig()
    let at = rig.clock.now
    let before = rig.model.log.count
    let record = #"""
      {"v":1,"by":"0a1b2c3d","updated_at":\#(at),"heard_at":\#(at - 5),"ping_at":\#(at - 20),
       "fields":{"power":{"value":1,"at":\#(at - 10)},"intensity":{"value":2,"at":\#(at - 10)},
                 "charging":{"value":2,"at":\#(at - 10)},"battery":{"value":200,"at":\#(at - 10)},
                 "rssi":{"value":-66,"at":\#(at - 10)}},
       "connected":{"fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01","at":\#(at - 3600)}}
      """#
    rig.radios.report(mqtt(state1Topic, record, at))
    let diffuser = try #require(rig.diffuser(id1))
    #expect(diffuser.name == nil && diffuser.kind == .air2 && diffuser.variant == "Black")
    #expect(diffuser.firmware == "2.30")
    #expect(diffuser.power?.value == true && diffuser.power?.via == .wifi)
    #expect(diffuser.level?.value == .medium)
    #expect(diffuser.pluggedIn?.value == true)
    #expect(diffuser.signal?.value == -66)
    #expect(diffuser.reachability == .answering(.wifi))
    #expect(diffuser.notKnownSince == nil)
    #expect(rig.model.log.count == before)
    // It is kept, so the phone knows it on its next launch with no seed.
    await rig.settle()
    let saved = try #require(try Store(folder: rig.folder).load())
    #expect(saved.devices.map(\.id) == [id1])
  }

  @Test func aReportFromBeforeAReconnectShowsAsNotKnownUntilANewerOne() async throws {
    let rig = Rig(file: twoDiffusers())
    #expect(rig.diffuser(id1)?.power?.value == true)
    let at = rig.clock.now
    let reconnect =
      #","connected":{"fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01","at":\#(at - 60)}"#
    rig.radios.report(shared("", reconnect, at))
    let unknown = try #require(rig.diffuser(id1))
    #expect(unknown.power == nil)
    #expect(unknown.notKnownSince == Date(timeIntervalSince1970: TimeInterval(at - 60)))
    rig.radios.report(report(id1, "S:0,L:3", at))
    let known = try #require(rig.diffuser(id1))
    #expect(known.power?.value == false && known.level?.value == .high)
    #expect(known.notKnownSince == nil)
  }

  @Test func theHeartbeatSaysWhetherTheListenerIsRunning() {
    let rig = Rig()
    #expect(rig.model.listener == .notSeen)
    let seen = rig.clock.now
    rig.radios.report(
      mqtt(listenerTopic, #"{"v":1,"running":true,"at":\#(seen),"started_at":1}"#, seen))
    #expect(rig.model.listener == .running)
    // A heartbeat is due every 30 s: with none for longer than 75 s, it isn't running. One
    // that can't be used changes nothing.
    rig.clock.now += 76
    rig.radios.report(
      mqtt(listenerTopic, #"{"v":2,"running":true,"at":\#(rig.clock.now)}"#, rig.clock.now))
    let lastSeen = Date(timeIntervalSince1970: TimeInterval(seen))
    #expect(rig.model.listener == .notRunning(lastSeen: lastSeen))
    // Its will, published by the broker when it drops off, keeps when it was last seen.
    rig.radios.report(mqtt(listenerTopic, #"{"v":1,"running":false}"#, rig.clock.now))
    #expect(rig.model.listener == .notRunning(lastSeen: lastSeen))
  }

  @Test func theListenersMessagesAreNeverLogged() {
    let rig = Rig(file: twoDiffusers())
    let before = rig.model.log.count
    for (topic, body) in [
      (state1Topic, #"{"v":1,"updated_at":\#(rig.clock.now),"fields":{}}"#),
      (state1Topic, #"{"v":1,"device_id":"TESTDEVICE01"}"#),
      (listenerTopic, #"{"v":1,"running":true,"at":\#(rig.clock.now)}"#),
      (listenerTopic, #"{"v":1}"#),
    ] {
      rig.radios.report(mqtt(topic, body, rig.clock.now))
    }
    #expect(rig.model.log.count == before)
  }
}
