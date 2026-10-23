import Foundation
import ScentProtocol

/// The log's rows in words: each entry as a short sentence, such as "Turned Bedroom on, over
/// WiFi" or "Bedroom reported on, Medium, plugged in". Ported from
/// `apps/desktop/src/ui/log_summary.rs`, so the phone's log reads as the Mac's. Reads only
/// the entry's masked text: a sentence never quotes a payload that can carry a secret.
enum LogWords {
  static func intensity(_ level: Int) -> String {
    switch level {
    case 0: "–"
    case 1: "Low"
    case 2: "Medium"
    case 3: "High"
    default: String(level)
    }
  }

  static func viaName(_ via: Via) -> String {
    via == .bluetooth ? "Bluetooth" : "WiFi"
  }

  /// What the diffuser runs on: charging 2 or battery 200 is plugged in, charging 1 is
  /// charging, and otherwise a battery level is the battery's.
  static func powerSource(charging: Int?, battery: Int?) -> String? {
    switch (charging, battery) {
    case (2?, _), (_, 200?): "plugged in"
    case (1?, _): "charging"
    case (_, let percent?): "battery \(percent)%"
    case (0?, nil): "battery"
    default: nil
    }
  }

  /// The entry as a sentence about the diffuser called `label`.
  static func sentence(_ entry: LogEntry, label: String) -> String {
    switch (entry.direction, entry.via) {
    case (.toDevice, nil): failure(entry, label: label)
    case (.toDevice, let via?) where entry.sent: sent(entry, label: label, via: via)
    case (.toDevice, let via?): told(entry, label: label, via: via)
    case (.fromDevice, let via): heard(entry, label: label, via: via)
    }
  }

  private static func commandJSON(_ text: String) -> [String: Any]? {
    var frame = text.trimmingCharacters(in: .whitespacesAndNewlines)
    if frame.hasSuffix("-|") { frame.removeLast(2) }
    return (try? JSONSerialization.jsonObject(with: Data(frame.utf8))) as? [String: Any]
  }

  private static func number(_ json: [String: Any]?, _ keys: [String]) -> Int? {
    for part in ["payload", "data"] {
      guard let part = json?[part] as? [String: Any] else { continue }
      for key in keys {
        if let value = part[key] as? NSNumber { return value.intValue }
      }
    }
    return nil
  }

  private static func commandDone(_ action: String, _ json: [String: Any]?, _ label: String) -> String? {
    switch action {
    case "TURN_ON":
      guard let on = number(json, ["on", "enable"]) else { return nil }
      return on == 0 ? "Turned \(label) off" : "Turned \(label) on"
    case "SET_LEVEL":
      guard let level = number(json, ["intensity", "level"]) else { return nil }
      return "Set \(label) to \(intensity(level)) intensity"
    case "PING": return "Pinged \(label)"
    case "GET_STATUS": return "Asked \(label) for its status"
    case "SYNC_TIME": return "Told \(label) the time"
    case "SAVE_MQTT": return "Pointed \(label) at your broker"
    case "WIFI_CONNECT": return "Gave \(label) a WiFi network to join"
    case "SECRET_COMMAND", "RESET": return "Told \(label) to restart"
    case "UPDATE_PROFILE": return scheduleSent(json, label)
    default: return nil
    }
  }

  private static func scheduleSent(_ json: [String: Any]?, _ label: String) -> String {
    let payload = json?["payload"] as? [String: Any]
    let flag = (payload?["schedule_flag"] as? NSNumber)?.intValue
    let schedule = ((payload?["profile"] as? [String: Any])?["schedule"] as? [Any])?.count
    if flag == 0 { return "Sent \(label) its always-on setting" }
    guard let schedule else { return "Sent \(label) a schedule" }
    // The factory entry comes first; the rest are the times.
    switch max(schedule - 1, 0) {
    case 0: return "Sent \(label) a schedule with no times"
    case 1: return "Sent \(label) a schedule of 1 time"
    case let times: return "Sent \(label) a schedule of \(times) times"
    }
  }

