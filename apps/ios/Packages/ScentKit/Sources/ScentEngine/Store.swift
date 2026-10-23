import Foundation
import ScentProtocol

// What the app keeps between launches, in the same shape as the Mac's `desktop.json`
// (`apps/desktop/src/store.rs`), so a file means the same on both: known diffusers, their
// names, rooms, readings and last schedule, and the broker's address. Never a password:
// the broker's is in the Keychain, and WiFi passwords are never kept anywhere.

/// A schedule as the editor shows it: times as `HH:MM`, days from 0 (Sunday) to 6, levels
/// from 1 to 3, and the zone's offset from UTC in hours. The copy shared on the broker holds
/// it in this form too, so the Mac and the phone read each other's.
struct StoredProfile: Codable, Equatable {
  struct StoredAlwaysOn: Codable, Equatable {
    var enable: Bool
    var level: Int
  }

  struct StoredEntry: Codable, Equatable {
    var start: String
    var end: String
    var weekdays: [Int]
    var level: Int
    var enable: Bool
  }

  var alwaysOn: StoredAlwaysOn
  var entries: [StoredEntry]
  var timezoneHours: Double
  /// What the last `UPDATE_PROFILE` sent: `"replace"` or `"always_on_only"`.
  var flag: String

  enum CodingKeys: String, CodingKey {
    case alwaysOn = "always_on"
    case entries
    case timezoneHours = "timezone_hours"
    case flag
  }

  init(_ profile: Profile) {
    alwaysOn = StoredAlwaysOn(enable: profile.alwaysOn.enable, level: profile.alwaysOn.level.value)
    entries = profile.entries.map {
      StoredEntry(
        start: $0.start.description, end: $0.end.description, weekdays: $0.weekdays,
        level: $0.level.value, enable: $0.enable)
    }
    timezoneHours = profile.timezoneHours
    flag = profile.flag == .replace ? "replace" : "always_on_only"
  }

  /// The schedule, or nil when any part of it isn't one a diffuser could hold.
  var profile: Profile? {
    var timed: [Entry] = []
    for entry in entries {
      guard entry.weekdays.allSatisfy({ (0...6).contains($0) }),
        let start = HhMm(parsing: entry.start), let end = HhMm(parsing: entry.end),
        let level = Level(entry.level)
      else { return nil }
      timed.append(Entry(start: start, end: end, weekdays: entry.weekdays, level: level, enable: entry.enable))
    }
    guard let level = Level(alwaysOn.level), timezoneHours.isFinite else { return nil }
    let flag: ScheduleFlag
    switch self.flag {
    case "replace": flag = .replace
    case "always_on_only": flag = .alwaysOnOnly
    default: return nil
    }
    return Profile(
      alwaysOn: AlwaysOn(enable: alwaysOn.enable, level: level), entries: timed,
      timezoneHours: timezoneHours, flag: flag)
  }
}

/// One saved diffuser.
struct StoredDevice: Codable, Equatable {
  struct StoredInfo: Codable, Equatable {
    var manufacturerID: String
    var modelName: String
    var variantName: String
    var deviceID: String
    var firmware: String
    var wifiConfigured: Bool
    var wifiStatus: UInt8

    enum CodingKeys: String, CodingKey {
      case manufacturerID = "manufacturer_id"
      case modelName = "model_name"
      case variantName = "variant_name"
      case deviceID = "device_id"
      case firmware
      case wifiConfigured = "wifi_configured"
      case wifiStatus = "wifi_status"
    }
  }

  struct StoredReading: Codable, Equatable {
    var value: Int
    var at: Int
    /// `"wifi"` or `"bluetooth"`.
    var via: String
  }

  var id: String
  var label: String
  var room: String?
  /// The Bluetooth peripheral's identifier on this phone. Each phone and Mac has its own.
  var bleID: String?
  var info: StoredInfo?
  var readings: [String: StoredReading]
  var sentProfile: StoredProfile?
  var sentProfileAt: Int?
  /// The phone's Send Over choice for it. The Mac keeps its own in memory only.
  var sendOver: String?

