import ScentEngine
import ScentProtocol
import SwiftUI

/// One stretch of a day the schedule runs, in minutes from midnight.
struct DaySpan: Hashable {
  let start: Int
  let end: Int
  let level: Int
}

extension Profile {
  /// The enabled times that run on `weekday` (0 = Sunday).
  func spans(on weekday: Int) -> [DaySpan] {
    entries
      .filter { $0.enable && $0.weekdays.contains(weekday) }
      .map { DaySpan(start: $0.start.minutes, end: $0.end.minutes, level: $0.level.value) }
  }

  /// Today's weekday and the fraction of the day gone, in the schedule's time zone.
  func today(at now: Date) -> (weekday: Int, fraction: Double) {
    let (weekday, time) = localWeekdayAndTime(
      nowUTC: Int(now.timeIntervalSince1970), tzHours: timezoneHours)
    let seconds = Int(now.timeIntervalSince1970) % 60
    return (weekday, (Double(time.minutes) + Double(seconds) / 60) / 1440)
  }
}

/// How strongly a span is filled: Low is a wash, High is solid.
private func fill(_ level: Int) -> Double {
  [0.42, 0.7, 1][max(0, min(level - 1, 2))]
}

/// A day as a track: the times the schedule runs filled with the accent, deeper for higher
/// intensity, the always-on entry as a faint wash under them, and a marker for now.
struct DayTrack: View {
  let spans: [DaySpan]
  var alwaysOn: Int?
  var now: Double?
  var height: CGFloat = 10

  var body: some View {
    GeometryReader { proxy in
      let width = proxy.size.width
      ZStack(alignment: .leading) {
        Capsule().fill(.quaternary.opacity(0.7))
        if let alwaysOn {
          Capsule().fill(.tint.opacity(fill(alwaysOn) * 0.35))
        }
        ForEach(spans, id: \.self) { span in
          let x = width * CGFloat(span.start) / 1440
          let w = max(width * CGFloat(span.end - span.start) / 1440, height * 0.6)
          Capsule()
            .fill(.tint.opacity(fill(span.level)))
            .frame(width: w)
            .offset(x: x)
        }
        if let now {
          NowMarker(height: height)
            .offset(x: width * now - 1.5)
        }
      }
    }
    .frame(height: height)
  }
}

/// A short upright bar, taller than the track, where now is.
private struct NowMarker: View {
  let height: CGFloat

  var body: some View {
    Capsule()
      .fill(.primary)
      .frame(width: 3, height: height + 10)
      .overlay {
        Capsule().strokeBorder(Color(.systemBackground), lineWidth: 1)
      }
      .frame(height: height)
  }
}

/// Today, as a track across the card with the hours under it.
struct TodayStrip: View {
  let profile: Profile
  let now: Date

  var body: some View {
    let today = profile.today(at: now)
    VStack(spacing: 6) {
      DayTrack(
        spans: profile.spans(on: today.weekday),
        alwaysOn: profile.alwaysOn.enable ? profile.alwaysOn.level.value : nil,
        now: today.fraction, height: 14)
      HourLabels()
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(accessibilityText(today.weekday))
  }

  private func accessibilityText(_ weekday: Int) -> String {
    let spans = profile.spans(on: weekday)
    if spans.isEmpty { return "Nothing scheduled today" }
    return "Today: "
      + spans.map {
        "\(Words.time(HhMm(minutes: $0.start))) to \(Words.time(HhMm(minutes: $0.end)))"
      }.joined(separator: ", ")
  }
}

/// Midnight, 6, noon, 6 and midnight under a track, in this phone's clock style.
struct HourLabels: View {
  var body: some View {
    // Each label is centred on its hour, except the two ends, which sit inside the track.
    Color.clear
      .frame(height: 14)
      .overlay {
        GeometryReader { proxy in
          ForEach([0, 6, 12, 18, 24], id: \.self) { hour in
            Text(label(hour))
              .fixedSize()
              .frame(width: 80, alignment: alignment(hour))
              .position(x: x(hour, width: proxy.size.width), y: proxy.size.height / 2)
          }
        }
      }
    .font(.caption2)
    .foregroundStyle(.secondary)
    .monospacedDigit()
  }

  private func label(_ hour: Int) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval((hour % 24) * 3600))
    var style = Date.FormatStyle().hour(.defaultDigits(amPM: .abbreviated))
    style.timeZone = TimeZone(identifier: "UTC")!
    return date.formatted(style)
  }

  private func x(_ hour: Int, width: CGFloat) -> CGFloat {
    switch hour {
    case 0: 40
    case 24: width - 40
    default: width * CGFloat(hour) / 24
    }
  }

  private func alignment(_ hour: Int) -> Alignment {
    switch hour {
    case 0: .leading
    case 24: .trailing
    default: .center
    }
  }
}

/// The week, a track per day from this phone's first weekday, today's in bold with a marker
/// for now: the Mac's week preview, turned for a phone.
struct WeekGrid: View {
  let profile: Profile
  let now: Date
  var trackHeight: CGFloat = 9

  var body: some View {
    let today = profile.today(at: now)
    let first = Calendar.current.firstWeekday - 1
    VStack(spacing: 9) {
      ForEach(0..<7, id: \.self) { row in
        let day = (first + row) % 7
        HStack(spacing: 10) {
          Text(Words.dayLetters[day])
            .font(.caption.weight(day == today.weekday ? .heavy : .medium))
            .foregroundStyle(day == today.weekday ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
            .frame(width: 14)
          DayTrack(
            spans: profile.spans(on: day),
            alwaysOn: profile.alwaysOn.enable ? profile.alwaysOn.level.value : nil,
            now: day == today.weekday ? today.fraction : nil, height: trackHeight)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(dayText(day))
      }
      HourLabels().padding(.leading, 24)
    }
  }

  private func dayText(_ day: Int) -> String {
    let spans = profile.spans(on: day)
    let times = spans.map {
      "\(Words.time(HhMm(minutes: $0.start))) to \(Words.time(HhMm(minutes: $0.end)))"
    }
    return "\(Words.dayNames[day]): " + (times.isEmpty ? "nothing scheduled" : times.joined(separator: ", "))
  }
}
