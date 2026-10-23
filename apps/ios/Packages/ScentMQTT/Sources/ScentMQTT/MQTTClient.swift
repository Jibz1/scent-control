// The connection to the broker: connect, log in, subscribe, publish, keep alive, and
// reconnect, as the Mac app's broker worker does (`apps/desktop/src/mqtt.rs`, which uses
// rumqttc's defaults where it sets nothing).
import Foundation

/// One connection to an MQTT 3.1.1 broker over a WebSocket, kept up until `stop()`.
///
/// After `start()` the client connects, logs in with a clean session, subscribes to every
/// filter in the options at QoS 1, and reports `.connected`. It pings the broker on the
/// keep-alive, and a ping still unanswered at the next one means the connection is dead.
/// When the connection fails or drops it reports `.disconnected` with the reason and tries
/// again after `retryAfter`, subscribing again on the new CONNACK. A refused login is
/// reported as `.loginRefused` and the client stops trying until `start()` is called
/// again, as trying the same password again only repeats the refusal.
///
/// Every message is published at QoS 1 and stays in flight until the broker's PUBACK. As
/// rumqttc does, a message in flight or still waiting when the connection drops is sent
/// again (in flight ones marked as duplicates) only if the broker says it kept the session.
/// The client always asks for a clean session, so in practice those messages are dropped,
/// and nothing goes out minutes later when the person no longer expects it.
public actor MQTTClient {
  /// How many messages may wait to go out before `publish` says it is too busy: the
  /// Mac's queue.
  static let queueLimit = 16
  /// How many messages may be out and waiting for their PUBACK at once: rumqttc's default.
  /// More wait in the queue.
  static let inFlightLimit = 100
  /// The largest packet the client writes, and the largest remaining length it reads:
  /// 256 KiB, as on the Mac. A full schedule is about 13 KB.
  static let maxPacketSize = 256 * 1024
  /// How long a connection may take, from opening the WebSocket to the CONNACK: rumqttc's
  /// default.
  static let defaultConnectTimeout: Duration = .seconds(5)
  /// How long `stop()` waits for the DISCONNECT to go out before it closes the connection
  /// anyway.
  static let goodbyeTimeout: Duration = .seconds(1)

  private let options: MQTTOptions
  private let events: @Sendable (MQTTEvent) -> Void
  private let connector: any MQTTConnector
  private let sleeper: any MQTTSleeper
  private let connectTimeout: Duration

  /// True from `start()` until `stop()` or a refused login.
  private var running = false
  /// Changes on every `start()` and `stop()`, so that a loop from before knows to end.
  private var runID = 0
  /// Connects, waits out the connection, and tries again.
  private var loop: Task<Void, Never>?
  private var attemptCount = 0
  /// The connection being opened or in use, if any.
  private var attempt: Attempt?
  /// The CONNECT, made once the options have been checked by `start()`.
  private var connectPacket = Data()

  /// Messages waiting to go out, oldest first.
  private var queued: [Message] = []
  /// Messages sent and waiting for their PUBACK, in the order they went out.
  private var inFlight: [InFlight] = []
  /// Packet IDs of messages in flight to send again, marked as duplicates, after a CONNACK
  /// that says the broker kept the session.
  private var resends: [UInt16] = []
  private var lastPacketID: UInt16 = 0

  /// A client for the broker in `options`. It reports through `events`, in order, from its
  /// own executor. Nothing happens until `start()`.
  public init(options: MQTTOptions, events: @escaping @Sendable (MQTTEvent) -> Void) {
    self.init(
      options: options, events: events, connector: WebSocketConnector(),
      sleeper: TaskSleeper())
  }

  /// A client on a given network and clock, for the tests.
  init(
    options: MQTTOptions, events: @escaping @Sendable (MQTTEvent) -> Void,
    connector: any MQTTConnector, sleeper: any MQTTSleeper,
    connectTimeout: Duration = MQTTClient.defaultConnectTimeout
  ) {
    self.options = options
    self.events = events
    self.connector = connector
    self.sleeper = sleeper
    self.connectTimeout = connectTimeout
  }

  /// Whether the broker has accepted the login and the connection is up: true from the
  /// CONNACK until the connection drops or `stop()`.
  public var isConnected: Bool {
    attempt?.phase == .connected
  }

  /// Connects, and keeps reconnecting until `stop()` or a refused login. Does nothing while
  /// already running. Options that can't work are reported as `.disconnected` and nothing
  /// is tried.
  public func start() {
    guard !running else { return }
    do {
      connectPacket = try MQTTClient.connectPacket(for: options)
    } catch {
      events(.disconnected(error.message))
      return
    }
    running = true
    runID &+= 1
    let id = runID
    loop = Task { await self.run(id) }
  }

  /// Sends DISCONNECT and closes the connection, and stops trying. Messages still waiting
  /// or in flight are dropped. Reports nothing.
  public func stop() {
    running = false
    runID &+= 1
    loop?.cancel()
    loop = nil
    queued.removeAll()
    inFlight.removeAll()
    resends.removeAll()
    guard let attempt else { return }
    let goodbye = attempt.phase == .connected ? attempt.transport : nil
    end(attempt, .stopped, closingTransport: goodbye == nil)
    if let goodbye {
      let sleeper = sleeper
      Task { await MQTTClient.sayGoodbye(over: goodbye, sleeper: sleeper) }
    }
  }

  /// Queues a message for the broker at QoS 1 and returns without waiting for the
  /// network. Refuses while the broker isn't connected, so that nothing waits to go out
  /// later; when 16 messages are already waiting; for a topic MQTT can't publish to (empty,
  /// or with `+` or `#` in it); and for a packet over 256 KiB.
  public func publish(topic: String, payload: Data, retain: Bool = false) throws(MQTTError) {
    guard MQTTClient.isPublishable(topic) else {
      throw MQTTError(message: "\"\(topic)\" isn't a topic a message can be published to.")
    }
    let remainingLength = 2 + topic.utf8.count + 2 + payload.count
    guard MQTTPacket.size(remainingLength: remainingLength) <= MQTTClient.maxPacketSize else {
      throw .tooLarge
    }
    guard let attempt, attempt.phase == .connected else { throw .notConnected }
    guard queued.count < MQTTClient.queueLimit else { throw .tooBusy }
    queued.append(Message(topic: topic, payload: payload, retain: retain))
    wakeWriter(attempt)
  }

  // MARK: - Connecting and reconnecting

  private func run(_ id: Int) async {
    while running && runID == id {
      let outcome = await connectOnce()
      guard running && runID == id else { return }
      switch outcome {
      case .loginRefused(let reason):
        running = false
        loop = nil
        events(.loginRefused(reason))
        return
      case .lost(let reason):
        events(.disconnected(reason))
        do {
          try await sleeper.sleep(for: options.retryAfter)
        } catch {
          return
        }
      case .stopped:
        return
      }
    }
  }

  /// Opens a connection and waits until it ends, however it ends.
  private func connectOnce() async -> Outcome {
    attemptCount &+= 1
    let attempt = Attempt(id: attemptCount)
    self.attempt = attempt
    let id = attempt.id
    let connector = connector
    let sleeper = sleeper
    let url = options.url
    let timeout = connectTimeout
    attempt.connectTimer = Task {
      do {
        try await sleeper.sleep(for: timeout)
      } catch {
        return
      }
      self.connectTimedOut(id)
    }
    attempt.opening = Task {
      do {
        let transport = try await connector.connect(to: url)
        self.opened(transport, id)
      } catch {
        self.lose(id, error)
      }
    }
    return await withCheckedContinuation { attempt.endWaiter = $0 }
  }

  private func opened(_ transport: any MQTTTransport, _ id: Int) {
    guard let attempt = current(id), attempt.phase == .opening else {
      transport.close()
      return
    }
    attempt.transport = transport
    attempt.phase = .awaitingConnAck
    attempt.control.append(connectPacket)
    attempt.reader = Task { await self.readLoop(transport, id) }
    attempt.writer = Task { await self.writeLoop(transport, id) }
  }

  private func connectTimedOut(_ id: Int) {
    guard let attempt = current(id),
      attempt.phase == .opening || attempt.phase == .awaitingConnAck
    else { return }
    end(attempt, .lost("The broker didn't answer in time."))
  }

  /// The broker accepted the login.
  private func connected(_ attempt: Attempt, sessionPresent: Bool) {
    attempt.phase = .connected
    attempt.connectTimer?.cancel()
    attempt.connectTimer = nil
    if sessionPresent {
      resends = inFlight.map(\.packetID)
    } else {
      // A new session: the broker has forgotten the old one, and rumqttc drops what was
      // waiting from it.
      queued.removeAll()
      inFlight.removeAll()
      resends.removeAll()
    }
    if !options.subscriptions.isEmpty {
      let subscriptions = options.subscriptions.map {
        MQTTSubscription(filter: $0, qos: .atLeastOnce)
      }
      queueControl(.subscribe(packetID: nextPacketID(), subscriptions: subscriptions), on: attempt)
    }
    if options.keepAlive > .zero {
      let id = attempt.id
      let interval = options.keepAlive
      let sleeper = sleeper
      attempt.keepAlive = Task {
        while true {
          do {
            try await sleeper.sleep(for: interval)
          } catch {
            return
          }
          guard self.keepAliveTick(id) else { return }
        }
      }
    }
    events(.connected)
    wakeWriter(attempt)
  }

  /// Pings the broker, or ends the connection when the last ping had no answer.
  private func keepAliveTick(_ id: Int) -> Bool {
    guard let attempt = current(id), attempt.phase == .connected else { return false }
    guard !attempt.awaitingPingResponse else {
      end(attempt, .lost("The broker stopped answering pings."))
      return false
    }
    attempt.awaitingPingResponse = true
    queueControl(.pingReq, on: attempt)
    return true
  }

  /// Ends a connection: stops its timers, wakes its writer, closes it unless `stop()` is
  /// saying goodbye on it, and hands the outcome to the loop. Only the first call counts.
  private func end(_ attempt: Attempt, _ outcome: Outcome, closingTransport: Bool = true) {
    guard attempt.phase != .ended else { return }
    attempt.phase = .ended
    for task in [attempt.opening, attempt.connectTimer, attempt.keepAlive, attempt.reader] {
      task?.cancel()
    }
    if let writer = attempt.writerWaiting {
      attempt.writerWaiting = nil
      writer.resume(returning: nil)
    }
    if closingTransport {
      attempt.transport?.close()
    }
    if self.attempt === attempt {
      self.attempt = nil
    }
    if let waiter = attempt.endWaiter {
      attempt.endWaiter = nil
      waiter.resume(returning: outcome)
    }
  }

  /// A failure on the connection with this ID, if it is still the current one.
  private func lose(_ id: Int, _ error: any Error) {
    guard let attempt = current(id) else { return }
    end(attempt, .lost(error.localizedDescription))
  }

  private func current(_ id: Int) -> Attempt? {
    guard let attempt, attempt.id == id else { return nil }
    return attempt
  }

  private static func sayGoodbye(over transport: any MQTTTransport, sleeper: any MQTTSleeper) async
  {
    let disconnect = Data([0xE0, 0x00])
    await withTaskGroup(of: Void.self) { group in
      group.addTask { try? await transport.send(disconnect) }
      group.addTask { try? await sleeper.sleep(for: goodbyeTimeout) }
      _ = await group.next()
      transport.close()
      group.cancelAll()
    }
  }

  // MARK: - Reading

  private func readLoop(_ transport: any MQTTTransport, _ id: Int) async {
    while true {
      let data: Data
      do {
        data = try await transport.receive()
      } catch {
        lose(id, error)
        return
      }
      guard received(data, on: id) else { return }
    }
  }

  /// Takes in a WebSocket message's bytes. False once the connection has ended.
  private func received(_ data: Data, on id: Int) -> Bool {
    guard let attempt = current(id) else { return false }
    attempt.decoder.append(data)
    while attempt.phase != .ended {
      let packet: MQTTPacket?
      do {
        packet = try attempt.decoder.next()
      } catch {
        end(attempt, .lost(MQTTClient.words(for: error)))
        return false
      }
      guard let packet else { return true }
      handle(packet, on: attempt)
    }
    return false
  }

  private func handle(_ packet: MQTTPacket, on attempt: Attempt) {
    switch attempt.phase {
    case .awaitingConnAck:
      guard case .connAck(let sessionPresent, let returnCode) = packet else {
        end(attempt, .lost("The broker sent a \(packet.name) before accepting the login."))
        return
      }
      switch returnCode {
      case 0:
        connected(attempt, sessionPresent: sessionPresent)
      case 4:
        end(attempt, .loginRefused("The broker refused the username or password."))
      case 5:
        end(attempt, .loginRefused("The broker says this user is not authorized."))
      default:
        end(attempt, .lost(MQTTClient.refusal(returnCode)))
      }
    case .connected:
      switch packet {
      case .publish(let publish):
        deliver(publish, on: attempt)
      case .pubAck(let packetID):
        acknowledged(packetID, on: attempt)
      case .subAck:
        // As on the Mac, the granted QoS isn't checked.
        break
      case .pingResp:
        attempt.awaitingPingResponse = false
      default:
        end(attempt, .lost("The broker sent an unexpected \(packet.name)."))
      }
    case .opening, .ended:
      break
    }
  }

  private func deliver(_ publish: MQTTPublish, on attempt: Attempt) {
    switch publish.qos {
    case .atMostOnce:
      break
    case .atLeastOnce:
      if let packetID = publish.packetID {
        queueControl(.pubAck(packetID: packetID), on: attempt)
      }
    case .exactlyOnce:
      // Every subscription is at QoS 1, so the broker may not send QoS 2.
      end(attempt, .lost("The broker sent a QoS 2 message, which this client never asks for."))
      return
    }
    events(.message(topic: publish.topic, payload: publish.payload, retained: publish.retain))
  }

  /// The broker has the message with this packet ID. An ID that isn't in flight (one from
  /// a connection before) is ignored.
  private func acknowledged(_ packetID: UInt16, on attempt: Attempt) {
    guard let index = inFlight.firstIndex(where: { $0.packetID == packetID }) else { return }
    inFlight.remove(at: index)
    resends.removeAll { $0 == packetID }
    wakeWriter(attempt)
  }

  // MARK: - Writing

  /// One writer per connection sends everything, in order.
  private func writeLoop(_ transport: any MQTTTransport, _ id: Int) async {
    while let bytes = await nextOutgoing(id) {
      do {
        try await transport.send(bytes)
      } catch {
        lose(id, error)
        return
      }
    }
  }

  /// The next bytes to send, waiting until there are some. Nil once the connection ends.
  private func nextOutgoing(_ id: Int) async -> Data? {
    guard let attempt = current(id) else { return nil }
    if let bytes = takeOutgoing(attempt) { return bytes }
    return await withCheckedContinuation { attempt.writerWaiting = $0 }
  }

  /// Hands the waiting writer the next bytes, if it is waiting and there are some.
  private func wakeWriter(_ attempt: Attempt) {
    guard let writer = attempt.writerWaiting, let bytes = takeOutgoing(attempt) else { return }
    attempt.writerWaiting = nil
    writer.resume(returning: bytes)
  }

  /// What goes out next: the connection's own packets first (CONNECT, SUBSCRIBE, PUBACK,
  /// PINGREQ), then messages to send again, then waiting messages while fewer than 100 are
  /// in flight. A waiting message gets its packet ID here, as it goes out.
  private func takeOutgoing(_ attempt: Attempt) -> Data? {
    if !attempt.control.isEmpty {
      return attempt.control.removeFirst()
    }
    guard attempt.phase == .connected else { return nil }
    while !resends.isEmpty {
      let packetID = resends.removeFirst()
      if let entry = inFlight.first(where: { $0.packetID == packetID }),
        let bytes = try? MQTTPacket.publish(entry.message.publish(packetID, duplicate: true))
          .encode()
      {
        return bytes
      }
    }
    while inFlight.count < MQTTClient.inFlightLimit, !queued.isEmpty {
      let message = queued.removeFirst()
      let packetID = nextPacketID()
      // `publish` checked the topic and the size, so this can't fail.
      guard
        let bytes = try? MQTTPacket.publish(message.publish(packetID, duplicate: false)).encode()
      else { continue }
      inFlight.append(InFlight(packetID: packetID, message: message))
      return bytes
    }
    return nil
  }

  private func queueControl(_ packet: MQTTPacket, on attempt: Attempt) {
    // The packets queued here (SUBSCRIBE of checked filters, PUBACK, PINGREQ) always encode.
    guard let bytes = try? packet.encode() else { return }
    attempt.control.append(bytes)
    wakeWriter(attempt)
  }

  /// The next packet ID, from 1 to 65,535 and round again, skipping those in flight.
  private func nextPacketID() -> UInt16 {
    repeat {
      lastPacketID = lastPacketID == .max ? 1 : lastPacketID + 1
    } while inFlight.contains { $0.packetID == lastPacketID }
    return lastPacketID
  }

  // MARK: - Checks and words

  /// The CONNECT for these options, or why the options can't work.
  static func connectPacket(for options: MQTTOptions) throws(MQTTError) -> Data {
    let url = options.url
    guard url.user(percentEncoded: false) == nil, url.password(percentEncoded: false) == nil
    else {
      throw MQTTError(message: "The broker address can't have a username or password in it.")
    }
    guard let scheme = url.scheme?.lowercased(), scheme == "wss" || scheme == "ws",
      let host = url.host(percentEncoded: false), !host.isEmpty
    else {
      throw MQTTError(message: "The broker address must be wss:// and a host name.")
    }
    for filter in options.subscriptions where !isFilter(filter) {
      throw MQTTError(message: "\"\(filter)\" isn't a topic filter the broker can take.")
    }
    let subscribe = MQTTPacket.subscribe(
      packetID: 1,
      subscriptions: options.subscriptions.map { MQTTSubscription(filter: $0, qos: .atLeastOnce) })
    if !options.subscriptions.isEmpty, (try? subscribe.encode()) == nil {
      throw MQTTError(message: "There are too many topic filters to subscribe to in one packet.")
    }
    let hasLogin = !options.username.isEmpty || !options.password.isEmpty
    let connect = MQTTConnect(
      clientID: options.clientID,
      username: hasLogin ? options.username : nil,
      password: hasLogin ? options.password : nil,
      keepAliveSeconds: keepAliveSeconds(options.keepAlive),
      cleanSession: true)
    do {
      return try MQTTPacket.connect(connect).encode()
    } catch {
      throw MQTTError(
        message: "The client ID, username or password is too long for MQTT, "
          + "or the client ID or username has a null character in it.")
    }
  }

  /// The keep-alive in whole seconds for the CONNECT, rounded up and at most 65,535.
  static func keepAliveSeconds(_ keepAlive: Duration) -> UInt16 {
    guard keepAlive > .zero else { return 0 }
    let (seconds, attoseconds) = keepAlive.components
    return UInt16(clamping: seconds + (attoseconds > 0 ? 1 : 0))
  }

  /// Whether a message can be published to `topic`: not empty, no wildcard, no null
  /// character, and short enough for MQTT.
  static func isPublishable(_ topic: String) -> Bool {
    !topic.isEmpty && topic.utf8.count <= Int(UInt16.max)
      && !topic.utf8.contains { $0 == 0 || $0 == UInt8(ascii: "+") || $0 == UInt8(ascii: "#") }
  }

  /// Whether `filter` is a topic filter: not empty, short enough, no null character, `+`
  /// only as a whole level, and `#` only as the whole last level.
  static func isFilter(_ filter: String) -> Bool {
    guard !filter.isEmpty, filter.utf8.count <= Int(UInt16.max), !filter.utf8.contains(0) else {
      return false
    }
    let levels = filter.split(separator: "/", omittingEmptySubsequences: false)
    for (index, level) in levels.enumerated() {
      if level.contains("#") && (level != "#" || index != levels.count - 1) { return false }
      if level.contains("+") && level != "+" { return false }
    }
    return true
  }

  /// A CONNACK refusal that isn't about the login, in words. It is retried like an outage.
  static func refusal(_ returnCode: UInt8) -> String {
    switch returnCode {
    case 1: "The broker refused the connection: it doesn't speak MQTT 3.1.1."
    case 2: "The broker refused the connection: it turned down the client ID."
    case 3: "The broker refused the connection: its MQTT service is unavailable."
    default: "The broker refused the connection with return code \(returnCode)."
    }
  }

  static func words(for error: MQTTCodecError) -> String {
    switch error {
    case .packetTooLarge(let length, _):
      "The broker sent a packet of \(length) bytes, over the 256 KiB limit."
    case .unsupportedPacketType(let type):
      "The broker sent a packet of type \(type), which this client doesn't speak."
    default:
      "The broker sent a malformed packet. \(error.reason)"
    }
  }
}

