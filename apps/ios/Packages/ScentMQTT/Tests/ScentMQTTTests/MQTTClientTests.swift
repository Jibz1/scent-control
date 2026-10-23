// The client's state machine against a broker played by the test: connect, CONNACK,
// subscribe, publish and PUBACK, keep-alive, reconnect and a refused login. No network,
// and time moves only when a test advances it.
import Foundation
import Testing

@testable import ScentMQTT

extension MQTTOptions {
  /// A broker for the tests: an example host and user, and a placeholder password.
  static var example: MQTTOptions {
    MQTTOptions(
      url: URL(string: "wss://broker.example.test/mqtt")!, username: "example-user",
      password: "x", subscriptions: ["devices/#", "app/v1/#"])
  }
}

/// A client on a fake network and clock, with its events in a queue.
struct Harness {
  let connector: FakeConnector
  let clock: TestClock
  let events: AsyncQueue<MQTTEvent>
  let client: MQTTClient

  init(_ options: MQTTOptions = .example) {
    let events = AsyncQueue<MQTTEvent>()
    let connector = FakeConnector()
    let clock = TestClock()
    self.events = events
    self.connector = connector
    self.clock = clock
    client = MQTTClient(
      options: options, events: { events.push($0) }, connector: connector, sleeper: clock)
  }

  /// Starts the client and accepts its login; returns the connection once the SUBSCRIBE
  /// has come.
  func connect(sessionPresent: Bool = false) async throws -> FakeTransport {
    await client.start()
    return try await accept(sessionPresent: sessionPresent)
  }

  /// Accepts the next connection's login; returns it once the SUBSCRIBE has come.
  func accept(sessionPresent: Bool = false) async throws -> FakeTransport {
    let transport = try await connector.connections.next("a connection")
    try await transport.accept(sessionPresent: sessionPresent)
    #expect(try await events.next("the connected event") == .connected)
    let subscribe = try await transport.nextPacket("the SUBSCRIBE")
    guard case .subscribe = subscribe else {
      Issue.record("Expected a SUBSCRIBE, got \(subscribe)")
      return transport
    }
    return transport
  }

  /// Waits out the 5 s before the next try, and returns the new connection.
  func retry() async throws -> FakeTransport {
    try await clock.waitForSleeper(.seconds(5))
    clock.advance(by: .seconds(5))
    return try await connector.connections.next("the next connection")
  }
}

/// Gives the client a moment of real time, for checks that something did not happen.
private func settle() async throws {
  try await Task.sleep(for: .milliseconds(50))
}

/// Waits, in real time, until `condition` holds.
private func eventually(
  _ what: String, within seconds: Double = 5, _ condition: () async -> Bool
) async throws {
  let clock = ContinuousClock()
  let deadline = clock.now + .seconds(seconds)
  while await !condition() {
    guard clock.now < deadline else { throw TimedOut(what: what) }
    try await Task.sleep(for: .milliseconds(1))
  }
}

private func publishPacket(_ packet: MQTTPacket) -> MQTTPublish? {
  if case .publish(let publish) = packet { return publish }
  return nil
}

