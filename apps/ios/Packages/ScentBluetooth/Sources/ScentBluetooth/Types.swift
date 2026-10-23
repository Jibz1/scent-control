import Foundation

// What the Bluetooth layer hands to the app: scan results, events, and errors. These are
// plain values, so the engine can hold them, compare them and pass them between tasks.

/// A Scentiment diffuser found by a scan.
public struct ScanHit: Sendable, Hashable {
  /// The peripheral's identifier, which Core Bluetooth keeps for this app across launches.
  public let id: UUID
  /// The advertised name, such as `Scentiment-D B n 0A1B2C3D4E5F`.
  public let name: String
  /// The signal strength in dBm when it was last seen, if Bluetooth reported one.
  public let rssi: Int?

  public init(id: UUID, name: String, rssi: Int?) {
    self.id = id
    self.name = name
    self.rssi = rssi
  }
}

/// Whether Bluetooth can be used, as Core Bluetooth reports it.
public enum BluetoothAvailability: Sendable, Hashable {
  /// Not known yet. This is also the state before Bluetooth is first used, and the state
  /// the simulator stays in.
  case unknown
  /// Bluetooth is restarting.
  case resetting
  /// This device has no Bluetooth that can be used.
  case unsupported
  /// The person has not allowed the app to use Bluetooth.
  case unauthorized
  case poweredOff
  case poweredOn
}

/// What the Bluetooth layer reports without being asked.
public enum BluetoothEvent: Sendable {
  /// A value from `0xFEF3`: a notification, or the answer to `readStatus`. The bytes are
  /// as the diffuser sent them, terminator and all.
  case notification(peripheral: UUID, bytes: Data, at: Date)
  /// Bluetooth's state changed.
  case availability(BluetoothAvailability)
}

/// Why a Bluetooth request didn't work.
public struct BluetoothError: Error, Sendable, Equatable, LocalizedError {
  /// What went wrong, in words for the person.
  public let message: String
  /// True when a command ended without knowing whether the diffuser got it: the write of
  /// its last part went out but was never answered, because the time ran out, the link
  /// dropped, Bluetooth stopped, or the caller cancelled. The Mac calls this
  /// `BleError::Unanswered`. Sending it again over another way may repeat it.
  public let unanswered: Bool

  public init(message: String, unanswered: Bool = false) {
    self.message = message
    self.unanswered = unanswered
  }

  public var errorDescription: String? { message }
}
