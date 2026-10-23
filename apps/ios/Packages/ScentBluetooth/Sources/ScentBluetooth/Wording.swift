import Foundation

// What the person is told when Bluetooth doesn't work. The Mac's words
// (`apps/desktop/src/ble.rs`), with "this Mac" and System Settings turned into the
// iPhone's. Nothing here ever includes a frame: frames can hold WiFi and broker passwords.

enum Wording {
  /// What to try when a diffuser can't be reached. A diffuser takes one Bluetooth
  /// connection at a time, so the usual cause is another app or device holding it.
  static let connectHint =
    "A diffuser takes one Bluetooth connection at a time: close the Scentiment app, and "
    + "Scent Control on any other phone or Mac, and check that the diffuser is on and within "
    + "a few meters."

  static let busy = "Bluetooth is busy with this diffuser. Try again in a moment."
  /// The Mac hub's words for a request Bluetooth stopped before answering.
  static let stopped = "Bluetooth stopped before it answered."
  static let cancelled = "The Bluetooth request was cancelled."
  static let ioTimeout = "The diffuser didn't answer within 10 s."
  /// Said after an error when a command may have reached the diffuser all the same.
  static let goneThroughAnyway = "The command may have gone through anyway."
  /// What a lost link is called when Bluetooth gives no reason.
  static let linkLost = "the Bluetooth link was lost."
  /// What a failure is called when Bluetooth gives no reason.
  static let noReason = "no reason was given."

  static let readInfoFailed = "Couldn't read the diffuser's info"
  static let readStatusFailed = "Couldn't read the diffuser's status"
  static let writeFailed = "Couldn't write to the diffuser"

  /// Whether Bluetooth in this state can be used, and if not, what the person can do.
  /// `nil` when it can.
  static func notReady(_ state: BluetoothAvailability) -> String? {
    switch state {
    case .poweredOn:
      nil
    case .poweredOff:
      "Bluetooth is turned off on this iPhone. Turn it on in Control Center or Settings, then "
        + "try again."
    case .unauthorized:
      "iOS has not allowed Scent Control to use Bluetooth. Allow it in Settings, Privacy & "
        + "Security, Bluetooth, then try again."
    case .unsupported:
      "Bluetooth isn't available on this device."
    case .unknown, .resetting:
      "Bluetooth isn't ready on this iPhone. If iOS asked, allow Scent Control to use "
        + "Bluetooth (Settings, Privacy & Security, Bluetooth)."
    }
  }

  /// A diffuser that couldn't be found or connected to in time. It gets the hint about
  /// other apps, which is the usual cause.
  static func unreachable(_ error: String) -> BluetoothError {
    BluetoothError(message: error + " " + connectHint)
  }

  /// A device that connected but isn't a diffuser we can talk to. No hint: closing the
  /// Scentiment app is no help with that.
  static func notADiffuser(_ missing: GattCharacteristic) -> BluetoothError {
    BluetoothError(
      message: "The device has no Bluetooth characteristic \(missing.label), so it is not a "
        + "diffuser we can talk to.")
  }

  /// How a step of opening a link ended, as an error, or `nil` when it worked. `activity`
  /// says what the step was doing, as in "connecting".
  static func opening(_ end: WaitEnd<Answer>, activity: String) -> BluetoothError? {
    switch end {
    case .value(.done):
      nil
    case .value(.failed(let reason)):
      unreachable("Bluetooth failed while \(activity): \(reason)")
    case .value(.dropped(let reason)):
      unreachable("Bluetooth failed while \(activity): \(reason ?? linkLost)")
    case .value(.stopped(let message)):
      BluetoothError(message: message)
    case .timedOut:
      unreachable("Timed out while \(activity).")
    case .cancelled:
      BluetoothError(message: cancelled)
    }
  }

  /// How a read or write on an open link ended, as an error, or `nil` when it worked.
  /// `failure` names what was being done, as in "Couldn't write to the diffuser". When
  /// `mayHaveGoneThrough` (the step was the write of a command's last part) an end that
  /// leaves that unknown is unanswered. A failure Bluetooth reported is not: the write was
  /// refused.
  static func exchange(_ end: WaitEnd<Answer>, failure: String, mayHaveGoneThrough: Bool)
    -> BluetoothError?
  {
    func unknown(_ message: String) -> BluetoothError {
      mayHaveGoneThrough
        ? BluetoothError(message: message + " " + Wording.goneThroughAnyway, unanswered: true)
        : BluetoothError(message: message)
    }
    switch end {
    case .value(.done):
      return nil
    case .value(.failed(let reason)):
      return BluetoothError(message: "\(failure): \(reason)")
    case .value(.dropped(let reason)):
      return unknown("\(failure): \(reason ?? linkLost)")
    case .value(.stopped(let message)):
      return unknown(message)
    case .timedOut:
      return unknown(ioTimeout)
    case .cancelled:
      return unknown(cancelled)
    }
  }
}
