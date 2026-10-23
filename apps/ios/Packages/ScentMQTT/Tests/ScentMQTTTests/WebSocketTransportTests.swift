// The real client and its URLSession WebSocket against a small broker on 127.0.0.1, played
// with Network.framework: the `mqtt` subprotocol, binary messages, and the broker closing.
// Loopback only; nothing leaves this machine.
import Foundation
import Network
import Synchronization
import Testing

@testable import ScentMQTT

/// A WebSocket server on 127.0.0.1 that answers like a broker: CONNACK to a CONNECT,
/// SUBACK to a SUBSCRIBE, PUBACK to a QoS 1 PUBLISH, PINGRESP to a PINGREQ. Every packet
/// the client sends is kept, decoded.
final class LoopbackBroker: Sendable {
  /// Each packet the client sent, and whether it came as a binary message.
  let received = AsyncQueue<(packet: MQTTPacket, binary: Bool)>()
  private let listener: NWListener
  private let queue = DispatchQueue(label: "loopback-broker")
  private let connection = Mutex<NWConnection?>(nil)

  /// A broker that answers the WebSocket handshake with `subprotocol`.
  init(subprotocol: String?) throws {
    let webSocket = NWProtocolWebSocket.Options()
    webSocket.autoReplyPing = true
    webSocket.setClientRequestHandler(queue) { _, _ in
      NWProtocolWebSocket.Response(status: .accept, subprotocol: subprotocol)
    }
    let parameters = NWParameters.tcp
    parameters.defaultProtocolStack.applicationProtocols.insert(webSocket, at: 0)
    parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
    listener = try NWListener(using: parameters)
  }

  /// Starts listening and returns the port.
  func start() async throws -> UInt16 {
    listener.newConnectionHandler = { [self] connection in
      self.connection.withLock { $0 = connection }
      connection.start(queue: queue)
      receive(on: connection)
    }
    let port: UInt16 = try await withCheckedThrowingContinuation { continuation in
      // Taken by the first state that settles it, so it is resumed once.
      let waiting = Mutex<CheckedContinuation<UInt16, any Error>?>(continuation)
      listener.stateUpdateHandler = { [listener] state in
        let result: Result<UInt16, any Error>? =
          switch state {
          case .ready: .success(listener.port?.rawValue ?? 0)
          case .failed(let error): .failure(error)
          default: nil
          }
        guard let result, let continuation = waiting.withLock({ $0.take() }) else { return }
        continuation.resume(with: result)
      }
      listener.start(queue: queue)
    }
    return port
  }

  func stop() {
    connection.withLock { $0?.cancel() }
    listener.cancel()
  }

  /// Sends a packet to the client as a binary message.
  func send(_ packet: MQTTPacket) throws {
    send(try packet.encode(), opcode: .binary)
  }

  /// Sends a text message, which MQTT never uses.
  func sendText(_ text: String) {
    send(Data(text.utf8), opcode: .text)
  }

  /// Closes the WebSocket from the broker's side, with a close frame.
  func close() {
    let metadata = NWProtocolWebSocket.Metadata(opcode: .close)
    metadata.closeCode = .protocolCode(.normalClosure)
    let context = NWConnection.ContentContext(identifier: "close", metadata: [metadata])
    connection.withLock { connection in
      connection?.send(
        content: nil, contentContext: context, isComplete: true,
        completion: .contentProcessed { _ in })
    }
  }

  private func send(_ data: Data, opcode: NWProtocolWebSocket.Opcode) {
    let metadata = NWProtocolWebSocket.Metadata(opcode: opcode)
    let context = NWConnection.ContentContext(identifier: "message", metadata: [metadata])
    connection.withLock { connection in
      connection?.send(
        content: data, contentContext: context, isComplete: true,
        completion: .contentProcessed { _ in })
    }
  }

  private func receive(on connection: NWConnection) {
    connection.receiveMessage { [self] content, context, _, error in
      guard error == nil, let content else { return }
      let metadata =
        context?.protocolMetadata(definition: NWProtocolWebSocket.definition)
        as? NWProtocolWebSocket.Metadata
      if let packet = try? MQTTPacketDecoder.decode(content) {
        received.push((packet, metadata?.opcode == .binary))
        answer(packet)
      }
      receive(on: connection)
    }
  }

  private func answer(_ packet: MQTTPacket) {
    switch packet {
    case .connect:
      try? send(.connAck(sessionPresent: false, returnCode: 0))
    case .subscribe(let packetID, let subscriptions):
      try? send(.subAck(packetID: packetID, returnCodes: subscriptions.map { $0.qos.rawValue }))
    case .publish(let publish):
      if let packetID = publish.packetID { try? send(.pubAck(packetID: packetID)) }
    case .pingReq:
      try? send(.pingResp)
    default:
      break
    }
  }
}

