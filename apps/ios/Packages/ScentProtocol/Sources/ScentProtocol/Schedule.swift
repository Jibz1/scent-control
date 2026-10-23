// Schedules: the timetable a diffuser keeps on board, what it implies at a given moment,
// and the `UPDATE_PROFILE` payload that puts it there. Ported from
// `packages/scent-core/src/schedule.rs`; the two must agree.

import Foundation

/// Diffuser intensity: 1 (low), 2 (medium) or 3 (high).
public struct Level: Hashable, Sendable, Comparable {
  public let value: Int

  /// `nil` for anything outside 1...3.
  public init?(_ value: Int) {
    guard (1...3).contains(value) else { return nil }
    self.value = value
  }

  public static let low = Level(1)!
  public static let medium = Level(2)!
  public static let high = Level(3)!

  public static func < (lhs: Level, rhs: Level) -> Bool { lhs.value < rhs.value }
}

/// A time of day on a 24-hour clock. Times order by the clock.
public struct HhMm: Hashable, Sendable, Comparable, CustomStringConvertible {
  public let hour: Int
  public let minute: Int

  /// `nil` unless the hour is below 24 and the minute below 60.
  public init?(hour: Int, minute: Int) {
    guard (0..<24).contains(hour), (0..<60).contains(minute) else { return nil }
    self.hour = hour
    self.minute = minute
  }

  /// Reads exactly `HH:MM`: two digits, a colon, two digits, a valid time.
  public init?(parsing text: String) {
    let bytes = Array(text.utf8)
    guard bytes.count == 5, bytes[2] == UInt8(ascii: ":") else { return nil }
    func digit(_ b: UInt8) -> Int? {
      (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(b) ? Int(b - UInt8(ascii: "0")) : nil
    }
    guard let h1 = digit(bytes[0]), let h2 = digit(bytes[1]), let m1 = digit(bytes[3]),
      let m2 = digit(bytes[4])
    else { return nil }
    self.init(hour: h1 * 10 + h2, minute: m1 * 10 + m2)
  }

  /// `minutes` must be below a day.
  public init(minutes: Int) {
    hour = minutes / 60
    minute = minutes % 60
  }

  public var minutes: Int { hour * 60 + minute }

  public var description: String {
    (hour < 10 ? "0" : "") + String(hour) + ":" + (minute < 10 ? "0" : "") + String(minute)
  }

  public static func < (lhs: HhMm, rhs: HhMm) -> Bool { lhs.minutes < rhs.minutes }
}

/// A timed run: on at `level` from `start` up to (not including) `end` on each of
/// `weekdays` (0 = Sunday ... 6 = Saturday). It must not cross midnight.
public struct Entry: Hashable, Sendable {
  public var start: HhMm
  public var end: HhMm
  public var weekdays: [Int]
  public var level: Level
  public var enable: Bool

  public init(start: HhMm, end: HhMm, weekdays: [Int], level: Level, enable: Bool = true) {
    self.start = start
    self.end = end
    self.weekdays = weekdays
    self.level = level
    self.enable = enable
  }
}

/// The fixed "factory" entry that covers the whole day and applies when no timed entry
/// does. The firmware can't edit or delete it, only switch it on or off.
public struct AlwaysOn: Hashable, Sendable {
  public var enable: Bool
  public var level: Level

  public init(enable: Bool, level: Level) {
    self.enable = enable
    self.level = level
  }
}

/// The `schedule_flag` of `UPDATE_PROFILE`.
public enum ScheduleFlag: Int, Sendable {
  /// Change only the always-on entry. The diffuser keeps the timed entries it has.
  case alwaysOnOnly = 0
  /// Replace the whole table.
  case replace = 1
}

/// A diffuser's schedule table and the time zone its times are in.
public struct Profile: Hashable, Sendable {
  public var alwaysOn: AlwaysOn
  public var entries: [Entry]
  /// Offset from UTC in hours, such as `-5.0` or `5.5`.
  public var timezoneHours: Double
  public var flag: ScheduleFlag

  public init(
    alwaysOn: AlwaysOn, entries: [Entry], timezoneHours: Double, flag: ScheduleFlag = .replace
  ) {
    self.alwaysOn = alwaysOn
    self.entries = entries
    self.timezoneHours = timezoneHours
    self.flag = flag
  }
}

// MARK: The UPDATE_PROFILE payload

private func workingState(_ level: Level) -> JSON {
  .object([
    "level": .int(Int64(level.value)), "fan": .int(0), "led": .int(0), "timer": .int(0),
  ])
}

extension AlwaysOn {
  var json: JSON {
    .object([
      "index": .int(1),
      "startTime": .string("00:00"),
      "endTime": .string("23:59"),
      "weekdays": .array((0...6).map { .int(Int64($0)) }),
      "workingState": workingState(level),
      "enable": .bool(enable),
      "isFactory": .bool(true),
    ])
  }
}

extension Entry {
  var json: JSON {
    .object([
      "startTime": .string(start.description),
      "endTime": .string(end.description),
      "weekdays": .array(weekdays.map { .int(Int64($0)) }),
      "workingState": workingState(level),
      "enable": .bool(enable),
    ])
  }
}

extension Profile {
  /// The payload of `UPDATE_PROFILE` as compact JSON, keys in the order the Rust crate and
  /// the Phase 0 tools write them. The factory entry comes first. The timed entries follow,
  /// but only for `.replace`: flag 0 sends the factory entry alone.
  public func payloadJSON() -> String { payload.text }

