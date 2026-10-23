import Foundation

// The diffuser's GATT layout, and how a frame is split into writes. Matches
// `apps/desktop/src/ble.rs` and `docs/findings.md` (GATT layout, both models).

/// The diffuser's GATT service, which holds the three characteristics we use.
enum Gatt {
  /// Service `0x0180`.
  static let service = "00000180-0000-1000-8000-00805f9b34fb"

  /// How many bytes one write carries. Scentiment's app asks for an MTU of 180, so a
  /// write carries 177 bytes after the 3-byte header.
  static let chunkSize = 177

  /// The 16-bit form of a UUID on the Bluetooth base, as Core Bluetooth writes it.
  static func short(_ uuid: String) -> String {
    let start = uuid.index(uuid.startIndex, offsetBy: 4)
    return String(uuid[start..<uuid.index(start, offsetBy: 4)]).uppercased()
  }

  /// Splits a frame into the writes that carry it, in order, each at most `size` bytes.
  static func chunks(_ frame: Data, size: Int = chunkSize) -> [Data] {
    var parts: [Data] = []
    var start = frame.startIndex
    while start < frame.endIndex {
      let end = frame.index(start, offsetBy: size, limitedBy: frame.endIndex) ?? frame.endIndex
      parts.append(Data(frame[start..<end]))
      start = end
    }
    return parts
  }

  /// The first characteristic we use that discovery didn't find, in the order the Mac
  /// checks them, or `nil` when all three are there.
  static func firstMissing(from found: Set<GattCharacteristic>) -> GattCharacteristic? {
    GattCharacteristic.allCases.first { !found.contains($0) }
  }
}

/// The characteristics we use in the diffuser's service.
enum GattCharacteristic: CaseIterable, Hashable, Sendable {
  /// `0xDEAD`: frames are written here.
  case write
  /// `0xFEF3`: notifies when the state changes. Reading it returns the last value it sent.
  case status
  /// `0xFEF4`: reads the info string.
  case info

  var uuid: String {
    switch self {
    case .write: "0000dead-0000-1000-8000-00805f9b34fb"
    case .status: "0000fef3-0000-1000-8000-00805f9b34fb"
    case .info: "0000fef4-0000-1000-8000-00805f9b34fb"
    }
  }

  /// How errors name it, as `0xDEAD`.
  var label: String { "0x" + Gatt.short(uuid) }
}