@Suite("The client connecting and logging in")
struct ConnectingTests {
  @Test("sends CONNECT with the login, a clean session and a 30 s keep-alive")
  func connect() async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    #expect(harness.connector.attempts == [URL(string: "wss://broker.example.test/mqtt")!])
    let packet = try await transport.nextPacket("CONNECT")
    guard case .connect(let connect) = packet else {
      Issue.record("Expected a CONNECT, got \(packet)")
      return
    }
    #expect(connect.clientID.hasPrefix("scent-ios-"))
    #expect(connect.username == "example-user")
    #expect(connect.password == "x")
    #expect(connect.keepAliveSeconds == 30)
    #expect(connect.cleanSession)
    #expect(await !harness.client.isConnected)
  }

  @Test("subscribes to every filter at QoS 1 after the CONNACK, and says it is connected")
  func subscribes() async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    try await transport.accept()
    #expect(try await harness.events.next("connected") == .connected)
    #expect(await harness.client.isConnected)
    #expect(
      try await transport.nextPacket("SUBSCRIBE")
        == .subscribe(
          packetID: 1,
          subscriptions: [
            MQTTSubscription(filter: "devices/#", qos: .atLeastOnce),
            MQTTSubscription(filter: "app/v1/#", qos: .atLeastOnce),
          ]))
  }

  @Test("subscribes to nothing when there are no filters")
  func noFilters() async throws {
    var options = MQTTOptions.example
    options.subscriptions = []
    let harness = Harness(options)
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    try await transport.accept()
    #expect(try await harness.events.next("connected") == .connected)
    try await harness.client.publish(topic: "t", payload: Data())
    #expect(publishPacket(try await transport.nextPacket("the PUBLISH")) != nil)
  }

  @Test("starting twice opens one connection")
  func startTwice() async throws {
    let harness = Harness()
    await harness.client.start()
    await harness.client.start()
    _ = try await harness.connector.connections.next("a connection")
    try await settle()
    #expect(harness.connector.attempts.count == 1)
  }

  @Test(
    "a refused login is reported in the Mac's words and not tried again",
    arguments: [
      (4 as UInt8, "The broker refused the username or password."),
      (5, "The broker says this user is not authorized."),
    ])
  func loginRefused(code: UInt8, words: String) async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    _ = try await transport.nextPacket("CONNECT")
    try transport.deliver(.connAck(sessionPresent: false, returnCode: code))
    #expect(try await harness.events.next("the refusal") == .loginRefused(words))
    #expect(await !harness.client.isConnected)
    try await eventually("the connection to close") { transport.isClosed }

    // No retry, however long we wait.
    try await settle()
    #expect(harness.clock.waits.isEmpty)
    harness.clock.advance(by: .seconds(60))
    try await settle()
    #expect(harness.connector.attempts.count == 1)
    #expect(harness.events.pending.isEmpty)

    // Starting again tries again.
    _ = try await harness.connect()
    #expect(harness.connector.attempts.count == 2)
  }

  @Test(
    "a refusal that isn't about the login is retried after 5 s",
    arguments: [
      (1 as UInt8, "The broker refused the connection: it doesn't speak MQTT 3.1.1."),
      (2, "The broker refused the connection: it turned down the client ID."),
      (3, "The broker refused the connection: its MQTT service is unavailable."),
      (6, "The broker refused the connection with return code 6."),
    ])
  func otherRefusals(code: UInt8, words: String) async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    _ = try await transport.nextPacket("CONNECT")
    try transport.deliver(.connAck(sessionPresent: false, returnCode: code))
    #expect(try await harness.events.next("the refusal") == .disconnected(words))

    try await harness.clock.waitForSleeper(.seconds(5))
    harness.clock.advance(by: .milliseconds(4_999))
    try await settle()
    #expect(harness.connector.attempts.count == 1)
    harness.clock.advance(by: .milliseconds(1))
    _ = try await harness.accept()
    #expect(harness.connector.attempts.count == 2)
  }

  @Test("a connection that fails to open is reported and tried again after 5 s")
  func failedToOpen() async throws {
    let harness = Harness()
    harness.connector.plan(.fail("A server with the specified hostname could not be found."))
    await harness.client.start()
    #expect(
      try await harness.events.next("the failure")
        == .disconnected("A server with the specified hostname could not be found."))
    let transport = try await harness.retry()
    try await transport.accept()
    #expect(try await harness.events.next("connected") == .connected)
  }

  @Test("a connection that doesn't open within 5 s is given up and tried again")
  func openTimeout() async throws {
    let harness = Harness()
    harness.connector.plan(.hang)
    await harness.client.start()
    try await harness.clock.waitForSleeper(.seconds(5))
    harness.clock.advance(by: .seconds(5))
    #expect(
      try await harness.events.next("the timeout")
        == .disconnected("The broker didn't answer in time."))
    let transport = try await harness.retry()
    try await transport.accept()
    #expect(try await harness.events.next("connected") == .connected)
  }

  @Test("a CONNACK that doesn't come within 5 s is given up and tried again")
  func connAckTimeout() async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    _ = try await transport.nextPacket("CONNECT")
    try await harness.clock.waitForSleeper(.seconds(5))
    harness.clock.advance(by: .seconds(5))
    #expect(
      try await harness.events.next("the timeout")
        == .disconnected("The broker didn't answer in time."))
    #expect(transport.isClosed)
    _ = try await harness.retry()
  }

  @Test("the connect timer stops at the CONNACK")
  func connectTimerStops() async throws {
    let harness = Harness()
    _ = try await harness.connect()
    try await harness.clock.waitForSleeper(.seconds(30))
    // Only the keep-alive is waiting.
    #expect(harness.clock.waits == [.seconds(30)])
  }

  @Test("anything but a CONNACK first ends the connection")
  func notConnAckFirst() async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    _ = try await transport.nextPacket("CONNECT")
    try transport.deliver(.pingResp)
    #expect(
      try await harness.events.next("the failure")
        == .disconnected("The broker sent a PINGRESP before accepting the login."))
    _ = try await harness.retry()
  }

  @Test("options that can't work are reported and nothing is tried")
  func badOptions() async throws {
    var options: [(MQTTOptions, String)] = []
    for url in [
      "https://broker.example.test/mqtt", "mqtt://broker.example.test", "wss:///mqtt",
    ] {
      var bad = MQTTOptions.example
      bad.url = URL(string: url)!
      options.append((bad, "The broker address must be wss:// and a host name."))
    }
    var login = MQTTOptions.example
    login.url = URL(string: "wss://someone:x@broker.example.test/mqtt")!
    options.append((login, "The broker address can't have a username or password in it."))
    for filter in ["", "a/#/b", "a#", "a/b+", "a\u{0}"] {
      var bad = MQTTOptions.example
      bad.subscriptions = ["devices/#", filter]
      options.append((bad, "\"\(filter)\" isn't a topic filter the broker can take."))
    }
    var longID = MQTTOptions.example
    longID.clientID = String(repeating: "a", count: 70_000)
    options.append(
      (
        longID,
        "The client ID, username or password is too long for MQTT, "
          + "or the client ID or username has a null character in it."
      ))

    for (bad, words) in options {
      let harness = Harness(bad)
      await harness.client.start()
      #expect(try await harness.events.next("the complaint") == .disconnected(words))
      try await settle()
      #expect(harness.connector.attempts.isEmpty)
      #expect(harness.clock.waits.isEmpty)
    }
  }

  @Test("a ws:// address is taken, for a broker on this machine")
  func plainWebSocket() async throws {
    var options = MQTTOptions.example
    options.url = URL(string: "ws://127.0.0.1:9001/mqtt")!
    let harness = Harness(options)
    _ = try await harness.connect()
    #expect(harness.connector.attempts == [URL(string: "ws://127.0.0.1:9001/mqtt")!])
  }

  @Test("with no username and no password, the CONNECT carries no login")
  func anonymous() async throws {
    var options = MQTTOptions.example
    options.username = ""
    options.password = ""
    let harness = Harness(options)
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    guard case .connect(let connect) = try await transport.nextPacket("CONNECT") else {
      Issue.record("Expected a CONNECT")
      return
    }
    #expect(connect.username == nil)
    #expect(connect.password == nil)
  }
}

