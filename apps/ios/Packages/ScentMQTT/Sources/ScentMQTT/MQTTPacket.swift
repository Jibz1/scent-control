// The MQTT 3.1.1 packets this client speaks, and how each is written as bytes. Pure: no
// I/O and no state. Reading them back is in `MQTTPacketDecoder.swift`.
import Foundation

/// How hard the client and the broker try to deliver a message.
enum MQTTQoS: UInt8, Sendable {
  case atMostOnce = 0
  case atLeastOnce = 1
  case exactlyOnce = 2
}

/// The CONNECT packet's contents. This client never sends a will, so there is none here.
struct MQTTConnect: Sendable, Equatable {
  var clientID: String
  var username: String?
  /// Never printed: see the `description` below.
  var password: String?
  var keepAliveSeconds: UInt16
  var cleanSession: Bool
}

extension MQTTConnect: CustomStringConvertible, CustomDebugStringConvertible {
  /// The packet in words, with the password hidden, so that a test failure or a log line
  /// never shows it.
  var description: String {
    let username = username.map { "\"\($0)\"" } ?? "none"
    let password = password == nil ? "none" : "<hidden>"
    return "CONNECT(clientID: \"\(clientID)\", username: \(username), password: \(password), "
      + "keepAlive: \(keepAliveSeconds) s, cleanSession: \(cleanSession))"
  }

  var debugDescription: String { description }
}

/// A PUBLISH packet's contents.
struct MQTTPublish: Sendable, Equatable {
  var topic: String
  var payload: Data
  var qos: MQTTQoS
  var retain: Bool
  /// Set when the sender is delivering the message again.
  var duplicate: Bool
  /// Present for QoS 1 and 2, and never 0.
  var packetID: UInt16?

  init(
    topic: String, payload: Data, qos: MQTTQoS = .atMostOnce, retain: Bool = false,
    duplicate: Bool = false, packetID: UInt16? = nil
  ) {
    self.topic = topic
    self.payload = payload
    self.qos = qos
    self.retain = retain
    self.duplicate = duplicate
    self.packetID = packetID
  }
}

/// One topic filter in a SUBSCRIBE, with the highest QoS asked for on it.
struct MQTTSubscription: Sendable, Equatable {
  var filter: String
  var qos: MQTTQoS
}

/// A packet of MQTT 3.1.1, limited to the ones this client sends or expects: no QoS 2
/// handshake (PUBREC, PUBREL, PUBCOMP) and no UNSUBSCRIBE.
enum MQTTPacket: Sendable, Equatable {
  case connect(MQTTConnect)
  case connAck(sessionPresent: Bool, returnCode: UInt8)
  case publish(MQTTPublish)
  case pubAck(packetID: UInt16)
  case subscribe(packetID: UInt16, subscriptions: [MQTTSubscription])
  case subAck(packetID: UInt16, returnCodes: [UInt8])
  case pingReq
  case pingResp
  case disconnect

  /// The largest remaining length MQTT can write: four length bytes, 268,435,455.
  static let maxRemainingLength = 268_435_455

  /// The packet's name as the specification spells it, for messages.
  var name: String {
    switch self {
    case .connect: "CONNECT"
    case .connAck: "CONNACK"
    case .publish: "PUBLISH"
    case .pubAck: "PUBACK"
    case .subscribe: "SUBSCRIBE"
    case .subAck: "SUBACK"
    case .pingReq: "PINGREQ"
    case .pingResp: "PINGRESP"
    case .disconnect: "DISCONNECT"
    }
  }

