// What diffusers send: Bluetooth notifications, the 0xFEF4 info string and MQTT messages,
// read as far as they can be. Ported from `packages/scent-core/src/message.rs`; both are
// tested against the captured messages in `fixtures/scentiment/`.

import Foundation

/// Status array positions. Indices 0-3 are a header; 7, 10 and 13 are read by nobody (7
/// is not the oil level, as once assumed).
private enum StatusIndex {
  static let power = 4
  static let charging = 5
  static let battery = 6
  static let fan = 8
  static let intensity = 9
  static let gasPump = 11
  static let light = 12
}

/// What a diffuser reported about itself. A field is `nil` when the message didn't carry
/// it.
public struct Status: Sendable, Hashable {
  public var power: Int?
  public var intensity: Int?
  public var charging: Int?
  public var battery: Int?
  public var fan: Int?
  public var gasPump: Int?
  public var light: Int?

  public init(
    power: Int? = nil, intensity: Int? = nil, charging: Int? = nil, battery: Int? = nil,
    fan: Int? = nil, gasPump: Int? = nil, light: Int? = nil
  ) {
    self.power = power
    self.intensity = intensity
    self.charging = charging
    self.battery = battery
    self.fan = fan
    self.gasPump = gasPump
    self.light = light
  }
}

/// The shape of a Bluetooth notification.
public enum NoteKind: Sendable, Hashable {
  /// `S:1,L:3,C:2,Bat:200`
  case statusText
  /// `{"data":[85,170,11,255,...]}`
  case statusArray
  /// One `key:value` pair, such as `enable:1` or `led:50`.
  case value
  /// Valid JSON that is not a status array.
  case json
  case unknown
}

/// A Bluetooth notification as the device sent it, plus what could be read from it.
public struct Notification: Sendable, Hashable {
  /// The text without the `-|` terminator or surrounding whitespace.
  public var raw: String
  public var kind: NoteKind
  public var status: Status

  public init(raw: String, kind: NoteKind, status: Status) {
    self.raw = raw
    self.kind = kind
    self.status = status
  }
}

/// Reads one Bluetooth notification. Never fails: anything unreadable is `.unknown`.
/// Bytes that aren't UTF-8 become U+FFFD, as in Rust's `from_utf8_lossy`.
public func parseNotification(_ bytes: Data) -> Notification {
  var text = trimmed(String(decoding: bytes, as: UTF8.self))
  if text.utf8.suffix(2).elementsEqual("-|".utf8) {
    text = trimmed(String(decoding: text.utf8.dropLast(2), as: UTF8.self))
  }
  let (kind, status) = classify(text)
  return Notification(raw: text, kind: kind, status: status)
}

private func classify(_ text: String) -> (NoteKind, Status) {
  if let first = text.utf8.first, first == UInt8(ascii: "{") || first == UInt8(ascii: "["),
    let json = JSON.parse(text)
  {
    if let array = json["data"]?.array {
      return (.statusArray, statusFromArray(array))
    }
    return (.json, Status())
  }
  if let status = statusFromText(text) { return (.statusText, status) }
  if let status = statusFromValue(text) { return (.value, status) }
  return (.unknown, Status())
}

private func statusFromArray(_ array: [JSON]) -> Status {
  func at(_ index: Int) -> Int? {
    index < array.count ? array[index].int64.map { Int($0) } : nil
  }
  return Status(
    power: at(StatusIndex.power), intensity: at(StatusIndex.intensity),
    charging: at(StatusIndex.charging), battery: at(StatusIndex.battery),
    fan: at(StatusIndex.fan), gasPump: at(StatusIndex.gasPump), light: at(StatusIndex.light))
}

/// `S:1,L:3,C:2,Bat:200`: at least two pairs, every key one we know.
private func statusFromText(_ text: String) -> Status? {
  var status = Status()
  var pairs = 0
  for part in split(text, on: ",") {
    guard let (key, value) = splitOnce(part, on: ":") else { return nil }
    switch Bytes(asciiLowercased(trimmed(key))) {
    case "s": status.power = number(value)
    case "l": status.intensity = number(value)
    case "c": status.charging = number(value)
    case "bat": status.battery = number(value)
    default: return nil
    }
    pairs += 1
  }
  return pairs > 1 ? status : nil
}

