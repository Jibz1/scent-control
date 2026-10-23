import Foundation
import ScentBluetooth
import ScentMQTT
import ScentProtocol

/// How the engine reaches the diffusers: Bluetooth and the broker. `LiveRadios` is the
/// real thing; the tests use a fake, since nothing they run may touch either. Mirrors the
/// Mac's `Transport` (`apps/desktop/src/transport.rs`).
@MainActor
protocol Radios: AnyObject {
  /// Where the radios report what they hear. Set once, by the engine.
  var report: (EngineEvent) -> Void { get set }
  func scan(seconds: TimeInterval) async throws(BluetoothError) -> [ScanHit]
  func readInfo(_ bleID: UUID) async throws(BluetoothError) -> String
  func bluetoothSend(_ bleID: UUID, _ command: Command) async throws(BluetoothError)
  func readStatus(_ bleID: UUID) async throws(BluetoothError)
  /// Publishes a command on the diffuser's topic. Refuses while the broker isn't connected.
  func wifiSend(_ deviceID: String, _ command: Command) async throws(MQTTError)
  /// Stores the retained copy of a diffuser's schedule under our own prefix.
  func storeSchedule(_ deviceID: String, _ body: Data) async throws(MQTTError)
  /// True while the broker has taken our login.
  var brokerUp: Bool { get }
  /// Drops any broker connection and makes a new one with these settings, unless there is
  /// no password.
  func reconnect(_ settings: BrokerSettings, password: String?)
  /// Lets go of Bluetooth links, as the app goes to the background.
  func idle()
}

/// The real radios: Core Bluetooth and the MQTT client.
@MainActor
final class LiveRadios: Radios {
  var report: (EngineEvent) -> Void = { _ in }
  private lazy var bluetooth = BluetoothCentral { [weak self] event in
    switch event {
    case .notification(let peripheral, let bytes, let at):
      self?.report(.bluetoothNotification(bleID: peripheral, bytes: bytes, at: Int(at.timeIntervalSince1970)))
    case .availability:
      break
    }
  }
  private var client: MQTTClient?
  private(set) var brokerUp = false
  /// Which connection the events come from, so a dropped one's late events are ignored.
  private var generation = 0

  func scan(seconds: TimeInterval) async throws(BluetoothError) -> [ScanHit] {
    try await bluetooth.scan(seconds: seconds)
  }

  func readInfo(_ bleID: UUID) async throws(BluetoothError) -> String {
    try await bluetooth.readInfo(bleID)
  }

  func bluetoothSend(_ bleID: UUID, _ command: Command) async throws(BluetoothError) {
    try await bluetooth.send(bleID, frame: command.bleFrame())
  }

  func readStatus(_ bleID: UUID) async throws(BluetoothError) {
    try await bluetooth.readStatus(bleID)
  }

  func wifiSend(_ deviceID: String, _ command: Command) async throws(MQTTError) {
    guard let client, brokerUp else { throw MQTTError(message: "Not connected to the broker.") }
    try await client.publish(topic: Topics.device(deviceID), payload: command.mqttBody())
  }

  func storeSchedule(_ deviceID: String, _ body: Data) async throws(MQTTError) {
    // A retained message on a diffuser's own topic would be replayed to it on every
    // connect, so only our own prefix is ever stored to.
    guard let topic = Topics.schedule(deviceID) else {
      throw MQTTError(message: "\(deviceID) is not a device ID.")
    }
    guard let client, brokerUp else { throw MQTTError(message: "Not connected to the broker.") }
    try await client.publish(topic: topic, payload: body, retain: true)
  }

  func reconnect(_ settings: BrokerSettings, password: String?) {
    generation += 1
    brokerUp = false
    if let old = client {
      Task { await old.stop() }
      client = nil
    }
    guard let password, let url = URL(string: settings.url), !settings.username.isEmpty else { return }
    let current = generation
    let options = MQTTOptions(
      url: url, username: settings.username, password: password,
      subscriptions: [Topics.subscribeAll, Topics.subscribeApp])
    let client = MQTTClient(options: options) { [weak self] event in
      Task { @MainActor in self?.heard(event, from: current) }
    }
    self.client = client
    report(.brokerConnecting)
    Task { await client.start() }
  }

  private func heard(_ event: MQTTEvent, from connection: Int) {
    guard connection == generation else { return }
    let now = Int(Date().timeIntervalSince1970)
    switch event {
    case .connected:
      brokerUp = true
      report(.brokerConnected)
    case .loginRefused(let why):
      brokerUp = false
      report(.brokerLoginRefused(why))
    case .disconnected(let why):
      brokerUp = false
      report(.brokerDisconnected(why))
    case .message(let topic, let payload, _):
      report(.mqtt(topic: topic, body: payload, at: now))
    }
  }

  func idle() {
    bluetooth.disconnectAll()
  }
}
