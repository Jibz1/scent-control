// Reads MQTT 3.1.1 packets back out of bytes, however the bytes arrive: a WebSocket
// message may hold part of a packet, one packet, or several. Pure: bytes in, packets out.
import Foundation

/// Gathers bytes and hands back each packet once all of it has arrived.
struct MQTTPacketDecoder {
  /// The largest remaining length taken. A packet over it fails as soon as its length has
  /// been read, before the rest of it arrives.
  let maxRemainingLength: Int
  private var buffer: [UInt8] = []
  /// Where the next packet starts in `buffer`. The bytes before it have been read.
  private var start = 0

  init(maxRemainingLength: Int = MQTTPacket.maxRemainingLength) {
    self.maxRemainingLength = maxRemainingLength
  }

  /// How many bytes are waiting to make up a packet.
  var bufferedByteCount: Int { buffer.count - start }

  /// Adds bytes as they arrive.
  mutating func append(_ bytes: some Sequence<UInt8>) {
    buffer.append(contentsOf: bytes)
  }

  /// The next whole packet, or nil until more bytes arrive. Throws on bytes that can't be
  /// a packet; the stream can't be trusted after that.
  mutating func next() throws(MQTTCodecError) -> MQTTPacket? {
    guard start < buffer.count else { return nil }
    let firstByte = buffer[start]
    let type = firstByte >> 4
    guard MQTTPacketDecoder.supportedTypes.contains(type) else {
      throw .unsupportedPacketType(type)
    }

    var index = start + 1
    var length = 0
    var multiplier = 1
    var lengthBytes = 0
    while true {
      guard index < buffer.count else { return nil }
      let byte = buffer[index]
      index += 1
      lengthBytes += 1
      length += Int(byte & 0x7F) * multiplier
      if byte & 0x80 == 0 { break }
      guard lengthBytes < 4 else { throw .malformedRemainingLength }
      multiplier *= 128
    }
    guard length <= maxRemainingLength else {
      throw .packetTooLarge(length: length, limit: maxRemainingLength)
    }
    guard buffer.count - index >= length else { return nil }

    start = index + length
    let packet = try MQTTPacketDecoder.packet(
      firstByte: firstByte, body: buffer[index..<start])
    if start == buffer.count {
      buffer.removeAll(keepingCapacity: true)
      start = 0
    } else if start >= 64 * 1024 {
      buffer.removeFirst(start)
      start = 0
    }
    return packet
  }

  /// One whole packet from exactly its bytes, for tests and tools.
  static func decode(_ bytes: some Sequence<UInt8>) throws(MQTTCodecError) -> MQTTPacket {
    var decoder = MQTTPacketDecoder()
    decoder.append(bytes)
    guard let packet = try decoder.next() else {
      throw .malformed("The bytes end before the packet does.")
    }
    guard decoder.bufferedByteCount == 0 else {
      throw .malformed("There are bytes after the packet.")
    }
    return packet
  }

  /// The packet types read: CONNECT, CONNACK, PUBLISH, PUBACK, SUBSCRIBE, SUBACK, PINGREQ,
  /// PINGRESP and DISCONNECT.
  private static let supportedTypes: Set<UInt8> = [1, 2, 3, 4, 8, 9, 12, 13, 14]

  private static func packet(
    firstByte: UInt8, body: ArraySlice<UInt8>
  ) throws(MQTTCodecError) -> MQTTPacket {
    let type = firstByte >> 4
    let flags = firstByte & 0x0F
    var reader = ByteReader(body)
    let packet: MQTTPacket
    switch type {
    case 1:
      try expectFlags(flags, 0, "CONNECT")
      packet = .connect(try connect(&reader))
    case 2:
      try expectFlags(flags, 0, "CONNACK")
      try expectLength(body, 2, "CONNACK")
      let acknowledgeFlags = try reader.byte()
      guard acknowledgeFlags & 0xFE == 0 else {
        throw .malformed("A CONNACK sets reserved bits in its acknowledge flags.")
      }
      packet = .connAck(sessionPresent: acknowledgeFlags == 1, returnCode: try reader.byte())
    case 3:
      packet = .publish(try publish(flags: flags, &reader))
    case 4:
      try expectFlags(flags, 0, "PUBACK")
      try expectLength(body, 2, "PUBACK")
      packet = .pubAck(packetID: try reader.packetID("PUBACK"))
    case 8:
      try expectFlags(flags, 0x02, "SUBSCRIBE")
      let packetID = try reader.packetID("SUBSCRIBE")
      var subscriptions: [MQTTSubscription] = []
      while !reader.isAtEnd {
        let filter = try reader.string()
        let requested = try reader.byte()
        guard let qos = MQTTQoS(rawValue: requested) else {
          throw .malformed("A SUBSCRIBE asks for QoS byte \(requested).")
        }
        subscriptions.append(MQTTSubscription(filter: filter, qos: qos))
      }
      guard !subscriptions.isEmpty else {
        throw .malformed("A SUBSCRIBE has no topic filter.")
      }
      packet = .subscribe(packetID: packetID, subscriptions: subscriptions)
    case 9:
      try expectFlags(flags, 0, "SUBACK")
      let packetID = try reader.packetID("SUBACK")
      let returnCodes = reader.rest()
      guard !returnCodes.isEmpty else {
        throw .malformed("A SUBACK has no return code.")
      }
      for code in returnCodes where ![0x00, 0x01, 0x02, 0x80].contains(code) {
        throw .malformed("A SUBACK has the reserved return code \(code).")
      }
      packet = .subAck(packetID: packetID, returnCodes: returnCodes)
    case 12:
      try expectFlags(flags, 0, "PINGREQ")
      try expectLength(body, 0, "PINGREQ")
      packet = .pingReq
    case 13:
      try expectFlags(flags, 0, "PINGRESP")
      try expectLength(body, 0, "PINGRESP")
      packet = .pingResp
    case 14:
      try expectFlags(flags, 0, "DISCONNECT")
      try expectLength(body, 0, "DISCONNECT")
      packet = .disconnect
    default:
      throw .unsupportedPacketType(type)
    }
    guard reader.isAtEnd else {
      throw .malformed("A \(packet.name) has bytes left over after its contents.")
    }
    return packet
  }