/// One `key:value` pair. Keys we don't map (like `led`) still make it a value.
private func statusFromValue(_ text: String) -> Status? {
  guard let (key, value) = splitOnce(text, on: ":") else { return nil }
  let name = asciiLowercased(trimmed(key))
  if name.isEmpty { return nil }
  var status = Status()
  switch Bytes(name) {
  case "enable": status.power = number(value)
  case "level": status.intensity = number(value)
  case "battery": status.battery = number(value)
  default: break
  }
  return status
}

private func number(_ text: String) -> Int? {
  parseInteger(trimmed(text), as: Int64.self).map { Int($0) }
}

/// What the 0xFEF4 info characteristic holds:
/// `manufacturer|model|variant|id|firmware|wifiConfigured|wifiStatus`.
public struct DeviceInfo: Sendable, Hashable {
  public var manufacturerID: String
  public var modelName: String
  public var variantName: String
  public var deviceID: String
  public var firmware: String
  public var wifiConfigured: Bool
  public var wifiStatus: UInt8

  public init(
    manufacturerID: String, modelName: String, variantName: String, deviceID: String,
    firmware: String, wifiConfigured: Bool, wifiStatus: UInt8
  ) {
    self.manufacturerID = manufacturerID
    self.modelName = modelName
    self.variantName = variantName
    self.deviceID = deviceID
    self.firmware = firmware
    self.wifiConfigured = wifiConfigured
    self.wifiStatus = wifiStatus
  }
}

/// Why the info string couldn't be read. The message is the Rust crate's, word for word.
public struct InfoError: Error, Sendable, Hashable, CustomStringConvertible, LocalizedError {
  public let message: String

  public init(_ message: String) { self.message = message }

  public var description: String { message }
  public var errorDescription: String? { message }
}

/// Reads the info string. Fields after the seventh are ignored.
public func parseInfo(_ text: String) throws(InfoError) -> DeviceInfo {
  let fields = split(trimmed(text), on: "|").map { trimmed($0) }
  guard fields.count >= 7 else {
    throw InfoError(
      "expected 7 '|'-separated fields in the 0xFEF4 info string, got \(fields.count): \(rustDebug(text))"
    )
  }
  guard let wifiStatus = parseInteger(fields[6], as: UInt8.self) else {
    throw InfoError(
      "the WiFi status field is not a number from 0 to 255: \(rustDebug(fields[6]))")
  }
  return DeviceInfo(
    manufacturerID: fields[0], modelName: fields[1], variantName: fields[2],
    deviceID: fields[3], firmware: fields[4],
    wifiConfigured: same(asciiLowercased(fields[5]), "true"), wifiStatus: wifiStatus)
}

/// Which way an MQTT message was going.
public enum Direction: Sendable, Hashable {
  /// Published to a `device/<id>` topic.
  case toDevice
  case fromDevice
}

/// The result of the first `PING` with a user ID.
public enum Ack: Sendable, Hashable {
  case added
  case alreadyAdded
  case other(String)
}

/// A diffuser announcing that it joined the broker.
public struct Connected: Sendable, Hashable {
  public var fw: String
  public var model: String
  public var variant: String
  public var manufacturer: String

  public init(fw: String, model: String, variant: String, manufacturer: String) {
    self.fw = fw
    self.model = model
    self.variant = variant
    self.manufacturer = manufacturer
  }
}

/// One MQTT message, read as far as it can be.
public struct MqttMessage: Sendable, Hashable {
  public var direction: Direction
  public var action: String?
  /// From the payload's `deviceId`, else from a `device/<id>` topic. Not checked with
  /// `Topics.validDeviceID`.
  public var deviceID: String?
  /// Set when the payload's `data` held something readable. It can be empty when the
  /// data parsed but had no status fields.
  public var status: Status?
  public var rssi: Int?
  /// Only for `acknowledgement` messages.
  public var ack: Ack?
  /// Only for `device_connected` messages.
  public var connected: Connected?
  /// False when the body was not valid JSON as received. The message may still have been
  /// repaired and read.
  public var validJSON: Bool
  /// The body as text, exactly as received (bad bytes replaced).
  public var raw: String

