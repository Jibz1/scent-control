import ScentProtocol

// Which way a command goes to a diffuser, and when to give up on Bluetooth for a while.
// Ported from `apps/desktop/src/route.rs`. Pure: nothing here touches Bluetooth, the
// network or the clock.

/// How long Auto leaves Bluetooth alone after it failed to reach a diffuser, when WiFi is
/// there to take over. A failed connect takes about 13 s, so without this every command to
/// a diffuser that is out of range would wait that long before it went over WiFi.
let bluetoothRetryAfterSeconds = 120

extension SendOver {
  /// What is tried first: Bluetooth, unless WiFi is forced.
  var firstChoice: Via {
    self == .wifi ? .wifi : .bluetooth
  }
}

/// The transports to try, in order. `bluetooth` is whether Bluetooth is worth trying for
/// this diffuser (see `bluetoothWorthIt`); `wifi` is whether WiFi is a way to it (see
/// `wifiAvailable`). Auto is Bluetooth first, then WiFi; a forced transport has no
/// fallback.
func route(_ sendOver: SendOver, bluetooth: Bool, wifi: Bool) -> [Via] {
  switch sendOver {
  case .auto: (bluetooth ? [.bluetooth] : []) + (wifi ? [.wifi] : [])
  case .wifi: wifi ? [.wifi] : []
  case .bluetooth: bluetooth ? [.bluetooth] : []
  }
}

/// Whether the cool-down is over, so that Bluetooth is worth trying again: nothing has
/// failed, the diffuser was heard over Bluetooth after the failure (so it is in range
/// again), or `bluetoothRetryAfterSeconds` have passed.
func bluetoothWorthTrying(failedAt: Int?, heardBluetooth: Int?, now: Int) -> Bool {
  guard let failedAt else { return true }
  if let heardBluetooth, heardBluetooth > failedAt { return true }
  return now - failedAt >= bluetoothRetryAfterSeconds
}

/// Whether WiFi is a way to this diffuser. The broker must be up. A forced WiFi is the
/// person's call. For Auto the diffuser must also be on our broker (it has been heard
/// there), or have no Bluetooth link: a publish to the topic of one that isn't on our
/// broker would be accepted, reported as sent, and heard by nobody.
func wifiAvailable(_ sendOver: SendOver, brokerUp: Bool, onOurBroker: Bool, hasBluetooth: Bool) -> Bool {
  brokerUp && (sendOver == .wifi || onOurBroker || !hasBluetooth)
}

/// Whether to count Bluetooth as available. It needs a Bluetooth link. A forced Bluetooth
/// is always tried. Auto tries it unless the cool-down says to leave it alone, and only
/// when WiFi is there to fall back to: with no WiFi, skipping Bluetooth would turn a slow
/// command into a certain failure.
func bluetoothWorthIt(
  _ sendOver: SendOver, hasBluetooth: Bool, failedAt: Int?, heardBluetooth: Int?, wifi: Bool,
  now: Int
) -> Bool {
  hasBluetooth
    && (sendOver == .bluetooth || !wifi
      || bluetoothWorthTrying(failedAt: failedAt, heardBluetooth: heardBluetooth, now: now))
}

/// Why `route` came back empty.
func whyNoRoute(_ sendOver: SendOver) -> String {
  switch sendOver {
  case .wifi: "The broker isn't connected."
  case .bluetooth: "This diffuser hasn't been found over Bluetooth on this iPhone yet. Find Nearby links it."
  case .auto: "It hasn't been found over Bluetooth yet, and the broker isn't connected."
  }
}