  enum CodingKeys: String, CodingKey {
    case id, label, room, info, readings
    case bleID = "ble_id"
    case sentProfile = "sent_profile"
    case sentProfileAt = "sent_profile_at"
    case sendOver = "send_over"
  }

  init(
    id: String, label: String, room: String?, bleID: String?, info: StoredInfo?,
    readings: [String: StoredReading], sentProfile: StoredProfile?, sentProfileAt: Int?,
    sendOver: String?
  ) {
    self.id = id
    self.label = label
    self.room = room
    self.bleID = bleID
    self.info = info
    self.readings = readings
    self.sentProfile = sentProfile
    self.sentProfileAt = sentProfileAt
    self.sendOver = sendOver
  }

  init(from decoder: any Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    id = try c.decode(String.self, forKey: .id)
    label = try c.decodeIfPresent(String.self, forKey: .label) ?? id
    room = try c.decodeIfPresent(String.self, forKey: .room)
    bleID = try c.decodeIfPresent(String.self, forKey: .bleID)
    info = try c.decodeIfPresent(StoredInfo.self, forKey: .info)
    readings = try c.decodeIfPresent([String: StoredReading].self, forKey: .readings) ?? [:]
    sentProfile = try c.decodeIfPresent(StoredProfile.self, forKey: .sentProfile)
    sentProfileAt = try c.decodeIfPresent(Int.self, forKey: .sentProfileAt)
    sendOver = try c.decodeIfPresent(String.self, forKey: .sendOver)
  }
}

/// The saved file.
struct StoredFile: Codable, Equatable {
  struct StoredBroker: Codable, Equatable {
    var url: String
    var username: String
    var routeID: String

    enum CodingKeys: String, CodingKey {
      case url, username
      case routeID = "route_id"
    }
  }

  struct StoredUI: Codable, Equatable {
    /// `"light"` or `"dark"`; missing means the system's.
    var appearance: String?
  }

  var devices: [StoredDevice]
  var broker: StoredBroker
  var ui: StoredUI?

  init(devices: [StoredDevice], broker: StoredBroker, ui: StoredUI?) {
    self.devices = devices
    self.broker = broker
    self.ui = ui
  }

  init(from decoder: any Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    devices = try c.decodeIfPresent([StoredDevice].self, forKey: .devices) ?? []
    broker =
      try c.decodeIfPresent(StoredBroker.self, forKey: .broker)
      ?? StoredBroker(url: "", username: "", routeID: "")
    ui = try c.decodeIfPresent(StoredUI.self, forKey: .ui)
  }
}

/// Reads and writes the saved file in a folder the app gives it: Application Support on
/// the phone, a temporary folder in tests.
struct Store: Sendable {
  let folder: URL

  var file: URL { folder.appendingPathComponent("state.json") }

  /// The saved file, or nil when there is none. A file that can't be read is set aside as
  /// `state.json.damaged` so the next save doesn't overwrite what might be recovered, and
  /// the error says so.
  func load() throws(StoreError) -> StoredFile? {
    guard FileManager.default.fileExists(atPath: file.path) else { return nil }
    do {
      let data = try Data(contentsOf: file)
      return try JSONDecoder().decode(StoredFile.self, from: data)
    } catch {
      let aside = folder.appendingPathComponent("state.json.damaged")
      try? FileManager.default.removeItem(at: aside)
      try? FileManager.default.moveItem(at: file, to: aside)
      throw StoreError(
        message:
          "The saved diffusers couldn't be read, so the app started without them. The file was kept as state.json.damaged."
      )
    }
  }

  /// Writes the file atomically.
  func save(_ stored: StoredFile) throws(StoreError) {
    do {
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      let encoder = JSONEncoder()
      encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
      try encoder.encode(stored).write(to: file, options: .atomic)
    } catch {
      throw StoreError(message: "The diffusers couldn't be saved: \(error.localizedDescription)")
    }
  }
}

struct StoreError: Error, Equatable {
  let message: String
}
