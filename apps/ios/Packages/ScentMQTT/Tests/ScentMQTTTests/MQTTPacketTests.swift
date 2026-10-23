// The packet codec, byte for byte against the MQTT 3.1.1 specification (OASIS standard,
// 29 October 2014). Section numbers refer to it.
import Foundation
import Testing

@testable import ScentMQTT

private func encoded(_ packet: MQTTPacket) throws -> [UInt8] {
  Array(try packet.encode())
}

private func decoded(_ bytes: [UInt8]) throws -> MQTTPacket {
  try MQTTPacketDecoder.decode(bytes)
}

/// The error encoding `packet` throws, or nil when it encodes.
private func encodingError(_ packet: MQTTPacket) -> MQTTCodecError? {
  do {
    _ = try packet.encode()
    return nil
  } catch {
    return error
  }
}

/// The error decoding `bytes` throws, or nil when it decodes.
private func decodingError(_ bytes: [UInt8]) -> MQTTCodecError? {
  do {
    _ = try MQTTPacketDecoder.decode(bytes)
    return nil
  } catch {
    return error
  }
}

private func isMalformed(_ error: MQTTCodecError?) -> Bool {
  if case .malformed = error { return true }
  return false
}

@Suite("Remaining length (2.2.3)")
struct RemainingLengthTests {
  /// The boundaries in table 2.4: the smallest and largest value of each length in bytes.
  static let boundaries: [(Int, [UInt8])] = [
    (0, [0x00]),
    (127, [0x7F]),
    (128, [0x80, 0x01]),
    (16_383, [0xFF, 0x7F]),
    (16_384, [0x80, 0x80, 0x01]),
    (2_097_151, [0xFF, 0xFF, 0x7F]),
    (2_097_152, [0x80, 0x80, 0x80, 0x01]),
    (268_435_455, [0xFF, 0xFF, 0xFF, 0x7F]),
  ]

  @Test("is written as table 2.4 says", arguments: boundaries)
  func writes(length: Int, bytes: [UInt8]) throws {
    #expect(try MQTTPacket.remainingLength(length) == bytes)
    #expect(MQTTPacket.size(remainingLength: length) == 1 + bytes.count + length)
  }

  @Test("is read back as table 2.4 says", arguments: boundaries.dropFirst())
  func reads(length: Int, bytes: [UInt8]) throws {
    // A PUBLISH header announcing `length`, with no body: the decoder has read the length
    // and waits for the rest.
    var decoder = MQTTPacketDecoder()
    decoder.append([0x30] + bytes)
    #expect(try decoder.next() == nil)
    guard length <= 2_097_152 else { return }
    // One byte short of the whole packet is still not enough. The body is an empty topic
    // and a payload of zeros.
    decoder.append([UInt8](repeating: 0, count: length - 1))
    #expect(try decoder.next() == nil)
    decoder.append([0])
    let payload = Data(repeating: 0, count: length - 2)
    #expect(try decoder.next() == .publish(MQTTPublish(topic: "", payload: payload)))
  }

  @Test("over 268,435,455 can't be written")
  func tooLong() {
    #expect(throws: MQTTCodecError.packetTooLarge(length: 268_435_456, limit: 268_435_455)) {
      try MQTTPacket.remainingLength(268_435_456)
    }
  }

  @Test("of five bytes is malformed")
  func fiveBytes() {
    var decoder = MQTTPacketDecoder()
    decoder.append([0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0x01])
    #expect(throws: MQTTCodecError.malformedRemainingLength) { try decoder.next() }
  }
}

