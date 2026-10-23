// What the app may send to a diffuser, and the exact bytes it sends. Ported from
// `packages/scent-core/src/command.rs`; both are tested against
// `fixtures/commands/commands.json`, so the two write the same frames byte for byte.

import Foundation

/// Largest piece written to the Bluetooth characteristic in one go. Scentiment's app asks
/// for an MTU of 180, so a write carries 177 bytes after the 3-byte header.
public let bleChunk = 177

private let bleTerminator = "-|"
private let mask = "***"

/// Splits a Bluetooth frame into writes of at most `bleChunk` bytes.
public func chunks(_ frame: Data) -> [Data] {
  stride(from: frame.startIndex, to: frame.endIndex, by: bleChunk).map { start in
    Data(frame[start..<min(start + bleChunk, frame.endIndex)])
  }
}

/// How a command reaches the diffuser. Two commands differ between the two.
public enum Via: Hashable, Sendable {
  case bluetooth
  case wifi
}

/// A value that must never reach a log or a debug print. Printing, debug printing and
/// `dump` show `***`; only `expose()` gives the text back.
public struct Secret: Sendable, Hashable, CustomStringConvertible, CustomDebugStringConvertible,
  CustomReflectable
{
  private let value: String

  public init(_ value: String) { self.value = value }

  public func expose() -> String { value }

  public var description: String { mask }
  public var debugDescription: String { mask }
  public var customMirror: Mirror { Mirror(self, children: [:]) }
}

/// Everything the app may send to a diffuser, and nothing else. There is no case for
/// firmware updates, hard or factory resets, light commands or free-form JSON, so none of
/// those can be expressed, let alone sent.
public enum Command: Sendable, Hashable {
  case turnOn(Bool)
  case setLevel(Level)
  case ping(userid: String?)
  case getStatus
  case syncTime(serverTime: UInt64)
  case saveMqtt(broker: String, username: String, password: Secret)
  case wifiConnect(ssid: String, password: Secret)
  case softRestart
  /// Sets the schedule table, or only the always-on entry. Follow it with `syncTime`, as
  /// Scentiment's app does; the other order did not run on the Air 2.
  case updateProfile(Profile)

  /// The `action` field of the JSON envelope.
  public func action(via: Via) -> String {
    switch self {
    case .turnOn: "TURN_ON"
    case .setLevel: "SET_LEVEL"
    case .ping: "PING"
    case .getStatus: "GET_STATUS"
    case .syncTime: "SYNC_TIME"
    case .saveMqtt: "SAVE_MQTT"
    case .wifiConnect: "WIFI_CONNECT"
    case .softRestart: via == .bluetooth ? "SECRET_COMMAND" : "RESET"
    case .updateProfile: "UPDATE_PROFILE"
    }
  }

  /// The `payload` field of the JSON envelope as compact JSON, secrets included, exactly
  /// as the device needs it. `nil` when the command has no payload.
  public func payloadJSON(via: Via) -> String? {
    payload(via: via, revealSecrets: true)?.text
  }

  /// The full Bluetooth frame: compact JSON followed by `-|`. Split it with `chunks` to
  /// write it.
  public func bleFrame() -> Data {
    Data((envelope(via: .bluetooth, revealSecrets: true).text + bleTerminator).utf8)
  }

  /// The MQTT message body: compact JSON, no terminator.
  public func mqttBody() -> Data {
    Data(envelope(via: .wifi, revealSecrets: true).bytes)
  }

  /// What goes in a log: the frame for `via` with every password shown as `***`.
  /// Bluetooth keeps its `-|` terminator, as the Python tools' `redact_frame` does.
  public func redacted(via: Via) -> String {
    let text = envelope(via: via, revealSecrets: false).text
    return via == .bluetooth ? text + bleTerminator : text
  }

  private func envelope(via: Via, revealSecrets: Bool) -> JSON {
    var envelope: JSONObject = ["action": .string(action(via: via))]
    if let payload = payload(via: via, revealSecrets: revealSecrets) {
      envelope.set("payload", payload)
    }
    return .object(envelope)
  }

  /// Keys are written in the order the Python tools use, so frames match byte for byte.
  private func payload(via: Via, revealSecrets: Bool) -> JSON? {
    func shown(_ secret: Secret) -> JSON { .string(revealSecrets ? secret.expose() : mask) }
    return switch self {
    case .turnOn(let on): .object(["on": .int(on ? 1 : 0)])
    case .setLevel(let level): .object(["intensity": .int(Int64(level.value))])
    case .ping(userid: nil): nil
    case .ping(userid: let userid?): .object(["userid": .string(userid)])
    case .getStatus: .object([:])
    case .syncTime(let serverTime): .object(["serverTime": .uint(serverTime)])
    case .saveMqtt(let broker, let username, let password):
      .object([
        "broker": .string(broker), "username": .string(username), "password": shown(password),
      ])
    case .wifiConnect(let ssid, let password):
      .object(["ssid": .string(ssid), "password": shown(password)])
    case .softRestart:
      via == .bluetooth
        ? .object(["command": .string("restart")]) : .object(["mode": .string("soft")])
    case .updateProfile(let profile): profile.payload
    }
  }
}