  private static func sent(_ entry: LogEntry, label: String, via: Via) -> String {
    let done =
      commandDone(entry.action, commandJSON(entry.text), label) ?? "Sent \(entry.action) to \(label)"
    return "\(done), over \(viaName(via))"
  }

  /// A command on the diffuser's topic that this app didn't send.
  private static func told(_ entry: LogEntry, label: String, via: Via) -> String {
    let json = commandJSON(entry.text)
    let what: String? =
      switch entry.action {
      case "TURN_ON": number(json, ["on", "enable"]).map { $0 == 0 ? "turn off" : "turn on" }
      case "SET_LEVEL": number(json, ["intensity", "level"]).map { "set its intensity to \(intensity($0))" }
      case "PING": "answer a ping"
      case "GET_STATUS": "report its status"
      case "SYNC_TIME": "take the time"
      case "SAVE_MQTT": "use a new broker"
      case "WIFI_CONNECT": "join a WiFi network"
      case "SECRET_COMMAND", "RESET": "restart"
      case "UPDATE_PROFILE": "take a new schedule"
      default: nil
      }
    let over = viaName(via)
    if let what { return "\(label) was told to \(what), over \(over)" }
    return "\(label) was sent \(entry.action), over \(over)"
  }

  private static func statusWords(_ status: Status) -> String? {
    var parts: [String] = []
    switch status.power {
    case 0?: parts.append("off")
    case .some: parts.append("on")
    case nil: break
    }
    if status.power != 0, let level = status.intensity, (1...3).contains(level) {
      parts.append(intensity(level))
    }
    if let source = powerSource(charging: status.charging, battery: status.battery) {
      parts.append(source)
    }
    return parts.isEmpty ? nil : parts.joined(separator: ", ")
  }

  private static func reported(_ status: Status?, _ label: String) -> String {
    if let words = status.flatMap(statusWords) { return "\(label) reported \(words)" }
    return "\(label) sent a value this app doesn't use"
  }

  private static func heard(_ entry: LogEntry, label: String, via: Via?) -> String {
    let text = entry.text.trimmingCharacters(in: .whitespacesAndNewlines)
    switch entry.action {
    case "broker" where entry.deviceID == nil:
      return text.hasSuffix(".") ? String(text.dropLast()) : text
    case "Add": return "\(label) joined your broker"
    case "info": return identified(text, label)
    case "acknowledgement": return acknowledged(text, label)
    case "device_connected": return "\(label) connected to the broker"
    case "initial_load", "update_profile": return "\(label) sent its schedule"
    case sharedScheduleAction: return "Loaded \(label)'s schedule from the broker"
    case unreadableScheduleAction, otherVersionScheduleAction:
      if entry.deviceID == nil {
        return "Ignored a message under Scent Control's topics on the broker that it can't use"
      }
      let why =
        entry.action == otherVersionScheduleAction
        ? "another version of Scent Control stored it" : "it can't be read"
      return "Ignored \(label)'s schedule on the broker: \(why)"
    default: break
    }
    if text.lowercased().contains("credentials saved") { return "\(label) saved the broker login" }
    switch (via, entry.action) {
    case (.bluetooth?, "status"), (.bluetooth?, "value"), (.bluetooth?, "json"), (.bluetooth?, "unknown"):
      return reported(parseNotification(Data(text.utf8)).status, label)
    case (.wifi?, "update_status"), (.wifi?, "notification"):
      return reported(parseMqtt(topic: "", body: Data(text.utf8)).status, label)
    default:
      return "\(label) sent \(entry.action)"
    }
  }

  private static func identified(_ text: String, _ label: String) -> String {
    guard let info = try? parseInfo(text) else { return "\(label) sent its details" }
    let kind = DiffuserKind(model: info.modelName)
    let model = kind == .other ? info.modelName : kind.modelName
    let parts = [model.isEmpty ? nil : model, info.firmware.isEmpty ? nil : "firmware \(info.firmware)"]
      .compactMap { $0 }
    return parts.isEmpty ? "\(label) sent its details" : "\(label) identified itself: \(parts.joined(separator: ", "))"
  }

