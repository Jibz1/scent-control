// Ported from the tests in `packages/scent-core/src/listener.rs` and from
// `packages/scent-core/tests/listener_fixtures.rs`: the messages in `fixtures/scent-control/`
// are exactly what the listener publishes (apps/server tests render them), and they must
// decode here as they do on the Mac.

import Foundation
import Testing

@testable import ScentProtocol

private let now = 1_792_510_200

private func state(_ extra: String) -> Data {
  data(#"{"v":1,"updated_at":\#(now),"fields":{}\#(extra)}"#)
}

@Suite struct ListenerTests {
  @Test func anotherVersionIsUnusable() {
    #expect(
      decodeState(data(#"{"v":2,"updated_at":\#(now),"fields":{}}"#), now: now)
        == .failure(.version))
    #expect(
      decodeHeartbeat(data(#"{"v":2,"running":true,"at":\#(now)}"#), now: now) == .failure(.version)
    )
  }

  @Test func aTimeMoreThanADayAheadIsUnusable() {
    let ahead = now + 86_401
    for extra in [
      #","heard_at":\#(ahead)"#,
      #","ping_at":\#(ahead)"#,
      #","connected":{"fw":"","model":"","variant":"","manufacturer":"","at":\#(ahead)}"#,
    ] {
      #expect(decodeState(state(extra), now: now) == .failure(.ahead), "\(extra)")
    }
    let field = #"{"v":1,"updated_at":\#(now),"fields":{"power":{"value":1,"at":\#(ahead)}}}"#
    #expect(decodeState(data(field), now: now) == .failure(.ahead))
    let updated = #"{"v":1,"updated_at":\#(ahead),"fields":{}}"#
    #expect(decodeState(data(updated), now: now) == .failure(.ahead))
    #expect(
      decodeHeartbeat(data(#"{"v":1,"running":true,"at":\#(ahead)}"#), now: now) == .failure(.ahead)
    )
    let edge = #"{"v":1,"updated_at":\#(now + 86_400),"fields":{}}"#
    #expect((try? decodeState(data(edge), now: now).get()) != nil)
  }

  @Test func anEmptyOrMisshapenBodyIsInvalid() {
    for body in [
      Data(),
      data("not json"),
      data("[]"),
      data(#"{"updated_at":1,"fields":{}}"#),
      data(#"{"v":1,"fields":{}}"#),
      data(#"{"v":1,"updated_at":"now","fields":{}}"#),
      data(#"{"v":1,"updated_at":1}"#),
      state(#","heard_at":1.5"#),
      state(#","connected":{"fw":2.3,"model":"","variant":"","manufacturer":"","at":1}"#),
      data(#"{"v":1,"updated_at":1,"fields":{"power":{"value":"1","at":1}}}"#),
      data(#"{"v":1,"updated_at":1,"fields":{"power":1}}"#),
    ] {
      #expect(decodeState(body, now: now) == .failure(.invalid), "\(text(body))")
    }
    for body in ["", "{}", #"{"v":1}"#, #"{"v":1,"running":"yes"}"#] {
      #expect(decodeHeartbeat(data(body), now: now) == .failure(.invalid), "\(body)")
    }
  }

  @Test func fieldKeysItDoesNotKnowAreDropped() throws {
    let body = #"""
      {"v":1,"updated_at":\#(now),"fields":{"power":{"value":1,"at":\#(now)},"smell":{"value":3,"at":\#(now)},"odd":"x"}}
      """#
    let state = try decodeState(data(body), now: now).get()
    #expect(state.fields == ["power": Stamped(value: 1, at: now)])
  }

  @Test func theListenerRunsFor75SecondsAfterItsHeartbeat() {
    let beat = Heartbeat(running: true, at: now)
    #expect(listenerRunning(beat, now: now + 75))
    #expect(!listenerRunning(beat, now: now + 76))
    #expect(!listenerRunning(Heartbeat(running: false, at: nil), now: now))
    #expect(!listenerRunning(Heartbeat(running: true, at: nil), now: now))
    #expect(!listenerRunning(nil, now: now))
  }

  @Test func aReadingInTheSameSecondAsTheReconnectIsCurrent() {
    #expect(isCurrent(readingAt: now, connectedAt: now))
    #expect(!isCurrent(readingAt: now - 1, connectedAt: now))
    #expect(isCurrent(readingAt: now - 1000, connectedAt: nil))
  }

  /// Swift only: numbers as serde_json's `as_i64` takes them, which Foundation's reader
  /// would not tell apart.
  @Test func onlyWholeNumbersWithoutAFractionAreTimesOrValues() {
    #expect(decodeState(state(#","heard_at":1.0"#), now: now) == .failure(.invalid))
    #expect(decodeState(state(#","heard_at":1e3"#), now: now) == .failure(.invalid))
    #expect(decodeState(state(#","heard_at":true"#), now: now) == .failure(.invalid))
    #expect(decodeState(state(#","heard_at":null"#), now: now) == .failure(.invalid))
    #expect(
      decodeState(data(#"{"v":1.0,"updated_at":1,"fields":{}}"#), now: now) == .failure(.invalid))
    #expect(
      decodeState(data(#"{"v":true,"updated_at":1,"fields":{}}"#), now: now) == .failure(.invalid))
    let negative = #"{"v":1,"updated_at":1,"fields":{"rssi":{"value":-66,"at":1}}}"#
    #expect((try? decodeState(data(negative), now: now).get())?.fields["rssi"]?.value == -66)
  }
}

// MARK: The contract with the listener

private let contractNow = 1_792_510_230

private func contract(_ name: String) throws -> Data {
  let folder = repoRoot.appendingPathComponent("fixtures/scent-control")
  return try Data(contentsOf: folder.appendingPathComponent(name))
}

@Suite struct ListenerFixtureTests {
  @Test func theStateFixtureDecodes() throws {
    let at = 1_792_510_100
    let fields = Dictionary(
      uniqueKeysWithValues: [
        ("power", 1), ("intensity", 2), ("charging", 2), ("battery", 200), ("rssi", -66),
      ]
      .map { ($0.0, Stamped(value: $0.1, at: at)) })
    #expect(
      decodeState(try contract("state-v1.json"), now: contractNow)
        == .success(
          SharedState(
            updatedAt: 1_792_510_230, heardAt: 1_792_510_229, pingAt: 1_792_510_200, fields: fields,
            connected: ConnectedAt(
              fw: "2.30", model: "D", variant: "B", manufacturer: "01", at: 1_792_509_000))))
  }

  @Test func theHeartbeatAndWillFixturesDecode() throws {
    #expect(
      decodeHeartbeat(try contract("heartbeat-v1.json"), now: contractNow)
        == .success(Heartbeat(running: true, at: 1_792_510_230)))
    #expect(
      decodeHeartbeat(try contract("will-v1.json"), now: contractNow)
        == .success(Heartbeat(running: false, at: nil)))
  }
}