  /// The packet as bytes: the fixed header, then the variable header and the payload.
  /// Fails on what MQTT can't carry: a string over 65,535 bytes or with a null character
  /// in it, a QoS 1 or 2 PUBLISH without a packet ID, a SUBSCRIBE without a filter, a
  /// password without a username, or a packet over the largest remaining length.
  func encode() throws(MQTTCodecError) -> Data {
    var body = ByteWriter()
    let firstByte: UInt8
    switch self {
    case .connect(let connect):
      firstByte = 0x10
      try body.string("MQTT")
      body.byte(4)  // protocol level: 3.1.1
      var flags: UInt8 = 0
      if connect.username != nil { flags |= 0x80 }
      if connect.password != nil {
        guard connect.username != nil else {
          throw .malformed("A CONNECT can't carry a password without a username.")
        }
        flags |= 0x40
      }
      if connect.cleanSession { flags |= 0x02 }
      body.byte(flags)
      body.uint16(connect.keepAliveSeconds)
      try body.string(connect.clientID)
      if let username = connect.username { try body.string(username) }
      if let password = connect.password { try body.binary(Array(password.utf8)) }
    case .connAck(let sessionPresent, let returnCode):
      firstByte = 0x20
      body.byte(sessionPresent ? 1 : 0)
      body.byte(returnCode)
    case .publish(let publish):
      firstByte =
        0x30 | (publish.duplicate ? 0x08 : 0) | (publish.qos.rawValue << 1)
        | (publish.retain ? 0x01 : 0)
      try body.string(publish.topic)
      if publish.qos != .atMostOnce {
        guard let packetID = publish.packetID, packetID != 0 else {
          throw .malformed("A QoS \(publish.qos.rawValue) PUBLISH needs a packet ID other than 0.")
        }
        body.uint16(packetID)
      }
      body.append(publish.payload)
    case .pubAck(let packetID):
      firstByte = 0x40
      body.uint16(packetID)
    case .subscribe(let packetID, let subscriptions):
      firstByte = 0x82
      guard !subscriptions.isEmpty else {
        throw .malformed("A SUBSCRIBE needs at least one topic filter.")
      }
      body.uint16(packetID)
      for subscription in subscriptions {
        try body.string(subscription.filter)
        body.byte(subscription.qos.rawValue)
      }
    case .subAck(let packetID, let returnCodes):
      firstByte = 0x90
      body.uint16(packetID)
      body.append(returnCodes)
    case .pingReq:
      firstByte = 0xC0
    case .pingResp:
      firstByte = 0xD0
    case .disconnect:
      firstByte = 0xE0
    }
    var packet = Data([firstByte])
    packet.append(contentsOf: try Self.remainingLength(body.bytes.count))
    packet.append(contentsOf: body.bytes)
    return packet
  }

  /// A remaining length in MQTT's variable-length encoding: seven bits a byte, least
  /// significant first, the top bit set on every byte but the last.
  static func remainingLength(_ length: Int) throws(MQTTCodecError) -> [UInt8] {
    guard (0...maxRemainingLength).contains(length) else {
      throw .packetTooLarge(length: length, limit: maxRemainingLength)
    }
    var bytes: [UInt8] = []
    var rest = length
    repeat {
      var byte = UInt8(rest % 128)
      rest /= 128
      if rest > 0 { byte |= 0x80 }
      bytes.append(byte)
    } while rest > 0
    return bytes
  }

  /// The size of a whole packet, fixed header included, for a remaining length that MQTT
  /// can write.
  static func size(remainingLength: Int) -> Int {
    let lengthBytes =
      switch remainingLength {
      case ..<128: 1
      case ..<16_384: 2
      case ..<2_097_152: 3
      default: 4
      }
    return 1 + lengthBytes + remainingLength
  }
}

/// Why bytes could not be written as a packet or read as one.
enum MQTTCodecError: Error, Sendable, Equatable {
  /// The remaining length runs past four bytes.
  case malformedRemainingLength
  /// A remaining length over the limit: the decoder's, or MQTT's own when writing.
  case packetTooLarge(length: Int, limit: Int)
  /// A packet type this client doesn't speak: the QoS 2 handshake, UNSUBSCRIBE and
  /// UNSUBACK, and the reserved types 0 and 15.
  case unsupportedPacketType(UInt8)
  /// A string over 65,535 bytes of UTF-8 or with the null character in it, which MQTT
  /// can't carry.
  case unencodableString
  /// A packet that breaks the MQTT 3.1.1 rules, with what is wrong in words.
  case malformed(String)

  /// What went wrong, in a sentence.
  var reason: String {
    switch self {
    case .malformedRemainingLength:
      "A packet's length runs past the four bytes MQTT allows."
    case .packetTooLarge(let length, let limit):
      "A packet of \(length) bytes is over the limit of \(limit)."
    case .unsupportedPacketType(let type):
      "Packet type \(type) is not one this client speaks."
    case .unencodableString:
      "A string is over 65,535 bytes or has a null character in it."
    case .malformed(let what):
      what
    }
  }
}

/// Builds a packet's variable header and payload.
private struct ByteWriter {
  var bytes: [UInt8] = []

  mutating func byte(_ value: UInt8) {
    bytes.append(value)
  }

  mutating func uint16(_ value: UInt16) {
    bytes.append(UInt8(value >> 8))
    bytes.append(UInt8(value & 0xFF))
  }

  mutating func append(_ values: some Sequence<UInt8>) {
    bytes.append(contentsOf: values)
  }

  /// A UTF-8 string: its length in two bytes, then the bytes. MQTT forbids the null
  /// character in one.
  mutating func string(_ value: String) throws(MQTTCodecError) {
    let utf8 = Array(value.utf8)
    guard !utf8.contains(0) else { throw .unencodableString }
    try binary(utf8)
  }

  /// Binary data (the password): its length in two bytes, then the bytes.
  mutating func binary(_ value: [UInt8]) throws(MQTTCodecError) {
    guard value.count <= Int(UInt16.max) else { throw .unencodableString }
    uint16(UInt16(value.count))
    bytes.append(contentsOf: value)
  }
}