@Suite("Strings (1.5.3)")
struct StringTests {
  @Test("are written as the length then UTF-8, as in figure 1.3")
  func figure13() throws {
    // "A𪛔": LATIN CAPITAL LETTER A, then U+2A6D4.
    let publish = MQTTPublish(topic: "A\u{2A6D4}", payload: Data())
    #expect(
      try encoded(.publish(publish)) == [0x30, 0x07, 0x00, 0x05, 0x41, 0xF0, 0xAA, 0x9B, 0x94])
    #expect(
      try decoded([0x30, 0x07, 0x00, 0x05, 0x41, 0xF0, 0xAA, 0x9B, 0x94]) == .publish(publish))
  }

  @Test("can't hold the null character")
  func noNull() {
    #expect(throws: MQTTCodecError.unencodableString) {
      try MQTTPacket.publish(MQTTPublish(topic: "a\u{0}b", payload: Data())).encode()
    }
    #expect(isMalformed(decodingError([0x30, 0x05, 0x00, 0x03, 0x61, 0x00, 0x62])))
  }

  @Test("can't be over 65,535 bytes")
  func tooLong() throws {
    let longest = String(repeating: "a", count: 65_535)
    let bytes = try encoded(.publish(MQTTPublish(topic: longest, payload: Data())))
    #expect(Array(bytes.prefix(6)) == [0x30, 0x81, 0x80, 0x04, 0xFF, 0xFF])
    #expect(throws: MQTTCodecError.unencodableString) {
      try MQTTPacket.publish(MQTTPublish(topic: longest + "a", payload: Data())).encode()
    }
  }

  @Test("must be valid UTF-8")
  func invalidUTF8() {
    // 0xC3 starts a two-byte sequence that never comes.
    #expect(isMalformed(decodingError([0x30, 0x04, 0x00, 0x02, 0xC3, 0x28])))
  }

  @Test("can't run past the end of the packet")
  func pastTheEnd() {
    #expect(isMalformed(decodingError([0x30, 0x04, 0x00, 0x05, 0x61, 0x62])))
  }
}

@Suite("CONNECT (3.1)")
struct ConnectTests {
  @Test("with a login and a clean session")
  func login() throws {
    let connect = MQTTConnect(
      clientID: "c", username: "u", password: "p", keepAliveSeconds: 30, cleanSession: true)
    let bytes: [UInt8] = [
      0x10, 0x13,  // CONNECT, 19 bytes
      0x00, 0x04, 0x4D, 0x51, 0x54, 0x54,  // "MQTT"
      0x04,  // level 4: 3.1.1
      0xC2,  // username, password, clean session
      0x00, 0x1E,  // keep alive 30 s
      0x00, 0x01, 0x63,  // client ID "c"
      0x00, 0x01, 0x75,  // username "u"
      0x00, 0x01, 0x70,  // password "p"
    ]
    #expect(try encoded(.connect(connect)) == bytes)
    #expect(try decoded(bytes) == .connect(connect))
  }

  @Test("without a login or a clean session")
  func anonymous() throws {
    let connect = MQTTConnect(
      clientID: "ab", username: nil, password: nil, keepAliveSeconds: 0x0102,
      cleanSession: false)
    let bytes: [UInt8] = [
      0x10, 0x0E, 0x00, 0x04, 0x4D, 0x51, 0x54, 0x54, 0x04, 0x00, 0x01, 0x02,
      0x00, 0x02, 0x61, 0x62,
    ]
    #expect(try encoded(.connect(connect)) == bytes)
    #expect(try decoded(bytes) == .connect(connect))
  }

  @Test("can't carry a password without a username (3.1.2.9)")
  func passwordNeedsUsername() {
    let connect = MQTTConnect(
      clientID: "c", username: nil, password: "p", keepAliveSeconds: 0, cleanSession: true)
    #expect(isMalformed(encodingError(.connect(connect))))
  }

  @Test("is never printed with its password")
  func hidesThePassword() {
    let connect = MQTTConnect(
      clientID: "c", username: "u", password: "not-a-real-secret", keepAliveSeconds: 30,
      cleanSession: true)
    for text in [
      String(describing: connect), String(reflecting: connect),
      String(describing: MQTTPacket.connect(connect)),
    ] {
      #expect(!text.contains("not-a-real-secret"), "\(text)")
    }
  }

  @Test("that isn't MQTT 3.1.1 is malformed")
  func otherProtocols() {
    // Level 3 (MQTT 3.1), and the reserved flag set.
    let level3: [UInt8] = [
      0x10, 0x0D, 0x00, 0x04, 0x4D, 0x51, 0x54, 0x54, 0x03, 0x02, 0x00, 0x00, 0x00, 0x01, 0x63,
    ]
    let reserved: [UInt8] = [
      0x10, 0x0D, 0x00, 0x04, 0x4D, 0x51, 0x54, 0x54, 0x04, 0x03, 0x00, 0x00, 0x00, 0x01, 0x63,
    ]
    #expect(isMalformed(decodingError(level3)))
    #expect(isMalformed(decodingError(reserved)))
  }
}

