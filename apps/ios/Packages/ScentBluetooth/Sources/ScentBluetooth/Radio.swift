import Foundation

// The seam between the Bluetooth layer's logic and Core Bluetooth. `CoreBluetoothRadio`
// is the real one, kept thin; tests use a fake. Peripherals are named by identifier, and
// every answer comes back as a `RadioEvent`, as Core Bluetooth's delegate callbacks do.

/// Where a peripheral's connection stands.
enum PeripheralConnection: Sendable, Equatable {
  case disconnected
  case connecting
  case connected
  case disconnecting
}

/// Why Bluetooth says a request failed.
struct RadioFailure: Sendable, Equatable {
  /// Bluetooth's own words.
  let reason: String
  /// True when the cause is the link going down rather than a refusal, so a write that
  /// failed this way may still have reached the diffuser.
  let linkLost: Bool

  init(_ reason: String, linkLost: Bool = false) {
    self.reason = reason
    self.linkLost = linkLost
  }
}

/// What Bluetooth reports, one delegate callback each.
enum RadioEvent: Sendable, Equatable {
  case stateChanged(BluetoothAvailability)
  /// A scan heard a peripheral. The name is its advertised name, or the name Bluetooth
  /// knows it by.
  case discovered(UUID, name: String?, rssi: Int?)
  case connected(UUID)
  case failedToConnect(UUID, RadioFailure?)
  /// The link went down. No failure means this app asked for it.
  case disconnected(UUID, RadioFailure?)
  case discoveredServices(UUID, RadioFailure?)
  case discoveredCharacteristics(UUID, RadioFailure?)
  case subscribed(UUID, RadioFailure?)
  /// A value read from a characteristic, or notified by it: Bluetooth doesn't tell the
  /// two apart.
  case valueUpdated(UUID, GattCharacteristic, Data?, RadioFailure?)
  case wrote(UUID, RadioFailure?)
}

/// Core Bluetooth, as the Bluetooth layer uses it. Nothing here waits: each request is
/// answered later through `onEvent`, on the main actor.
@MainActor protocol Radio: AnyObject {
  /// Where the answers go. Set once, before anything else.
  var onEvent: (@MainActor (RadioEvent) -> Void)? { get set }
  /// Whether `start` has been called.
  var isStarted: Bool { get }
  /// Bluetooth's state now; unknown until started.
  var state: BluetoothAvailability { get }
  /// True while iOS has not asked the person about Bluetooth yet, or is asking now.
  var authorizationPending: Bool { get }

  /// Sets up Bluetooth. Until this is called nothing touches it, so iOS asks for
  /// permission only when Bluetooth is first needed.
  func start()
  func startScan()
  func stopScan()
  /// Asks Bluetooth for a peripheral it remembers, from this launch or an earlier one,
  /// with no scan. True when it has it; it is then known.
  func retrieve(_ id: UUID) -> Bool
  /// Whether this launch already has the peripheral, from a scan or a retrieval.
  func isKnown(_ id: UUID) -> Bool
  func connection(_ id: UUID) -> PeripheralConnection
  func connect(_ id: UUID)
  /// Ends a connection, or one being made.
  func cancelConnection(_ id: UUID)
  /// Looks for the diffuser's service.
  func discoverServices(_ id: UUID)
  /// Looks for our characteristics in the diffuser's service.
  func discoverCharacteristics(_ id: UUID)
  /// Which of our characteristics the diffuser's service has.
  func characteristics(_ id: UUID) -> Set<GattCharacteristic>
  /// Turns on notifications from `0xFEF3`.
  func subscribe(_ id: UUID)
  func read(_ id: UUID, _ characteristic: GattCharacteristic)
  /// Writes one part of a frame to `0xDEAD`, with response.
  func write(_ id: UUID, _ chunk: Data)
}