@Suite("The client publishing")
struct PublishingTests {
  @Test("publishes at QoS 1 with a fresh packet ID, retained when asked")
  func publishes() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    try await harness.client.publish(topic: "devices/TESTDEVICE01", payload: Data("{}".utf8))
    try await harness.client.publish(
      topic: "app/v1/schedule/x", payload: Data("[]".utf8), retain: true)
    #expect(
      try await transport.nextPacket("the first PUBLISH")
        == .publish(
          MQTTPublish(
            topic: "devices/TESTDEVICE01", payload: Data("{}".utf8), qos: .atLeastOnce,
            packetID: 2)))
    #expect(
      try await transport.nextPacket("the second PUBLISH")
        == .publish(
          MQTTPublish(
            topic: "app/v1/schedule/x", payload: Data("[]".utf8), qos: .atLeastOnce,
            retain: true, packetID: 3)))
    try transport.deliver(.pubAck(packetID: 2), .pubAck(packetID: 3))
  }

  @Test("refuses while not connected")
  func notConnected() async throws {
    let harness = Harness()
    await #expect(throws: MQTTError.notConnected) {
      try await harness.client.publish(topic: "t", payload: Data())
    }
    // Opening, before the CONNACK.
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    _ = try await transport.nextPacket("CONNECT")
    await #expect(throws: MQTTError.notConnected) {
      try await harness.client.publish(topic: "t", payload: Data())
    }
    // After the connection drops.
    try transport.deliver(.connAck(sessionPresent: false, returnCode: 0))
    #expect(try await harness.events.next("connected") == .connected)
    transport.fail("The network connection was lost.")
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The network connection was lost."))
    await #expect(throws: MQTTError.notConnected) {
      try await harness.client.publish(topic: "t", payload: Data())
    }
  }

  @Test("refuses a topic that can't be published to", arguments: ["", "a/+", "#", "a/#", "a\u{0}"])
  func badTopics(topic: String) async throws {
    let harness = Harness()
    _ = try await harness.connect()
    await #expect(
      throws: MQTTError(message: "\"\(topic)\" isn't a topic a message can be published to.")
    ) {
      try await harness.client.publish(topic: topic, payload: Data())
    }
  }

  @Test("refuses a packet over 256 KiB, and takes one at the limit")
  func sizeLimit() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    // Topic "t": 1 fixed header byte, 3 length bytes, 2 + 1 topic, 2 packet ID.
    let largest = 256 * 1024 - 9
    await #expect(throws: MQTTError.tooLarge) {
      try await harness.client.publish(topic: "t", payload: Data(count: largest + 1))
    }
    try await harness.client.publish(topic: "t", payload: Data(count: largest))
    let sent = try await transport.sent.next("the PUBLISH")
    #expect(sent.count == 256 * 1024)
  }

  @Test("takes 16 waiting messages behind the one being written, then is too busy")
  func tooBusy() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    transport.closeGate()
    try await harness.client.publish(topic: "t/0", payload: Data())
    try await eventually("the first message to be held in its send") { transport.sendsWaiting == 1 }
    for index in 1..<17 {
      try await harness.client.publish(topic: "t/\(index)", payload: Data())
    }
    await #expect(throws: MQTTError.tooBusy) {
      try await harness.client.publish(topic: "t/17", payload: Data())
    }
    transport.openGate()
    for index in 0..<17 {
      let publish = publishPacket(try await transport.nextPacket("PUBLISH \(index)"))
      #expect(publish?.topic == "t/\(index)")
      #expect(publish?.packetID == UInt16(index + 2))
    }
    // With the queue drained, it takes messages again.
    try await harness.client.publish(topic: "t/18", payload: Data())
  }

  @Test("keeps at most 100 messages in flight, and sends the next as a PUBACK frees a place")
  func inFlightLimit() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    for index in 0..<100 {
      try await harness.client.publish(topic: "t/\(index)", payload: Data())
      _ = try await transport.nextPacket("PUBLISH \(index)")
    }
    try await harness.client.publish(topic: "t/100", payload: Data())
    try await settle()
    #expect(transport.sent.pending.isEmpty)

    try transport.deliver(.pubAck(packetID: 2))
    let next = publishPacket(try await transport.nextPacket("PUBLISH 100"))
    #expect(next?.topic == "t/100")
    #expect(next?.packetID == 102)
  }

  @Test("ignores a PUBACK for a packet ID that isn't in flight")
  func strayPubAck() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    try transport.deliver(.pubAck(packetID: 999))
    try await harness.client.publish(topic: "t", payload: Data())
    #expect(publishPacket(try await transport.nextPacket("the PUBLISH")) != nil)
    #expect(await harness.client.isConnected)
  }
}