  public init(
    direction: Direction, action: String? = nil, deviceID: String? = nil,
    status: Status? = nil, rssi: Int? = nil, ack: Ack? = nil, connected: Connected? = nil,
    validJSON: Bool, raw: String
  ) {
    self.direction = direction
    self.action = action
    self.deviceID = deviceID
    self.status = status
    self.rssi = rssi
    self.ack = ack
    self.connected = connected
    self.validJSON = validJSON
    self.raw = raw
  }
}

/// Reads one MQTT message. Never fails: unreadable bodies give a message with nothing in
/// it but `raw` and `validJSON: false`.
///
/// Firmware 2.30 sends notifications with `"data":enable:0` (no quotes). That is repaired
/// by quoting the value and parsing again.
public func parseMqtt(topic: String, body: Data) -> MqttMessage {
  let raw = String(decoding: body, as: UTF8.self)
  let parsed = JSON.parse(raw)
  let topicDevice = stripPrefix(topic, Topics.devicePrefix)
  var message = MqttMessage(
    direction: topicDevice == nil ? .fromDevice : .toDevice, deviceID: topicDevice,
    validJSON: parsed != nil, raw: raw)
  let document = parsed ?? JSON.parse(quoteUnquotedData(Array(raw.utf8)))
  guard let root = document?.object else { return message }
  message.action = root["action"]?.string
  guard let payload = root["payload"]?.object else { return message }
  if let id = payload["deviceId"]?.string, !id.isEmpty {
    message.deviceID = id
  }
  message.rssi = payload["rssi"]?.int64.map { Int($0) }
  message.status = payload["data"].flatMap(statusFromData)
  switch message.action.map({ Bytes($0) }) {
  case "acknowledgement"?: message.ack = ack(from: payload)
  case "device_connected"?: message.connected = connected(from: payload)
  default: break
  }
  return message
}

/// `data` is Bluetooth-style text (`"S:1,L:3"`, a status array as a string) or, on some
/// firmware, the array's JSON object itself.
private func statusFromData(_ data: JSON) -> Status? {
  let note: Notification
  switch data {
  case .string(let text): note = parseNotification(Data(text.utf8))
  case .object: note = parseNotification(Data(data.bytes))
  default: return nil
  }
  return note.kind == .unknown ? nil : note.status
}

/// The acknowledgement's `msg` is itself JSON text: `{"user_id":"...","result":"ADDED"}`.
private func ack(from payload: JSONObject) -> Ack? {
  guard let msg = payload["msg"]?.string, let result = JSON.parse(msg)?["result"]?.string
  else { return nil }
  return switch Bytes(result) {
  case "ADDED": .added
  case "ALREADY_ADDED": .alreadyAdded
  default: .other(result)
  }
}

/// Fields the device left out come back empty: the connect itself is what matters.
private func connected(from payload: JSONObject) -> Connected {
  func field(_ key: String) -> String { payload[key]?.string ?? "" }
  return Connected(
    fw: field("fw"), model: field("model"), variant: field("variant"),
    manufacturer: field("manufacturer"))
}

private let dataKey = Array(#""data":"#.utf8)

/// Puts quotes around every `"data":` value that doesn't start with `"`, `{` or `[`. The
/// value runs to the next `,` or `}`. Meant for text that failed to parse as JSON; valid
/// JSON is better left alone.
func quoteUnquotedData(_ text: [UInt8]) -> [UInt8] {
  var out: [UInt8] = []
  out.reserveCapacity(text.count + 2)
  var rest = text[...]
  while let found = rest.firstRange(of: dataKey) {
    out += rest[..<found.upperBound]
    rest = rest[found.upperBound...]
    switch rest.first {
    case nil, UInt8(ascii: "\""), UInt8(ascii: "{"), UInt8(ascii: "["): continue
    default: break
    }
    let end =
      rest.firstIndex { $0 == UInt8(ascii: ",") || $0 == UInt8(ascii: "}") } ?? rest.endIndex
    out += JSON.string(trimmed(String(decoding: rest[..<end], as: UTF8.self))).bytes
    rest = rest[end...]
  }
  out += rest
  return out
}

/// `quoteUnquotedData` on text, for tests.
func quoteUnquotedData(_ text: String) -> String {
  String(decoding: quoteUnquotedData(Array(text.utf8)), as: UTF8.self)
}
