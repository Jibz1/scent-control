// MQTT topics: Scentiment's, which the diffusers use and we keep, and our own. Ported
// from `packages/scent-core/src/topics.rs`.

public enum Topics {
  /// Commands to a diffuser are published to this prefix plus its device ID.
  public static let devicePrefix = "scentiment-mqtt-prod/device/"

  /// What the app subscribes to: everything under the prefix. Subscribe again on every
  /// ConnAck.
  public static let subscribeAll = "scentiment-mqtt-prod/#"

  /// Where Scent Control keeps what it stores on the broker for its other copies. Never
  /// under Scentiment's prefix: the diffusers subscribe there, and a retained message
  /// there would be replayed to a diffuser every time it connects.
  public static let appPrefix = "scent-control/v1/"

  /// What the app subscribes to besides `subscribeAll`: everything under our prefix.
  /// Subscribe again on every ConnAck.
  public static let subscribeApp = appPrefix + "#"

  /// The retained copy of the last schedule sent to a diffuser is kept at this prefix
  /// plus its device ID.
  public static let schedulePrefix = appPrefix + "schedule/"

  /// The listener's retained record of a diffuser is kept at this prefix plus its device ID.
  public static let statePrefix = appPrefix + "state/"

  /// The listener's retained heartbeat, and the will the broker publishes if it drops off.
  public static let listenerTopic = appPrefix + "listener"

  /// The topic to publish a command to. Check the ID with `validDeviceID` first; this
  /// does not, so an ID with `/`, `+` or `#` would build a different topic.
  public static func device(_ deviceID: String) -> String {
    devicePrefix + deviceID
  }

  /// The topic of a diffuser's stored schedule, or `nil` when `deviceID` is not a device
  /// ID, so that no ID can build a topic outside `schedulePrefix`.
  public static func schedule(_ deviceID: String) -> String? {
    validDeviceID(deviceID) ? schedulePrefix + deviceID : nil
  }

  /// The device ID a stored schedule's topic is for, or `nil` when the topic is not one.
  public static func scheduleDevice(_ topic: String) -> String? {
    stripPrefix(topic, schedulePrefix).flatMap { validDeviceID($0) ? $0 : nil }
  }

  /// The topic of a diffuser's record, or `nil` when `deviceID` is not a device ID.
  public static func state(_ deviceID: String) -> String? {
    validDeviceID(deviceID) ? statePrefix + deviceID : nil
  }

  /// The device ID a record's topic is for, or `nil` when the topic is not one.
  public static func stateDevice(_ topic: String) -> String? {
    stripPrefix(topic, statePrefix).flatMap { validDeviceID($0) ? $0 : nil }
  }

  /// A device ID is 12 characters, each an upper-case letter or a digit.
  public static func validDeviceID(_ deviceID: String) -> Bool {
    deviceID.utf8.count == 12
      && deviceID.utf8.allSatisfy {
        (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0)
          || (UInt8(ascii: "A")...UInt8(ascii: "Z")).contains($0)
      }
  }
}