@Suite("The client receiving")
struct ReceivingTests {
  @Test("reports a QoS 1 message and acknowledges it")
  func qos1() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    try transport.deliver(
      .publish(
        MQTTPublish(
          topic: "devices/TESTDEVICE01", payload: Data("{\"a\":1}".utf8), qos: .atLeastOnce,
          retain: true, packetID: 42)))
    #expect(
      try await harness.events.next("the message")
        == .message(topic: "devices/TESTDEVICE01", payload: Data("{\"a\":1}".utf8), retained: true))
    #expect(try await transport.nextPacket("the PUBACK") == .pubAck(packetID: 42))
  }

  @Test("reports a QoS 0 message without acknowledging it")
  func qos0() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    try transport.deliver(.publish(MQTTPublish(topic: "a", payload: Data("1".utf8))))
    #expect(
      try await harness.events.next("the message")
        == .message(topic: "a", payload: Data("1".utf8), retained: false))
    // The next thing sent is our own message, not a PUBACK.
    try await harness.client.publish(topic: "b", payload: Data())
    #expect(publishPacket(try await transport.nextPacket("the PUBLISH"))?.topic == "b")
  }

  @Test("reads packets split across WebSocket messages and packed into one")
  func framing() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    var stream = Data()
    for index in 0..<3 {
      stream.append(
        try MQTTPacket.publish(
          MQTTPublish(topic: "t/\(index)", payload: Data(repeating: UInt8(index), count: 300))
        )
        .encode())
    }
    // The first message ends inside the first packet's length; the second holds the rest of
    // it and most of the second packet; the third has the end of that and the whole third.
    let cuts = [0, 2, 450, stream.count]
    for (from, to) in zip(cuts, cuts.dropFirst()) {
      transport.deliver(bytes: Array(stream[from..<to]))
    }
    for index in 0..<3 {
      #expect(
        try await harness.events.next("message \(index)")
          == .message(
            topic: "t/\(index)", payload: Data(repeating: UInt8(index), count: 300),
            retained: false))
    }
  }

  @Test("drops the connection on a QoS 2 message, which it never asked for")
  func qos2() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    try transport.deliver(
      .publish(MQTTPublish(topic: "a", payload: Data(), qos: .exactlyOnce, packetID: 1)))
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The broker sent a QoS 2 message, which this client never asks for."))
    #expect(transport.isClosed)
  }

  @Test("drops the connection on a packet type it doesn't speak, and tries again")
  func unsupportedPacket() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    transport.deliver(bytes: [0x62, 0x02, 0x00, 0x01])  // PUBREL
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The broker sent a packet of type 6, which this client doesn't speak."))
    _ = try await harness.retry()
  }

  @Test("drops the connection on a malformed packet")
  func malformedPacket() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    transport.deliver(bytes: [0xD0, 0x01, 0x00])  // PINGRESP with a body
    #expect(
      try await harness.events.next("the drop")
        == .disconnected(
          "The broker sent a malformed packet. "
            + "A PINGRESP has the wrong length after its fixed header: 1, not 0."))
  }

  @Test("drops the connection on a packet over 256 KiB, before the rest of it arrives")
  func oversizedPacket() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    transport.deliver(bytes: [0x30] + (try MQTTPacket.remainingLength(256 * 1024 + 1)))
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The broker sent a packet of 262145 bytes, over the 256 KiB limit."))
  }

  @Test("drops the connection on a packet only a client sends")
  func unexpectedPacket() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    try transport.deliver(.pingReq)
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The broker sent an unexpected PINGREQ."))
  }
}

