// The streaming decoder: packets split across WebSocket messages, several in one, and the
// size limit.
import Foundation
import Testing

@testable import ScentMQTT

@Suite("The streaming decoder")
struct MQTTPacketDecoderTests {
  let publish = MQTTPacket.publish(
    MQTTPublish(topic: "a/b", payload: Data("hello".utf8), qos: .atLeastOnce, packetID: 7))

  /// Every packet the decoder has whole.
  private func drain(_ decoder: inout MQTTPacketDecoder) throws -> [MQTTPacket] {
    var packets: [MQTTPacket] = []
    while let packet = try decoder.next() { packets.append(packet) }
    return packets
  }

  @Test("waits for nothing when there is nothing")
  func empty() throws {
    var decoder = MQTTPacketDecoder()
    #expect(try decoder.next() == nil)
    #expect(decoder.bufferedByteCount == 0)
  }

  @Test("reads several packets from one message")
  func packed() throws {
    var bytes = try MQTTPacket.connAck(sessionPresent: false, returnCode: 0).encode()
    bytes.append(try publish.encode())
    bytes.append(try MQTTPacket.pingResp.encode())
    var decoder = MQTTPacketDecoder()
    decoder.append(bytes)
    #expect(
      try drain(&decoder) == [.connAck(sessionPresent: false, returnCode: 0), publish, .pingResp])
    #expect(decoder.bufferedByteCount == 0)
  }

  @Test("reads a packet that arrives a byte at a time")
  func byteByByte() throws {
    let bytes = try publish.encode()
    var decoder = MQTTPacketDecoder()
    for byte in bytes.dropLast() {
      decoder.append([byte])
      #expect(try decoder.next() == nil)
    }
    decoder.append([bytes.last!])
    #expect(try decoder.next() == publish)
    #expect(try decoder.next() == nil)
  }

  @Test("reads a packet split in its remaining length")
  func splitLength() throws {
    // 200 bytes of payload: the remaining length takes two bytes.
    let large = MQTTPacket.publish(
      MQTTPublish(topic: "t", payload: Data(repeating: 0x2A, count: 200)))
    let bytes = Array(try large.encode())
    #expect(bytes[1] & 0x80 != 0)
    var decoder = MQTTPacketDecoder()
    decoder.append(bytes[0..<2])
    #expect(try decoder.next() == nil)
    decoder.append(bytes[2...])
    #expect(try decoder.next() == large)
  }

  @Test("reads packets that straddle messages")
  func straddling() throws {
    var stream = Data()
    let packets: [MQTTPacket] = [
      .connAck(sessionPresent: false, returnCode: 0),
      .subAck(packetID: 1, returnCodes: [1, 1]),
      publish,
      .pubAck(packetID: 3),
      .publish(MQTTPublish(topic: "x", payload: Data(repeating: 1, count: 1_000))),
      .pingResp,
    ]
    for packet in packets { stream.append(try packet.encode()) }
    // Cut the stream every 7 bytes, so the cuts fall all over the packets.
    var decoder = MQTTPacketDecoder()
    var read: [MQTTPacket] = []
    var offset = 0
    while offset < stream.count {
      let end = min(offset + 7, stream.count)
      decoder.append(stream[offset..<end])
      read += try drain(&decoder)
      offset = end
    }
    #expect(read == packets)
  }

  @Test("reads a 200 KiB packet that comes in pieces")
  func largePacket() throws {
    let payload = Data((0..<(200 * 1024)).map { UInt8(truncatingIfNeeded: $0) })
    let large = MQTTPacket.publish(
      MQTTPublish(topic: "big", payload: payload, qos: .atLeastOnce, packetID: 9))
    let bytes = try large.encode()
    var decoder = MQTTPacketDecoder(maxRemainingLength: 256 * 1024)
    var offset = 0
    var read: [MQTTPacket] = []
    while offset < bytes.count {
      let end = min(offset + 16 * 1024, bytes.count)
      decoder.append(bytes[offset..<end])
      read += try drain(&decoder)
      offset = end
    }
    #expect(read == [large])
  }

  @Test("refuses a packet over its limit as soon as the length is known")
  func overTheLimit() throws {
    var decoder = MQTTPacketDecoder(maxRemainingLength: 256 * 1024)
    // A PUBLISH announcing 256 KiB and one byte: refused before any of its body arrives.
    decoder.append([0x30] + (try MQTTPacket.remainingLength(256 * 1024 + 1)))
    #expect(throws: MQTTCodecError.packetTooLarge(length: 256 * 1024 + 1, limit: 256 * 1024)) {
      try decoder.next()
    }
    // Exactly at the limit is fine: it waits for the body.
    var atLimit = MQTTPacketDecoder(maxRemainingLength: 256 * 1024)
    atLimit.append([0x30] + (try MQTTPacket.remainingLength(256 * 1024)))
    #expect(try atLimit.next() == nil)
  }

  @Test("decoding exactly one packet refuses bytes short of it or past it")
  func exactlyOne() throws {
    let bytes = Array(try MQTTPacket.pubAck(packetID: 1).encode())
    #expect(throws: MQTTCodecError.self) { try MQTTPacketDecoder.decode(bytes.dropLast()) }
    #expect(throws: MQTTCodecError.self) { try MQTTPacketDecoder.decode(bytes + [0xC0]) }
  }

  @Test("keeps reading after many packets have gone through it")
  func longStream() throws {
    let one = try publish.encode()
    var decoder = MQTTPacketDecoder()
    var count = 0
    // Enough bytes that the buffer is trimmed along the way; half a packet left over each
    // time, so the buffer is never empty when it is.
    let half = one.count / 2
    decoder.append(one[0..<half])
    for _ in 0..<10_000 {
      decoder.append(one[half...])
      decoder.append(one[0..<half])
      count += try drain(&decoder).count
    }
    #expect(count == 10_000)
    #expect(decoder.bufferedByteCount == half)
  }
}
