// What the listener (apps/server) shares on the broker, as its readers decode it: each
// diffuser's record at `scent-control/v1/state/<deviceId>` and the listener's heartbeat at
// `scent-control/v1/listener` (spec 2026-10-22-listener-design, §6.4 and §6.5). Ported
// from `packages/scent-core/src/listener.rs`; the two must agree. The contract is
// `fixtures/scent-control/`.

import Foundation

/// A time further ahead of the reader's clock than this makes a message unusable.
public let dayAheadSeconds = 86_400

/// The readings a record can hold. Others are dropped, so that a later listener can add one
/// without breaking older readers.
public let fieldKeys = [
  "power", "intensity", "charging", "battery", "fan", "gas_pump", "light", "rssi",
]

private let listenerVersion: Int64 = 1

/// One value a diffuser reported, with when the listener heard it.
public struct Stamped: Hashable, Sendable {
  public var value: Int
  public var at: Int

  public init(value: Int, at: Int) {
    self.value = value
    self.at = at
  }
}

/// What a diffuser said about itself when it last joined the broker, and when.
public struct ConnectedAt: Hashable, Sendable {
  public var fw: String
  public var model: String
  public var variant: String
  public var manufacturer: String
  public var at: Int

  public init(fw: String, model: String, variant: String, manufacturer: String, at: Int) {
    self.fw = fw
    self.model = model
    self.variant = variant
    self.manufacturer = manufacturer
    self.at = at
  }
}

/// A diffuser's record as the listener publishes it. Times are seconds since 1970, on the
/// listener's clock.
public struct SharedState: Hashable, Sendable {
  public var updatedAt: Int
  public var heardAt: Int?
  public var pingAt: Int?
  /// By `fieldKeys` name.
  public var fields: [String: Stamped]
  public var connected: ConnectedAt?

  public init(
    updatedAt: Int, heardAt: Int? = nil, pingAt: Int? = nil, fields: [String: Stamped] = [:],
    connected: ConnectedAt? = nil
  ) {
    self.updatedAt = updatedAt
    self.heardAt = heardAt
    self.pingAt = pingAt
    self.fields = fields
    self.connected = connected
  }
}

/// The listener's heartbeat, or its will (`running: false`, no time).
public struct Heartbeat: Hashable, Sendable {
  public var running: Bool
  public var at: Int?

  public init(running: Bool, at: Int?) {
    self.running = running
    self.at = at
  }
}

/// Why a message can't be used.
public enum Unusable: Error, Hashable, Sendable {
  /// Another version's.
  case version
  /// Not JSON, or not the shape the version says.
  case invalid
  /// A time more than `dayAheadSeconds` ahead of the reader's clock.
  case ahead
}

private func jsonObject(_ body: Data) throws(Unusable) -> JSONObject {
  guard case .object(let object)? = JSON.parse(Array(body)) else { throw .invalid }
  return object
}

private func checkVersion(_ object: JSONObject) throws(Unusable) {
  guard let version = object["v"]?.int64 else { throw .invalid }
  guard version == listenerVersion else { throw .version }
}

/// An optional time: absent is fine, anything but an integer is not, nor one too far ahead.
private func time(_ value: JSON?, now: Int) throws(Unusable) -> Int? {
  guard let value else { return nil }
  guard let at = value.int64.map(Int.init) else { throw .invalid }
  let (limit, overflow) = now.addingReportingOverflow(dayAheadSeconds)
  if !overflow && at > limit { throw .ahead }
  return at
}

private func requiredTime(_ value: JSON?, now: Int) throws(Unusable) -> Int {
  guard let at = try time(value, now: now) else { throw .invalid }
  return at
}

private func text(_ object: JSONObject, _ key: String) throws(Unusable) -> String {
  guard let value = object[key]?.string else { throw .invalid }
  return value
}

private func connectedFrom(_ value: JSON, now: Int) throws(Unusable) -> ConnectedAt {
  guard let object = value.object else { throw .invalid }
  return try ConnectedAt(
    fw: text(object, "fw"), model: text(object, "model"), variant: text(object, "variant"),
    manufacturer: text(object, "manufacturer"), at: requiredTime(object["at"], now: now))
}

/// A diffuser's record, read back at `now`. Field keys it doesn't know are dropped; a known
/// one in the wrong shape makes the whole record unusable.
public func decodeState(_ body: Data, now: Int) -> Result<SharedState, Unusable> {
  Result { () throws(Unusable) in
    let object = try jsonObject(body)
    try checkVersion(object)
    let updatedAt = try requiredTime(object["updated_at"], now: now)
    let heardAt = try time(object["heard_at"], now: now)
    let pingAt = try time(object["ping_at"], now: now)
    guard let raw = object["fields"]?.object else { throw .invalid }
    var fields: [String: Stamped] = [:]
    for key in fieldKeys {
      guard let entry = raw[key] else { continue }
      guard let entry = entry.object, let value = entry["value"]?.int64 else { throw .invalid }
      let at = try requiredTime(entry["at"], now: now)
      fields[key] = Stamped(value: Int(value), at: at)
    }
    var connected: ConnectedAt?
    if let value = object["connected"] { connected = try connectedFrom(value, now: now) }
    return SharedState(
      updatedAt: updatedAt, heardAt: heardAt, pingAt: pingAt, fields: fields, connected: connected)
  }
}

/// The listener's heartbeat or will, read back at `now`.
public func decodeHeartbeat(_ body: Data, now: Int) -> Result<Heartbeat, Unusable> {
  Result { () throws(Unusable) in
    let object = try jsonObject(body)
    try checkVersion(object)
    guard case .bool(let running)? = object["running"] else { throw .invalid }
    return try Heartbeat(running: running, at: time(object["at"], now: now))
  }
}

/// Whether the listener is running: its last heartbeat said so within `onlineForSeconds`.
/// Otherwise what it shared may be out of date.
public func listenerRunning(_ heartbeat: Heartbeat?, now: Int) -> Bool {
  guard let heartbeat, heartbeat.running, let at = heartbeat.at else { return false }
  return saturatingSubtract(now, at) <= onlineForSeconds
}

/// Whether a reading taken at `readingAt` still counts: one from before the diffuser's last
/// reconnect does not (owner, 2026-10-22). One from the same second does.
public func isCurrent(readingAt: Int, connectedAt: Int?) -> Bool {
  connectedAt.map { readingAt >= $0 } ?? true
}