@Suite("The client keeping the connection alive")
struct KeepAliveTests {
  @Test("pings every 30 s, and a ping unanswered by the next one ends the connection")
  func pings() async throws {
    let harness = Harness()
    let transport = try await harness.connect()

    try await harness.clock.waitForSleeper(.seconds(30))
    harness.clock.advance(by: .seconds(30))
    #expect(try await transport.nextPacket("the first PINGREQ") == .pingReq)
    // The answer, then a message, so that once the message is reported the answer has
    // been read.
    try transport.deliver(.pingResp, .publish(MQTTPublish(topic: "a", payload: Data())))
    #expect(
      try await harness.events.next("the message")
        == .message(topic: "a", payload: Data(), retained: false))

    try await harness.clock.waitForSleeper(.seconds(30))
    harness.clock.advance(by: .seconds(30))
    #expect(try await transport.nextPacket("the second PINGREQ") == .pingReq)
    #expect(await harness.client.isConnected)

    // No answer this time.
    try await harness.clock.waitForSleeper(.seconds(30))
    harness.clock.advance(by: .seconds(30))
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The broker stopped answering pings."))
    #expect(await !harness.client.isConnected)
    #expect(transport.isClosed)

    _ = try await harness.retry()
  }

  @Test("the keep-alive goes in the CONNECT rounded up to whole seconds, and zero turns it off")
  func keepAliveSeconds() {
    #expect(MQTTClient.keepAliveSeconds(.seconds(30)) == 30)
    #expect(MQTTClient.keepAliveSeconds(.milliseconds(1_500)) == 2)
    #expect(MQTTClient.keepAliveSeconds(.zero) == 0)
    #expect(MQTTClient.keepAliveSeconds(.seconds(100_000)) == 65_535)
  }

  @Test("with keep-alive off, nothing waits once connected")
  func noKeepAlive() async throws {
    var options = MQTTOptions.example
    options.keepAlive = .zero
    let harness = Harness(options)
    _ = try await harness.connect()
    try await settle()
    #expect(harness.clock.waits.isEmpty)
  }
}

