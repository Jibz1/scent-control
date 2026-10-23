import CoreBluetooth
import Foundation

// The real `Radio`: Core Bluetooth, with nothing decided here. Requests go straight to
// `CBCentralManager` and `CBPeripheral`, and their delegate callbacks come back as
// `RadioEvent`s.
//
// The central manager is created on the main queue, so every delegate callback arrives on
// the main thread. The delegate conformances are isolated to the main actor, which makes
// that explicit: the compiler holds this class's state to the main actor, and the runtime
// checks each callback is on it.

@MainActor final class CoreBluetoothRadio: NSObject, Radio {
  var onEvent: (@MainActor (RadioEvent) -> Void)?

  private var central: CBCentralManager?
  /// The peripherals this launch has, by identifier. Core Bluetooth drops a peripheral
  /// that nothing holds, along with any connection to it.
  private var peripherals: [UUID: CBPeripheral] = [:]

  private static let service = CBUUID(string: Gatt.short(Gatt.service))
  private static let characteristicIDs = GattCharacteristic.allCases.map {
    CBUUID(string: Gatt.short($0.uuid))
  }

  var isStarted: Bool { central != nil }

  var state: BluetoothAvailability {
    central.map { Self.availability($0.state) } ?? .unknown
  }

  var authorizationPending: Bool {
    #if targetEnvironment(simulator)
      // The simulator has no Bluetooth and never asks, so there is nothing to wait for.
      false
    #else
      CBManager.authorization == .notDetermined
    #endif
  }

  func start() {
    guard central == nil else { return }
    central = CBCentralManager(delegate: self, queue: .main)
  }

  func startScan() {
    guard let central = poweredOn() else { return }
    // The diffusers advertise their name only, with no service UUIDs to filter on.
    central.scanForPeripherals(withServices: nil)
  }

  func stopScan() {
    poweredOn()?.stopScan()
  }

  func retrieve(_ id: UUID) -> Bool {
    guard let central = poweredOn(),
      let found = central.retrievePeripherals(withIdentifiers: [id]).first(where: {
        $0.identifier == id
      })
    else { return false }
    keep(found)
    return true
  }

  func isKnown(_ id: UUID) -> Bool { peripherals[id] != nil }

  func connection(_ id: UUID) -> PeripheralConnection {
    switch peripherals[id]?.state {
    case .connecting: .connecting
    case .connected: .connected
    case .disconnecting: .disconnecting
    default: .disconnected
    }
  }

  func connect(_ id: UUID) {
    guard let central = poweredOn(), let peripheral = peripherals[id] else {
      emit(.failedToConnect(id, RadioFailure("Bluetooth doesn't know this diffuser.")))
      return
    }
    central.connect(peripheral)
  }

  func cancelConnection(_ id: UUID) {
    guard let central = poweredOn(), let peripheral = peripherals[id] else { return }
    central.cancelPeripheralConnection(peripheral)
  }

  func discoverServices(_ id: UUID) {
    guard let peripheral = connected(id) else {
      emit(.discoveredServices(id, Self.notConnected))
      return
    }
    peripheral.discoverServices([Self.service])
  }

  func discoverCharacteristics(_ id: UUID) {
    guard let peripheral = connected(id) else {
      emit(.discoveredCharacteristics(id, Self.notConnected))
      return
    }
    guard let service = Self.service(of: peripheral) else {
      // Nothing to look in: the check that follows names what is missing.
      emit(.discoveredCharacteristics(id, nil))
      return
    }
    peripheral.discoverCharacteristics(Self.characteristicIDs, for: service)
  }

  func characteristics(_ id: UUID) -> Set<GattCharacteristic> {
    guard let peripheral = peripherals[id], let service = Self.service(of: peripheral) else {
      return []
    }
    return Set((service.characteristics ?? []).compactMap { Self.gatt($0.uuid) })
  }

  func subscribe(_ id: UUID) {
    guard let (peripheral, characteristic) = find(id, .status) else {
      emit(.subscribed(id, Self.notConnected))
      return
    }
    peripheral.setNotifyValue(true, for: characteristic)
  }

  func read(_ id: UUID, _ which: GattCharacteristic) {
    guard let (peripheral, characteristic) = find(id, which) else {
      emit(.valueUpdated(id, which, nil, Self.notConnected))
      return
    }
    peripheral.readValue(for: characteristic)
  }

  func write(_ id: UUID, _ chunk: Data) {
    guard let (peripheral, characteristic) = find(id, .write) else {
      emit(.wrote(id, Self.notConnected))
      return
    }
    peripheral.writeValue(chunk, for: characteristic, type: .withResponse)
  }

  // MARK: Mapping, kept pure for tests