  private static func connect(_ reader: inout ByteReader) throws(MQTTCodecError) -> MQTTConnect {
    guard try reader.string() == "MQTT" else {
      throw .malformed("A CONNECT names a protocol other than MQTT.")
    }
    let level = try reader.byte()
    guard level == 4 else {
      throw .malformed("A CONNECT asks for protocol level \(level), not 4 (MQTT 3.1.1).")
    }
    let flags = try reader.byte()
    guard flags & 0x01 == 0 else {
      throw .malformed("A CONNECT sets the reserved flag.")
    }
    guard flags & 0x04 == 0 else {
      throw .malformed("A CONNECT carries a will, which this client never reads.")
    }
    let hasUsername = flags & 0x80 != 0
    let hasPassword = flags & 0x40 != 0
    guard hasUsername || !hasPassword else {
      throw .malformed("A CONNECT has a password without a username.")
    }
    let keepAlive = try reader.uint16()
    let clientID = try reader.string()
    let username = hasUsername ? try reader.string() : nil
    var password: String?
    if hasPassword {
      guard let text = String(validating: try reader.binary(), as: UTF8.self) else {
        throw .malformed("A CONNECT's password isn't UTF-8, which this client never sends.")
      }
      password = text
    }
    return MQTTConnect(
      clientID: clientID, username: username, password: password,
      keepAliveSeconds: keepAlive, cleanSession: flags & 0x02 != 0)
  }

  private static func publish(
    flags: UInt8, _ reader: inout ByteReader
  ) throws(MQTTCodecError) -> MQTTPublish {
    guard let qos = MQTTQoS(rawValue: (flags >> 1) & 0x03) else {
      throw .malformed("A PUBLISH has QoS 3, which doesn't exist.")
    }
    let topic = try reader.string()
    let packetID = qos == .atMostOnce ? nil : try reader.packetID("PUBLISH")
    return MQTTPublish(
      topic: topic, payload: Data(reader.rest()), qos: qos, retain: flags & 0x01 != 0,
      duplicate: flags & 0x08 != 0, packetID: packetID)
  }

  private static func expectFlags(
    _ flags: UInt8, _ expected: UInt8, _ name: String
  ) throws(MQTTCodecError) {
    guard flags == expected else {
      throw .malformed("A \(name) has the wrong flags in its fixed header.")
    }
  }

  private static func expectLength(
    _ body: ArraySlice<UInt8>, _ expected: Int, _ name: String
  ) throws(MQTTCodecError) {
    guard body.count == expected else {
      throw .malformed(
        "A \(name) has the wrong length after its fixed header: \(body.count), not \(expected).")
    }
  }
}

/// Reads a packet's variable header and payload, failing at the end of the bytes.
private struct ByteReader {
  private let bytes: ArraySlice<UInt8>
  private var index: Int

  init(_ bytes: ArraySlice<UInt8>) {
    self.bytes = bytes
    index = bytes.startIndex
  }

  var isAtEnd: Bool { index == bytes.endIndex }

  mutating func byte() throws(MQTTCodecError) -> UInt8 {
    guard index < bytes.endIndex else { throw .malformed("A packet ends too early.") }
    defer { index += 1 }
    return bytes[index]
  }

  mutating func uint16() throws(MQTTCodecError) -> UInt16 {
    let high = try byte()
    let low = try byte()
    return UInt16(high) << 8 | UInt16(low)
  }

  /// A packet ID, which is never 0.
  mutating func packetID(_ name: String) throws(MQTTCodecError) -> UInt16 {
    let packetID = try uint16()
    guard packetID != 0 else { throw .malformed("A \(name) has packet ID 0.") }
    return packetID
  }

  /// Binary data: a length in two bytes, then that many bytes.
  mutating func binary() throws(MQTTCodecError) -> [UInt8] {
    let length = Int(try uint16())
    guard bytes.endIndex - index >= length else {
      throw .malformed("A string runs past the end of its packet.")
    }
    defer { index += length }
    return Array(bytes[index..<(index + length)])
  }

  /// A UTF-8 string, which must be valid UTF-8 and has no null character.
  mutating func string() throws(MQTTCodecError) -> String {
    let utf8 = try binary()
    guard !utf8.contains(0), let text = String(validating: utf8, as: UTF8.self) else {
      throw .malformed("A string isn't valid UTF-8 or has a null character in it.")
    }
    return text
  }

  /// Everything left.
  mutating func rest() -> [UInt8] {
    defer { index = bytes.endIndex }
    return Array(bytes[index...])
  }
}