@Suite("The client reconnecting")
struct ReconnectingTests {
  @Test("a dropped connection is reported, retried after 5 s, and subscribed again")
  func reconnects() async throws {
    let harness = Harness()
    let first = try await harness.connect()
    first.fail("The network connection was lost.")
    #expect(
      try await harness.events.next("the drop")
        == .disconnected("The network connection was lost."))
    #expect(await !harness.client.isConnected)

    try await harness.clock.waitForSleeper(.seconds(5))
    harness.clock.advance(by: .milliseconds(4_999))
    try await settle()
    #expect(harness.connector.attempts.count == 1)
    harness.clock.advance(by: .milliseconds(1))
    // `accept` checks the new CONNECT, the connected event and the SUBSCRIBE.
    _ = try await harness.accept()
    #expect(await harness.client.isConnected)
  }

  @Test("a message in flight is dropped on a new session, as rumqttc drops it")
  func dropsInFlightOnACleanSession() async throws {
    let harness = Harness()
    let first = try await harness.connect()
    try await harness.client.publish(topic: "old", payload: Data())
    _ = try await first.nextPacket("the PUBLISH")
    first.fail("The network connection was lost.")
    _ = try await harness.events.next("the drop")

    let second = try await harness.retry()
    try await second.accept(sessionPresent: false)
    _ = try await harness.events.next("connected")
    #expect(try await second.nextPacket("the SUBSCRIBE").name == "SUBSCRIBE")
    try await harness.client.publish(topic: "new", payload: Data())
    #expect(publishPacket(try await second.nextPacket("the new PUBLISH"))?.topic == "new")
  }

  @Test("a message in flight is sent again, as a duplicate, when the broker kept the session")
  func resendsWhenTheSessionWasKept() async throws {
    let harness = Harness()
    let first = try await harness.connect()
    try await harness.client.publish(topic: "acked", payload: Data())
    try await harness.client.publish(topic: "unacked", payload: Data("1".utf8))
    _ = try await first.nextPacket("the first PUBLISH")
    _ = try await first.nextPacket("the second PUBLISH")
    try first.deliver(.pubAck(packetID: 2))
    // Once the message after the PUBACK is reported, the PUBACK has been read.
    try first.deliver(.publish(MQTTPublish(topic: "marker", payload: Data())))
    _ = try await harness.events.next("the marker")
    first.fail("The network connection was lost.")
    _ = try await harness.events.next("the drop")

    let second = try await harness.retry()
    try await second.accept(sessionPresent: true)
    _ = try await harness.events.next("connected")
    #expect(try await second.nextPacket("the SUBSCRIBE").name == "SUBSCRIBE")
    #expect(
      try await second.nextPacket("the resent PUBLISH")
        == .publish(
          MQTTPublish(
            topic: "unacked", payload: Data("1".utf8), qos: .atLeastOnce, duplicate: true,
            packetID: 3)))
    try await harness.client.publish(topic: "new", payload: Data())
    let new = publishPacket(try await second.nextPacket("the new PUBLISH"))
    #expect(new?.topic == "new")
    #expect(new?.duplicate == false)
    #expect(new?.packetID != 3)
  }

  @Test("a send that fails drops the connection")
  func sendFails() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    transport.failSends()
    try await harness.client.publish(topic: "t", payload: Data())
    #expect(try await harness.events.next("the drop") == .disconnected("The send failed."))
    #expect(transport.isClosed)
  }
}

