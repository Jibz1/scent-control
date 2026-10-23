import Foundation
import ScentProtocol

// Putting a diffuser on our broker, as a pure state machine. Ported from
// `apps/desktop/src/add_flow.rs`: read the info over Bluetooth; `SAVE_MQTT` with our
// broker, then 25 s for its test login; `WIFI_CONNECT` when the person gave a network,
// which they are asked for when the diffuser isn't on WiFi; a soft restart; then wait up
// to 90 s from the restart for its `device_connected` on our broker, and `PING` it with
// our route ID.

/// How long a diffuser has to show up on our broker after it was told to restart.
let waitConnectedSeconds = 90

/// How long to wait after `SAVE_MQTT` before the restart, always. The diffuser makes a test
/// login with the new credentials within about 20 s and announces itself on the broker as
/// it does; after an early restart that could pass for the restart having worked.
let credentialsWaitSeconds = 25

/// Why an add stops before it starts: `SAVE_MQTT` hands the diffuser our broker login.
let noBrokerPasswordToGive = "Save the broker password in Settings first."

/// Where an add is.
enum AddStep: Hashable, Sendable {
  /// Reading the info over Bluetooth, to learn its device ID and whether it is on WiFi.
  case readInfo
  /// It isn't on WiFi and no network was given: the person is asked for one.
  case needWifi
  case saveMqtt
  case wifiConnect
  case restart
  /// Waiting for its `device_connected` on our broker.
  case waitConnected
  case ping
  /// On our broker, with this device ID.
  case done(String)
  /// Stopped, and why, in words for the person.
  case failed(String)

  /// Whether the engine is working on this step, as against waiting for the person or
  /// having finished.
  var isRunning: Bool {
    switch self {
    case .needWifi, .done, .failed: false
    default: true
    }
  }
}

/// The step after `step`, given what the diffuser said about itself, whether the person
/// gave a network, and the device whose `device_connected` was heard since the restart.
func nextAddStep(
  _ step: AddStep, info: DeviceInfo?, wifiGiven: Bool, connectedDeviceID: String?
) -> AddStep {
  guard let info else { return step }
  switch step {
  case .readInfo: return wifiGiven || onWifi(info) ? .saveMqtt : .needWifi
  case .saveMqtt: return wifiGiven ? .wifiConnect : .restart
  case .wifiConnect: return .restart
  case .restart: return .waitConnected
  case .waitConnected: return connectedDeviceID == info.deviceID ? .ping : step
  case .ping: return .done(info.deviceID)
  case .needWifi, .done, .failed: return step
  }
}

/// Whether the diffuser is on WiFi: it has a network set up, and on firmware above 2.0
/// its WiFi status says connected (2 or 3).
func onWifi(_ info: DeviceInfo) -> Bool {
  let parts = info.firmware.split(separator: ".", omittingEmptySubsequences: false)
    .map { UInt32($0.trimmingCharacters(in: .whitespaces)) }
  let version = parts.contains(nil) ? nil : parts.compactMap { $0 }
  let statusKnown = version.map { [2, 0].lexicographicallyPrecedes($0) } ?? false
  return info.wifiConfigured && (!statusKnown || [2, 3].contains(info.wifiStatus))
}

/// How the wait for `device_connected` stands `waited` seconds after the restart.
func waitOutcome(waited: Int, connected: Bool) -> AddStep {
  if connected { return .ping }
  if waited < waitConnectedSeconds { return .waitConnected }
  return .failed(
    "The diffuser didn't join your broker within \(waitConnectedSeconds) s. Check the WiFi password, that the network is 2.4 GHz (the diffuser can't use 5 GHz), and that the diffuser is within range of the router, then try again."
  )
}