@Suite("The WebSocket transport, against a broker on 127.0.0.1")
struct WebSocketTransportTests {
  private func client(port: UInt16, events: AsyncQueue<MQTTEvent>) -> MQTTClient {
    let options = MQTTOptions(
      url: URL(string: "ws://127.0.0.1:\(port)/mqtt")!, username: "example-user",
      password: "x", subscriptions: ["devices/#"])
    return MQTTClient(options: options) { events.push($0) }
  }

  @Test("logs in, subscribes, publishes, receives and says goodbye, in binary messages")
  func roundTrip() async throws {
    let broker = try LoopbackBroker(subprotocol: "mqtt")
    defer { broker.stop() }
    let port = try await broker.start()
    let events = AsyncQueue<MQTTEvent>()
    let client = client(port: port, events: events)
    await client.start()

    #expect(try await events.next("connected") == .connected)
    let connect = try await broker.received.next("the CONNECT")
    #expect(connect.binary)
    guard case .connect(let login) = connect.packet else {
      Issue.record("Expected a CONNECT, got \(connect.packet)")
      return
    }
    #expect(login.username == "example-user")
    #expect(login.password == "x")
    #expect(
      try await broker.received.next("the SUBSCRIBE").packet
        == .subscribe(
          packetID: 1, subscriptions: [MQTTSubscription(filter: "devices/#", qos: .atLeastOnce)]))

    try broker.send(
      .publish(
        MQTTPublish(
          topic: "devices/TESTDEVICE01", payload: Data("{}".utf8), qos: .atLeastOnce,
          packetID: 7)))
    #expect(
      try await events.next("the message")
        == .message(topic: "devices/TESTDEVICE01", payload: Data("{}".utf8), retained: false))
    #expect(try await broker.received.next("the PUBACK").packet == .pubAck(packetID: 7))

    try await client.publish(topic: "devices/TESTDEVICE01", payload: Data("{\"a\":1}".utf8))
    #expect(
      try await broker.received.next("the PUBLISH").packet
        == .publish(
          MQTTPublish(
            topic: "devices/TESTDEVICE01", payload: Data("{\"a\":1}".utf8), qos: .atLeastOnce,
            packetID: 2)))

    await client.stop()
    #expect(try await broker.received.next("the DISCONNECT").packet == .disconnect)
    #expect(await !client.isConnected)
  }

  @Test("a broker that doesn't agree to the mqtt subprotocol is refused")
  func wrongSubprotocol() async throws {
    let broker = try LoopbackBroker(subprotocol: nil)
    defer { broker.stop() }
    let port = try await broker.start()
    let events = AsyncQueue<MQTTEvent>()
    let client = client(port: port, events: events)
    await client.start()
    #expect(
      try await events.next("the refusal")
        == .disconnected("The broker didn't agree to speak MQTT over the WebSocket."))
    await client.stop()
  }

  @Test("a text message ends the connection")
  func textMessage() async throws {
    let broker = try LoopbackBroker(subprotocol: "mqtt")
    defer { broker.stop() }
    let port = try await broker.start()
    let events = AsyncQueue<MQTTEvent>()
    let client = client(port: port, events: events)
    await client.start()
    #expect(try await events.next("connected") == .connected)
    broker.sendText("hello")
    #expect(
      try await events.next("the drop")
        == .disconnected("The broker sent text where MQTT needs binary."))
    await client.stop()
  }

  @Test("the broker closing the WebSocket is reported as that")
  func brokerCloses() async throws {
    let broker = try LoopbackBroker(subprotocol: "mqtt")
    defer { broker.stop() }
    let port = try await broker.start()
    let events = AsyncQueue<MQTTEvent>()
    let client = client(port: port, events: events)
    await client.start()
    #expect(try await events.next("connected") == .connected)
    broker.close()
    #expect(
      try await events.next("the drop") == .disconnected("The broker closed the connection."))
    await client.stop()
  }

  @Test("nothing listening is reported as a disconnect, in words")
  func nothingListening() async throws {
    let broker = try LoopbackBroker(subprotocol: "mqtt")
    let port = try await broker.start()
    broker.stop()
    try await Task.sleep(for: .milliseconds(100))
    let events = AsyncQueue<MQTTEvent>()
    let client = client(port: port, events: events)
    await client.start()
    let event = try await events.next("the failure")
    guard case .disconnected(let reason) = event else {
      Issue.record("Expected a disconnect, got \(event)")
      return
    }
    #expect(!reason.isEmpty)
    await client.stop()
  }
}