  nonisolated static func availability(_ state: CBManagerState) -> BluetoothAvailability {
    switch state {
    case .resetting: .resetting
    case .unsupported: .unsupported
    case .unauthorized: .unauthorized
    case .poweredOff: .poweredOff
    case .poweredOn: .poweredOn
    default: .unknown
    }
  }

  /// Which of our characteristics a UUID is, if any. Compared with `==`, which takes a
  /// UUID on the Bluetooth base to equal its 16-bit form.
  nonisolated static func gatt(_ uuid: CBUUID) -> GattCharacteristic? {
    GattCharacteristic.allCases.first { CBUUID(string: Gatt.short($0.uuid)) == uuid }
  }

  /// Bluetooth's error in our terms. Errors that mean the link went down are marked so,
  /// because a write that ends with one may still have reached the diffuser.
  nonisolated static func failure(_ error: (any Error)?) -> RadioFailure? {
    guard let error else { return nil }
    let lost: Set<CBError.Code> = [
      .notConnected, .connectionTimeout, .peripheralDisconnected, .connectionFailed,
      .operationCancelled,
    ]
    let nsError = error as NSError
    let linkLost =
      nsError.domain == CBErrorDomain
      && CBError.Code(rawValue: nsError.code).map(lost.contains)
        == true
    return RadioFailure(error.localizedDescription, linkLost: linkLost)
  }

  // MARK: Helpers

  /// For a request that couldn't be made, so never reached the diffuser.
  private static let notConnected = RadioFailure("The diffuser isn't connected.")

  private func emit(_ event: RadioEvent) {
    onEvent?(event)
  }

  private func poweredOn() -> CBCentralManager? {
    guard let central, central.state == .poweredOn else { return nil }
    return central
  }

  private func keep(_ peripheral: CBPeripheral) {
    guard peripherals[peripheral.identifier] == nil else { return }
    peripheral.delegate = self
    peripherals[peripheral.identifier] = peripheral
  }

  private func connected(_ id: UUID) -> CBPeripheral? {
    guard let peripheral = peripherals[id], peripheral.state == .connected else { return nil }
    return peripheral
  }

  private func find(_ id: UUID, _ which: GattCharacteristic) -> (CBPeripheral, CBCharacteristic)? {
    guard let peripheral = connected(id), let service = Self.service(of: peripheral),
      let characteristic = service.characteristics?.first(where: { Self.gatt($0.uuid) == which })
    else { return nil }
    return (peripheral, characteristic)
  }

  private static func service(of peripheral: CBPeripheral) -> CBService? {
    peripheral.services?.first { $0.uuid == service }
  }
}

extension CoreBluetoothRadio: @MainActor CBCentralManagerDelegate {
  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    let state = Self.availability(central.state)
    // Below powered off, every peripheral this manager handed out is no longer valid.
    if state != .poweredOn && state != .poweredOff {
      peripherals.removeAll()
    }
    emit(.stateChanged(state))
  }

  func centralManager(
    _ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
    advertisementData: [String: Any], rssi: NSNumber
  ) {
    keep(peripheral)
    let advertised = advertisementData[CBAdvertisementDataLocalNameKey] as? String
    emit(
      .discovered(
        peripheral.identifier, name: advertised ?? peripheral.name,
        rssi: ScanFilter.rssi(reported: rssi.intValue)))
  }

  func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
    emit(.connected(peripheral.identifier))
  }

  func centralManager(
    _ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: (any Error)?
  ) {
    emit(.failedToConnect(peripheral.identifier, Self.failure(error)))
  }

  func centralManager(
    _ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral,
    error: (any Error)?
  ) {
    emit(.disconnected(peripheral.identifier, Self.failure(error)))
  }
}

extension CoreBluetoothRadio: @MainActor CBPeripheralDelegate {
  func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: (any Error)?) {
    emit(.discoveredServices(peripheral.identifier, Self.failure(error)))
  }

  func peripheral(
    _ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService,
    error: (any Error)?
  ) {
    guard service.uuid == Self.service else { return }
    emit(.discoveredCharacteristics(peripheral.identifier, Self.failure(error)))
  }

  func peripheral(
    _ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic,
    error: (any Error)?
  ) {
    guard Self.gatt(characteristic.uuid) == .status else { return }
    emit(.subscribed(peripheral.identifier, Self.failure(error)))
  }

  func peripheral(
    _ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic,
    error: (any Error)?
  ) {
    guard let which = Self.gatt(characteristic.uuid) else { return }
    emit(.valueUpdated(peripheral.identifier, which, characteristic.value, Self.failure(error)))
  }

  func peripheral(
    _ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic,
    error: (any Error)?
  ) {
    guard Self.gatt(characteristic.uuid) == .write else { return }
    emit(.wrote(peripheral.identifier, Self.failure(error)))
  }
}
