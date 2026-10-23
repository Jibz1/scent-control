import Foundation

// Which peripherals a scan reports. Matches `hit_for` in `apps/desktop/src/ble.rs` (a name
// that contains "scentiment", in any case) plus the design's RSSI floor (§6.2), which is
// also what Scentiment's app uses.

/// What a scan last heard from one peripheral.
struct Sighting: Equatable, Sendable {
  var name: String?
  var rssi: Int?

  /// Takes in a newer sighting. A name or signal strength it lacks keeps the earlier one:
  /// Bluetooth can report a peripheral before its name arrives.
  mutating func update(name: String?, rssi: Int?) {
    if let name { self.name = name }
    if let rssi { self.rssi = rssi }
  }
}

enum ScanFilter {
  /// The weakest signal that still counts, in dBm.
  static let rssiFloor = -90

  /// True when a Bluetooth name belongs to a Scentiment diffuser.
  static func isScentiment(_ name: String) -> Bool {
    name.lowercased().contains("scentiment")
  }

  /// A signal strength as Core Bluetooth reports it. It says 127 when it has none.
  static func rssi(reported value: Int) -> Int? {
    value == 127 ? nil : value
  }

  /// The scan result for a peripheral, when it is a Scentiment diffuser close enough to
  /// reach. One with no signal strength is kept.
  static func hit(_ id: UUID, _ sighting: Sighting) -> ScanHit? {
    guard let name = sighting.name, isScentiment(name) else { return nil }
    if let rssi = sighting.rssi, rssi < rssiFloor { return nil }
    return ScanHit(id: id, name: name, rssi: sighting.rssi)
  }

  /// The diffusers among everything seen, by name and then by identifier.
  static func hits(_ seen: [UUID: Sighting]) -> [ScanHit] {
    seen.compactMap { hit($0.key, $0.value) }.sorted {
      ($0.name, $0.id.uuidString) < ($1.name, $1.id.uuidString)
    }
  }
}
