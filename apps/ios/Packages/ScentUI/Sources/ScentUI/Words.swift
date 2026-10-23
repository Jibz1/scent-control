import Foundation
import ScentEngine
import ScentProtocol

/// The words the screens use, in one place, so the list, the room and the log agree with each
/// other and with the Mac (`apps/desktop/src/ui/format.rs`). Nothing here reads the clock:
/// `now` is always passed in.
enum Words {
  static func level(_ level: Level) -> String {
    ["Low", "Medium", "High"][level.value - 1]
  }

  /// What it is doing, in one or two words, for the room's headline.
  static func state(_ d: DiffuserState) -> String {
    if d.pending != nil { return "Sending…" }
    if d.reachability == .notAnswering { return "Not answering" }
    switch d.power?.value {
    case true?: return "Diffusing"
    case false?: return "Off"
    case nil: return "Not reported yet"
    }
  }

  /// What it is doing, for a tile's second line: "Diffusing at Medium", "Off".
  static func summary(_ d: DiffuserState) -> String {
    if let pending = d.pending {
      if pending.stage == .waitingForDiffuser { return "Sent, waiting for it" }
      return "Sending…"
    }
    if d.reachability == .notAnswering { return "Not answering" }
    switch d.power?.value {
    case true?: return d.level.map { "Diffusing at \(level($0.value))" } ?? "Diffusing"
    case false?: return "Off"
    case nil: return "Not reported yet"
    }
  }

  /// The line under the room's headline: the intensity and how it last answered.
  static func detail(_ d: DiffuserState) -> String {
    var parts: [String] = []
    if let level = d.shownLevel, d.shownPower == true {
      parts.append("\(Words.level(level)) intensity")
    }
    parts.append(reachability(d.reachability))
    return parts.joined(separator: ", ").capitalizedFirst
  }

  static func reachability(_ r: Reachability) -> String {
    switch r {
    case .answering(let via): "answering over \(via.name)"
    case .waiting: "waiting for an answer"
    case .notAnswering: "not answering pings"
    case .unknown: "not pinged yet"
    }
  }

  /// The line the room shows while a change is on its way.
  static func pending(_ pending: Pending) -> String {
    switch pending.stage {
    case .sending(let via?): "Sending over \(via.name)…"
    case .sending(nil): "Sending…"
    case .waitingForDiffuser: "Sent, waiting for the diffuser"
    }
  }

  /// How long ago `at` was: "just now", "4 min ago", or from an hour on, the clock time.
  static func ago(_ at: Date, now: Date) -> String {
    let minutes = Int((now.timeIntervalSince(at) + 30) / 60)
    switch minutes {
    case ...0: return "just now"
    case 1..<60: return "\(minutes) min ago"
    default:
      let sameDay = Calendar.current.isDate(at, inSameDayAs: now)
      let time = at.formatted(date: .omitted, time: .shortened)
      return sameDay ? "at \(time)" : "\(at.formatted(.dateTime.weekday(.wide))) at \(time)"
    }
  }

  /// "Reported 3 min ago over WiFi".
  static func note<V>(_ reading: Reading<V>, now: Date) -> String {
    "Reported \(ago(reading.at, now: now)) over \(reading.via.name)"
  }

  /// A clock time from a schedule, in this phone's style: "7:30 PM" or "19:30".
  static func time(_ t: HhMm) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval(t.minutes * 60))
    var style = Date.FormatStyle(date: .omitted, time: .shortened)
    style.timeZone = TimeZone(identifier: "UTC")!
    return date.formatted(style)
  }

  /// A span of times: "6:30 – 8:00 AM" style, kept simple: "6:30 AM – 8:00 AM".
  static func span(_ start: HhMm, _ end: HhMm) -> String {
    "\(time(start)) – \(time(end))"
  }

  static let dayLetters = ["S", "M", "T", "W", "T", "F", "S"]
  static let dayNames = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]

  /// Days in words: "Every day", "Weekdays", "Weekends", or "Mon, Wed, Fri".
  static func days(_ weekdays: [Int]) -> String {
    let set = Set(weekdays)
    if set.count == 7 { return "Every day" }
    if set == [1, 2, 3, 4, 5] { return "Weekdays" }
    if set == [0, 6] { return "Weekends" }
    if set.isEmpty { return "No days" }
    return set.sorted().map { String(dayNames[$0].prefix(3)) }.joined(separator: ", ")
  }

  /// What the schedule says about now, for the room and the tiles: "On the schedule at
  /// Medium until 10:30 PM", "Off until 6:30 AM", or nil when no schedule is known.
  static func scheduleNow(_ profile: Profile?, now: Date) -> String? {
    guard let profile else { return nil }
    let utc = Int(now.timeIntervalSince1970)
    let expected = expectedState(profile, nowUTC: utc)
    if let until = expected.until, let level = expected.level {
      return "Scheduled at \(Words.level(level)) until \(time(until))"
    }
    if expected.power, let level = expected.level {
      return "Always on at \(Words.level(level))"
    }
    if let next = nextStart(profile, nowUTC: utc) {
      return next.today ? "Scheduled to start at \(time(next.at))" : "Next on \(dayNames[next.weekday]) at \(time(next.at))"
    }
    return "Nothing scheduled"
  }

  /// The schedule in a few words, for a tile: "Until 10:30 PM", "From 6:00 AM",
  /// "Friday at 6:00 AM".
  static func scheduleHint(_ profile: Profile?, now: Date) -> String? {
    guard let profile else { return nil }
    let utc = Int(now.timeIntervalSince1970)
    let expected = expectedState(profile, nowUTC: utc)
    if let until = expected.until { return "Scheduled until \(time(until))" }
    if expected.power { return "Always on" }
    guard let next = nextStart(profile, nowUTC: utc) else { return nil }
    return next.today ? "Scheduled from \(time(next.at))" : "Next \(dayNames[next.weekday]) \(time(next.at))"
  }

  /// The next enabled entry that starts after now, within a week.
  static func nextStart(_ profile: Profile, nowUTC: Int) -> (weekday: Int, at: HhMm, today: Bool)? {
    let (weekday, now) = localWeekdayAndTime(nowUTC: nowUTC, tzHours: profile.timezoneHours)
    for offset in 0..<8 {
      let day = (weekday + offset) % 7
      let starts = profile.entries
        .filter { $0.enable && $0.weekdays.contains(day) && (offset > 0 || $0.start > now) }
        .map(\.start)
        .sorted()
      if let first = starts.first { return (day, first, offset == 0) }
    }
    return nil
  }

  /// "UTC−5" or "UTC+5:30".
  static func offset(_ hours: Double) -> String {
    if hours == 0 { return "UTC" }
    let sign = hours < 0 ? "−" : "+"
    let whole = Int(abs(hours))
    let minutes = Int((abs(hours) - Double(whole)) * 60 + 0.5)
    return "UTC\(sign)\(whole)" + (minutes == 0 ? "" : String(format: ":%02d", minutes))
  }
}

extension String {
  var capitalizedFirst: String {
    prefix(1).uppercased() + dropFirst()
  }
}