@Suite("CONNACK (3.2)")
struct ConnAckTests {
  @Test("accepted, with and without a session present")
  func accepted() throws {
    #expect(
      try decoded([0x20, 0x02, 0x00, 0x00]) == .connAck(sessionPresent: false, returnCode: 0))
    #expect(try decoded([0x20, 0x02, 0x01, 0x00]) == .connAck(sessionPresent: true, returnCode: 0))
    #expect(try encoded(.connAck(sessionPresent: true, returnCode: 0)) == [0x20, 0x02, 0x01, 0x00])
  }

  @Test("refused, with each return code in table 3.1", arguments: [1, 2, 3, 4, 5] as [UInt8])
  func refused(code: UInt8) throws {
    #expect(
      try decoded([0x20, 0x02, 0x00, code]) == .connAck(sessionPresent: false, returnCode: code))
  }

  @Test("with reserved bits, the wrong flags or the wrong length is malformed")
  func malformed() {
    #expect(isMalformed(decodingError([0x20, 0x02, 0x02, 0x00])))
    #expect(isMalformed(decodingError([0x21, 0x02, 0x00, 0x00])))
    #expect(isMalformed(decodingError([0x20, 0x03, 0x00, 0x00, 0x00])))
    #expect(isMalformed(decodingError([0x20, 0x01, 0x00])))
  }
}

@Suite("PUBLISH (3.3)")
struct PublishTests {
  /// Topic "a/b" and packet ID 10, as in figure 3.11, with the payload "hi".
  @Test("at QoS 1, as in figure 3.11")
  func qos1() throws {
    let publish = MQTTPublish(
      topic: "a/b", payload: Data("hi".utf8), qos: .atLeastOnce, packetID: 10)
    let bytes: [UInt8] = [0x32, 0x09, 0x00, 0x03, 0x61, 0x2F, 0x62, 0x00, 0x0A, 0x68, 0x69]
    #expect(try encoded(.publish(publish)) == bytes)
    #expect(try decoded(bytes) == .publish(publish))
  }

  @Test("at QoS 0 has no packet ID")
  func qos0() throws {
    let publish = MQTTPublish(topic: "a/b", payload: Data("hi".utf8))
    let bytes: [UInt8] = [0x30, 0x07, 0x00, 0x03, 0x61, 0x2F, 0x62, 0x68, 0x69]
    #expect(try encoded(.publish(publish)) == bytes)
    #expect(try decoded(bytes) == .publish(publish))
  }

  @Test("sets DUP, QoS and RETAIN in the fixed header (3.3.1)")
  func flags() throws {
    let publish = MQTTPublish(
      topic: "t", payload: Data(), qos: .atLeastOnce, retain: true, duplicate: true,
      packetID: 0x0102)
    let bytes: [UInt8] = [0x3B, 0x05, 0x00, 0x01, 0x74, 0x01, 0x02]
    #expect(try encoded(.publish(publish)) == bytes)
    #expect(try decoded(bytes) == .publish(publish))
    // QoS 2 is read, so that the client can say it wasn't asked for.
    #expect(
      try decoded([0x34, 0x05, 0x00, 0x01, 0x74, 0x00, 0x07])
        == .publish(MQTTPublish(topic: "t", payload: Data(), qos: .exactlyOnce, packetID: 7)))
  }

  @Test("with QoS 3 is malformed (3.3.1.2)")
  func qos3() {
    #expect(isMalformed(decodingError([0x36, 0x05, 0x00, 0x01, 0x74, 0x00, 0x01])))
  }

  @Test("at QoS 1 needs a packet ID other than 0 (2.3.1)")
  func packetIDZero() {
    #expect(isMalformed(decodingError([0x32, 0x05, 0x00, 0x01, 0x74, 0x00, 0x00])))
    let noID = MQTTPublish(topic: "t", payload: Data(), qos: .atLeastOnce)
    #expect(isMalformed(encodingError(.publish(noID))))
    let zero = MQTTPublish(topic: "t", payload: Data(), qos: .atLeastOnce, packetID: 0)
    #expect(isMalformed(encodingError(.publish(zero))))
  }

  @Test("carries any payload, empty or binary")
  func payloads() throws {
    let binary = Data((0...255).map { UInt8($0) })
    let publish = MQTTPublish(topic: "t", payload: binary, qos: .atLeastOnce, packetID: 1)
    #expect(try decoded(encoded(.publish(publish))) == .publish(publish))
  }
}