@Suite("The client stopping")
struct StoppingTests {
  @Test("sends DISCONNECT, closes the connection, reports nothing and stops trying")
  func stops() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    await harness.client.stop()
    #expect(await !harness.client.isConnected)
    #expect(try await transport.nextPacket("the DISCONNECT") == .disconnect)
    try await eventually("the connection to close") { transport.isClosed }
    await #expect(throws: MQTTError.notConnected) {
      try await harness.client.publish(topic: "t", payload: Data())
    }
    harness.clock.advance(by: .seconds(60))
    try await settle()
    #expect(harness.events.pending.isEmpty)
    #expect(harness.connector.attempts.count == 1)
  }

  @Test("closes the connection anyway when the DISCONNECT can't go out within a second")
  func stopsWhenStuck() async throws {
    let harness = Harness()
    let transport = try await harness.connect()
    transport.closeGate()
    await harness.client.stop()
    try await harness.clock.waitForSleeper(.seconds(1))
    #expect(!transport.isClosed)
    harness.clock.advance(by: .seconds(1))
    try await eventually("the connection to close") { transport.isClosed }
  }

  @Test("stopping while waiting to retry stops the retries; starting again connects at once")
  func stopWhileWaiting() async throws {
    let harness = Harness()
    harness.connector.plan(.fail("The Internet connection appears to be offline."))
    await harness.client.start()
    _ = try await harness.events.next("the failure")
    try await harness.clock.waitForSleeper(.seconds(5))
    await harness.client.stop()
    #expect(harness.clock.waits.isEmpty)
    harness.clock.advance(by: .seconds(10))
    try await settle()
    #expect(harness.connector.attempts.count == 1)

    _ = try await harness.connect()
    #expect(harness.connector.attempts.count == 2)
  }

  @Test("stopping while connecting gives up the connection without a word")
  func stopWhileConnecting() async throws {
    let harness = Harness()
    harness.connector.plan(.hang)
    await harness.client.start()
    try await eventually("the connection attempt") { harness.connector.attempts.count == 1 }
    await harness.client.stop()
    harness.clock.advance(by: .seconds(10))
    try await settle()
    #expect(harness.events.pending.isEmpty)
    #expect(harness.connector.attempts.count == 1)
    #expect(harness.clock.waits.isEmpty)
  }

  @Test("stopping before the CONNACK closes the connection without a DISCONNECT")
  func stopBeforeConnAck() async throws {
    let harness = Harness()
    await harness.client.start()
    let transport = try await harness.connector.connections.next("a connection")
    _ = try await transport.nextPacket("CONNECT")
    await harness.client.stop()
    #expect(transport.isClosed)
    try await settle()
    #expect(transport.sent.pending.isEmpty)
    // A CONNACK arriving late changes nothing.
    try? transport.deliver(.connAck(sessionPresent: false, returnCode: 0))
    try await settle()
    #expect(harness.events.pending.isEmpty)
    #expect(await !harness.client.isConnected)
  }
}