  private static func acknowledged(_ text: String, _ label: String) -> String {
    let json = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any]
    let msg = ((json?["payload"] as? [String: Any])?["msg"] as? String) ?? ""
    if msg.lowercased().contains("credentials saved") { return "\(label) saved the broker login" }
    switch msg.trimmingCharacters(in: .whitespaces) {
    case "BATTERY_MODE": return "\(label) switched to its battery"
    case "PLUGIN_MODE": return "\(label) was plugged in"
    default: break
    }
    let inner = (try? JSONSerialization.jsonObject(with: Data(msg.utf8))) as? [String: Any]
    switch inner?["result"] as? String {
    case "ADDED": return "\(label) confirmed it was added"
    case "ALREADY_ADDED": return "\(label) answered a ping"
    default: break
    }
    if inner?["data"] != nil { return reported(parseNotification(Data(msg.utf8)).status, label) }
    return "\(label) sent an acknowledgement"
  }

  /// A failed job's name without the route it names, and " over Bluetooth" (or nothing).
  private static func jobAndRoute(_ what: String) -> (name: String, over: String) {
    for (via, prefix) in [(Via.bluetooth, "Bluetooth "), (.wifi, "WiFi ")] where what.hasPrefix(prefix) {
      return (String(what.dropFirst(prefix.count)), " over \(viaName(via))")
    }
    return (what, "")
  }

  /// What failed for the open diffuser, for its room to say until the next send or report:
  /// "Couldn't send the power command over Bluetooth: No answer." Nil for the routine ping.
  static func failureNote(what: String, error: String) -> String? {
    let (name, over) = jobAndRoute(what)
    let error = error.trimmingCharacters(in: .whitespacesAndNewlines)
    let attempt: String
    switch name {
    case "PING": return nil
    case "TURN_ON": attempt = "send the power command"
    case "SET_LEVEL": attempt = "set the intensity"
    case "GET_STATUS", "read status": attempt = "read its status"
    case "SYNC_TIME": attempt = "tell it the time"
    case "SAVE_MQTT": attempt = "point it at your broker"
    case "WIFI_CONNECT": attempt = "give it the WiFi network"
    case "SECRET_COMMAND", "RESET": attempt = "restart it"
    case "UPDATE_PROFILE": attempt = "send its schedule"
    case "Schedule": attempt = "finish sending its schedule"
    case "Add": attempt = "add it"
    default: return error
    }
    return "Couldn't \(attempt)\(over): \(error)"
  }

  private static func attempt(_ name: String, _ label: String) -> String? {
    switch name {
    case "TURN_ON": "send the power command to \(label)"
    case "SET_LEVEL": "set \(label)'s intensity"
    case "PING": "ping \(label)"
    case "GET_STATUS", "read status": "read \(label)'s status"
    case "SYNC_TIME": "tell \(label) the time"
    case "SAVE_MQTT": "point \(label) at your broker"
    case "WIFI_CONNECT": "give \(label) a WiFi network"
    case "SECRET_COMMAND", "RESET": "restart \(label)"
    case "UPDATE_PROFILE": "send \(label) its schedule"
    case "Schedule": "finish sending \(label)'s schedule"
    case "Add": "add \(label)"
    default: nil
    }
  }

  private static func failure(_ entry: LogEntry, label: String) -> String {
    let error = entry.text.trimmingCharacters(in: .whitespacesAndNewlines)
    if entry.action == scanFailedAction { return "Couldn't scan for diffusers: \(error)" }
    let (name, over) = jobAndRoute(entry.action)
    let label = entry.deviceID == nil ? "the diffuser" : label
    if let attempt = attempt(name, label) { return "Couldn't \(attempt)\(over): \(error)" }
    return entry.deviceID == nil ? error : "\(label): \(error)"
  }
}