  var payload: JSON {
    var schedule = [alwaysOn.json]
    if flag == .replace { schedule += entries.map(\.json) }
    return .object([
      "profile": .object(["schedule": .array(schedule)]),
      "timezone": .double(timezoneHours),
      "schedule_flag": .int(Int64(flag.rawValue)),
    ])
  }
}

// MARK: What the schedule implies

/// What decides the expected state.
public enum Source: Hashable, Sendable {
  /// A timed entry that is running now.
  case entry(start: HhMm, end: HhMm)
  case alwaysOn
}

/// What a diffuser should be doing at a given moment, according to its schedule.
public struct Expected: Hashable, Sendable {
  public var power: Bool
  /// `nil` when it should be off.
  public var level: Level?
  public var source: Source
  /// When the running entry ends. `nil` for the always-on entry.
  public var until: HhMm?

  public init(power: Bool, level: Level?, source: Source, until: HhMm?) {
    self.power = power
    self.level = level
    self.source = source
    self.until = until
  }
}

private let secondsPerDay = 86_400

/// The weekday (0 = Sunday ... 6 = Saturday) and clock time at UTC offset `tzHours`, for a
/// UTC time in seconds since 1970. Seconds are dropped. Out-of-range offsets give a valid,
/// if meaningless, answer rather than a crash.
public func localWeekdayAndTime(nowUTC: Int, tzHours: Double) -> (weekday: Int, time: HhMm) {
  let (local, overflow) = nowUTC.addingReportingOverflow(saturatingInt((tzHours * 3600).rounded()))
  let saturated = overflow ? (nowUTC < 0 ? Int.min : Int.max) : local
  let days = floorDiv(saturated, secondsPerDay)
  let weekday = mod(days + 4, 7)  // 1970-01-01 was a Thursday
  let minutes = mod(saturated, secondsPerDay) / 60
  return (weekday, HhMm(minutes: minutes))
}

/// What the diffuser should be doing at `nowUTC` (seconds since 1970, UTC).
///
/// A timed entry is running when it is enabled, today's weekday in the diffuser's time
/// zone is one of its weekdays, and `start <= now < end`. The first running entry wins;
/// otherwise the always-on entry decides. The `flag` plays no part: it says what to send,
/// while the table in `entries` says what the diffuser holds.
public func expectedState(_ profile: Profile, nowUTC: Int) -> Expected {
  let (weekday, now) = localWeekdayAndTime(nowUTC: nowUTC, tzHours: profile.timezoneHours)
  if let running = profile.entries.first(where: {
    $0.enable && $0.weekdays.contains(weekday) && $0.start <= now && now < $0.end
  }) {
    return Expected(
      power: true, level: running.level, source: .entry(start: running.start, end: running.end),
      until: running.end)
  }
  return Expected(
    power: profile.alwaysOn.enable, level: profile.alwaysOn.enable ? profile.alwaysOn.level : nil,
    source: .alwaysOn, until: nil)
}

/// Why a test schedule can't be made.
public struct TrialError: Error, Sendable, Hashable, CustomStringConvertible, LocalizedError {
  public let message: String

  public init(_ message: String) { self.message = message }

  public var description: String { message }
  public var errorDescription: String? { message }
}

private let minutesPerDay = 24 * 60

/// A one-day test entry for `weekday` that starts `startInMinutes` after `now` and lasts
/// `durationMinutes`. It fails if it would run past 23:59, since an entry can't cross
/// midnight, or if either count is negative.
public func trialEntry(
  weekday: Int, now: HhMm, startInMinutes: Int, durationMinutes: Int, level: Level
) throws(TrialError) -> Entry {
  guard startInMinutes >= 0, durationMinutes >= 0 else {
    throw TrialError("the test schedule needs a start and a length of zero minutes or more")
  }
  // Capped at a day, which is already too long, so huge counts can't overflow.
  let start = now.minutes + min(startInMinutes, minutesPerDay)
  let end = start + min(durationMinutes, minutesPerDay)
  guard end < minutesPerDay else {
    throw TrialError(
      "the test schedule would cross midnight; pick a start time that ends before 23:59")
  }
  return Entry(
    start: HhMm(minutes: start), end: HhMm(minutes: end), weekdays: [weekday], level: level,
    enable: true)
}

/// Rust's `f64 as i64`: NaN is 0, and values out of range stop at the ends.
private func saturatingInt(_ value: Double) -> Int {
  if value.isNaN { return 0 }
  if value >= Double(Int.max) { return Int.max }
  if value <= Double(Int.min) { return Int.min }
  return Int(value)
}

private func floorDiv(_ a: Int, _ b: Int) -> Int {
  let q = a / b
  return (a % b != 0 && (a < 0) != (b < 0)) ? q - 1 : q
}

private func mod(_ a: Int, _ b: Int) -> Int {
  let r = a % b
  return r < 0 ? r + b : r
}