@Suite("PUBACK (3.4), SUBSCRIBE (3.8), SUBACK (3.9)")
struct AcknowledgementTests {
  @Test("PUBACK is the packet ID")
  func pubAck() throws {
    #expect(try encoded(.pubAck(packetID: 10)) == [0x40, 0x02, 0x00, 0x0A])
    #expect(try decoded([0x40, 0x02, 0x00, 0x0A]) == .pubAck(packetID: 10))
    #expect(isMalformed(decodingError([0x40, 0x02, 0x00, 0x00])))
    #expect(isMalformed(decodingError([0x40, 0x03, 0x00, 0x0A, 0x00])))
  }

  @Test("SUBSCRIBE, as in figures 3.21 and 3.22")
  func subscribe() throws {
    let subscribe = MQTTPacket.subscribe(
      packetID: 10,
      subscriptions: [
        MQTTSubscription(filter: "a/b", qos: .atLeastOnce),
        MQTTSubscription(filter: "c/d", qos: .exactlyOnce),
      ])
    let bytes: [UInt8] = [
      0x82, 0x0E,  // SUBSCRIBE with its reserved flags 0010, 14 bytes
      0x00, 0x0A,  // packet ID 10
      0x00, 0x03, 0x61, 0x2F, 0x62, 0x01,  // "a/b" at QoS 1
      0x00, 0x03, 0x63, 0x2F, 0x64, 0x02,  // "c/d" at QoS 2
    ]
    #expect(try encoded(subscribe) == bytes)
    #expect(try decoded(bytes) == subscribe)
  }

  @Test("SUBSCRIBE needs a filter and its reserved flags")
  func subscribeRules() {
    #expect(isMalformed(encodingError(.subscribe(packetID: 1, subscriptions: []))))
    #expect(isMalformed(decodingError([0x82, 0x02, 0x00, 0x0A])))
    #expect(isMalformed(decodingError([0x80, 0x06, 0x00, 0x0A, 0x00, 0x01, 0x23, 0x01])))
  }

  @Test("SUBACK, as in figure 3.26")
  func subAck() throws {
    let bytes: [UInt8] = [0x90, 0x05, 0x00, 0x0A, 0x00, 0x02, 0x80]
    #expect(try decoded(bytes) == .subAck(packetID: 10, returnCodes: [0x00, 0x02, 0x80]))
    #expect(try encoded(.subAck(packetID: 10, returnCodes: [0x00, 0x02, 0x80])) == bytes)
  }

  @Test("SUBACK with a reserved return code or none is malformed")
  func subAckRules() {
    #expect(isMalformed(decodingError([0x90, 0x03, 0x00, 0x0A, 0x03])))
    #expect(isMalformed(decodingError([0x90, 0x02, 0x00, 0x0A])))
  }
}

@Suite("PINGREQ (3.12), PINGRESP (3.13), DISCONNECT (3.14)")
struct HeaderOnlyTests {
  @Test(
    "are two bytes",
    arguments: [
      (MQTTPacket.pingReq, [0xC0, 0x00] as [UInt8]),
      (.pingResp, [0xD0, 0x00]),
      (.disconnect, [0xE0, 0x00]),
    ])
  func twoBytes(packet: MQTTPacket, bytes: [UInt8]) throws {
    #expect(try encoded(packet) == bytes)
    #expect(try decoded(bytes) == packet)
  }

  @Test("with a body or flags are malformed")
  func malformed() {
    #expect(isMalformed(decodingError([0xC0, 0x01, 0x00])))
    #expect(isMalformed(decodingError([0xD1, 0x00])))
    #expect(isMalformed(decodingError([0xE0, 0x01, 0x00])))
  }
}

@Suite("Packet types")
struct PacketTypeTests {
  @Test(
    "that this client doesn't speak are refused at once",
    arguments: [0x00, 0x50, 0x62, 0x70, 0xA2, 0xB0, 0xF0] as [UInt8])
  func unsupported(firstByte: UInt8) {
    var decoder = MQTTPacketDecoder()
    decoder.append([firstByte])
    #expect(throws: MQTTCodecError.unsupportedPacketType(firstByte >> 4)) { try decoder.next() }
  }
}