// MARK: - The client's records

extension MQTTClient {
  /// How a connection ended.
  private enum Outcome {
    case loginRefused(String)
    case lost(String)
    case stopped
  }

  /// A message waiting to go out.
  private struct Message {
    let topic: String
    let payload: Data
    let retain: Bool

    func publish(_ packetID: UInt16, duplicate: Bool) -> MQTTPublish {
      MQTTPublish(
        topic: topic, payload: payload, qos: .atLeastOnce, retain: retain,
        duplicate: duplicate, packetID: packetID)
    }
  }

  /// A message sent and waiting for its PUBACK.
  private struct InFlight {
    let packetID: UInt16
    let message: Message
  }

  /// One connection, from opening the WebSocket until it ends. Only the client touches it,
  /// on its own executor.
  private final class Attempt {
    enum Phase {
      case opening
      case awaitingConnAck
      case connected
      case ended
    }

    let id: Int
    var phase = Phase.opening
    var transport: (any MQTTTransport)?
    var decoder = MQTTPacketDecoder(maxRemainingLength: MQTTClient.maxPacketSize)
    /// The connection's own packets waiting to go out, ahead of any message.
    var control: [Data] = []
    var awaitingPingResponse = false
    var writerWaiting: CheckedContinuation<Data?, Never>?
    var endWaiter: CheckedContinuation<Outcome, Never>?
    var opening: Task<Void, Never>?
    var connectTimer: Task<Void, Never>?
    var keepAlive: Task<Void, Never>?
    var reader: Task<Void, Never>?
    var writer: Task<Void, Never>?

    init(id: Int) {
      self.id = id
    }
  }
}
