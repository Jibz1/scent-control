import ScentEngine
import ScentProtocol
import SwiftUI

/// A schedule being edited, before it is sent. Times are kept as hours and minutes; the
/// pickers show them in this phone's style.
struct ScheduleDraft: Equatable {
  struct Time: Identifiable, Equatable {
    let id = UUID()
    var start: HhMm
    var end: HhMm
    var weekdays: Set<Int>
    var level: Level
    var enable: Bool
  }

  var times: [Time]
  var alwaysOn: Bool
  var alwaysOnLevel: Level
  var timezoneHours: Double

  init(_ profile: Profile?, phoneOffset: Double) {
    times = (profile?.entries ?? []).map {
      Time(start: $0.start, end: $0.end, weekdays: Set($0.weekdays), level: $0.level, enable: $0.enable)
    }
    alwaysOn = profile?.alwaysOn.enable ?? false
    alwaysOnLevel = profile?.alwaysOn.level ?? .low
    timezoneHours = profile?.timezoneHours ?? phoneOffset
  }

  var profile: Profile {
    Profile(
      alwaysOn: AlwaysOn(enable: alwaysOn, level: alwaysOnLevel),
      entries: times.map {
        Entry(start: $0.start, end: $0.end, weekdays: $0.weekdays.sorted(), level: $0.level, enable: $0.enable)
      },
      timezoneHours: timezoneHours)
  }

  /// What is wrong with a time, in the Mac's words, or nil.
  static func problem(_ time: Time) -> String? {
    if time.weekdays.isEmpty { return "Pick at least one day." }
    if time.end <= time.start {
      return "The end must be after the start. A time can't run past midnight: split it in two."
    }
    return nil
  }

  /// How many times a firmware keeps: 7 up to 2.21, 100 after.
  static func limit(firmware: String?) -> Int {
    guard let firmware, let version = Double(firmware) else { return 100 }
    return version <= 2.21 ? 7 : 100
  }
}

/// Edits a diffuser's schedule and sends it, replacing the whole timetable on the diffuser.
struct ScheduleEditor: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let diffuser: DiffuserState
  @State private var draft: ScheduleDraft
  @State private var expanded: UUID?
  @State private var confirming = false

  init(diffuser: DiffuserState, expandFirst: Bool = false) {
    self.diffuser = diffuser
    let phoneOffset = Double(TimeZone.current.secondsFromGMT()) / 3600
    let draft = ScheduleDraft(diffuser.schedule, phoneOffset: phoneOffset)
    _draft = State(initialValue: draft)
    _expanded = State(initialValue: expandFirst ? draft.times.last?.id : nil)
  }

  private var limit: Int { ScheduleDraft.limit(firmware: diffuser.firmware) }
  private var hasProblems: Bool {
    draft.times.contains { ScheduleDraft.problem($0) != nil } || draft.times.count > limit
  }

  var body: some View {
    NavigationStack {
      Form {
        Section {
          WeekGrid(profile: draft.profile, now: model.now, trackHeight: 11)
            .padding(.vertical, 6)
            .animation(.smooth(duration: 0.35), value: draft)
        } footer: {
          Text("A preview of the week, so overlaps and gaps show before sending.")
        }

        Section {
          if draft.times.isEmpty {
            Text("No times yet. Without any, Always On says what the diffuser does.")
              .foregroundStyle(.secondary)
          }
          ForEach($draft.times) { $time in
            TimeRow(time: $time, expanded: expanded == time.id) {
              withAnimation(.snappy) { expanded = expanded == time.id ? nil : time.id }
            } remove: {
              withAnimation(.snappy) { draft.times.removeAll { $0.id == time.id } }
            }
          }
          Button {
            addTime()
          } label: {
            Label("Add Time", systemImage: "plus.circle.fill")
          }
          .disabled(draft.times.count >= limit)
        } header: {
          Text("Times")
        } footer: {
          if draft.times.count > limit {
            Text("This diffuser keeps at most \(limit) times. Remove some.").foregroundStyle(Theme.alert)
          }
        }

        Section {
          Toggle("Always On", isOn: $draft.alwaysOn.animation(.snappy))
          if draft.alwaysOn {
            Picker("Intensity", selection: $draft.alwaysOnLevel) {
              ForEach([Level.low, .medium, .high], id: \.self) { Text(Words.level($0)).tag($0) }
            }
            .pickerStyle(.segmented)
          }
        } footer: {
          Text("Runs whenever none of the times above does.")
        }

        Section {
          Stepper(value: $draft.timezoneHours, in: -12...14, step: 0.5) {
            LabeledContent("Time zone", value: Words.offset(draft.timezoneHours))
          }
        } footer: {
          Text(zoneNote)
        }
      }
      .navigationTitle("Schedule")
      .navigationSubtitle(diffuser.title)
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Cancel", systemImage: "xmark") { dismiss() }
        }
        ToolbarItem(placement: .confirmationAction) {
          Button("Send", systemImage: "paperplane.fill") { confirming = true }
            .buttonStyle(.glassProminent)
            .disabled(hasProblems)
        }
      }
      .confirmationDialog(
        "Replace \(diffuser.title)'s stored schedule?", isPresented: $confirming, titleVisibility: .visible
      ) {
        Button("Replace") {
          model.send(.sendSchedule(id: diffuser.id, draft.profile))
          dismiss()
        }
      } message: {
        Text("Its whole timetable is replaced with this one, then it is told the time.")
      }
    }
  }

  private var zoneNote: String {
    let phone = Double(TimeZone.current.secondsFromGMT(for: model.now)) / 3600
    return phone == draft.timezoneHours
      ? "The times are in this zone, which is this iPhone's."
      : "The times are in this zone. This iPhone is at \(Words.offset(phone)) now."
  }

  private func addTime() {
    let last = draft.times.last
    let start = last.map { HhMm(minutes: min($0.end.minutes + 60, 22 * 60)) } ?? HhMm(hour: 19, minute: 0)!
    let time = ScheduleDraft.Time(
      start: start, end: HhMm(minutes: min(start.minutes + 120, 23 * 60 + 59)),
      weekdays: Set(0..<7), level: .low, enable: true)
    withAnimation(.snappy) {
      draft.times.append(time)
      expanded = time.id
    }
  }
}

/// One time in the editor: a summary row that opens into its pickers.
private struct TimeRow: View {
  @Binding var time: ScheduleDraft.Time
  let expanded: Bool
  let toggle: () -> Void
  let remove: () -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      Button(action: toggle) {
        HStack(alignment: .center, spacing: 12) {
          VStack(alignment: .leading, spacing: 3) {
            Text(Words.span(time.start, time.end))
              .font(.body.weight(.semibold))
              .monospacedDigit()
              .foregroundStyle(time.enable ? .primary : .secondary)
            Text("\(Words.days(time.weekdays.sorted())), \(Words.level(time.level))")
              .font(.subheadline)
              .foregroundStyle(.secondary)
          }
          Spacer()
          Image(systemName: "chevron.down")
            .font(.footnote.weight(.semibold))
            .foregroundStyle(.tertiary)
            .rotationEffect(.degrees(expanded ? 180 : 0))
        }
        .contentShape(Rectangle())
      }
      .buttonStyle(.plain)

      if let problem = ScheduleDraft.problem(time) {
        Label(problem, systemImage: "exclamationmark.circle.fill")
          .font(.footnote)
          .foregroundStyle(Theme.alert)
      }

      if expanded {
        VStack(alignment: .leading, spacing: 14) {
          DatePicker("Starts", selection: clock(\.start), displayedComponents: .hourAndMinute)
          DatePicker("Ends", selection: clock(\.end), displayedComponents: .hourAndMinute)
          DayChips(weekdays: $time.weekdays)
          Picker("Intensity", selection: $time.level) {
            ForEach([Level.low, .medium, .high], id: \.self) { Text(Words.level($0)).tag($0) }
          }
          .pickerStyle(.segmented)
          HStack {
            Toggle("On", isOn: $time.enable).fixedSize()
            Spacer()
            Button("Remove", role: .destructive, action: remove)
              .buttonStyle(.bordered)
          }
        }
        .transition(.opacity.combined(with: .move(edge: .top)))
      }
    }
    .padding(.vertical, 4)
  }

  /// A binding to one of the time's ends as a date today, for the picker.
  private func clock(_ key: WritableKeyPath<ScheduleDraft.Time, HhMm>) -> Binding<Date> {
    Binding {
      Calendar.current.date(
        bySettingHour: time[keyPath: key].hour, minute: time[keyPath: key].minute, second: 0, of: .now) ?? .now
    } set: { date in
      let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
      if let value = HhMm(hour: parts.hour ?? 0, minute: parts.minute ?? 0) {
        time[keyPath: key] = value
      }
    }
  }
}

/// Seven round day toggles, from this phone's first weekday.
private struct DayChips: View {
  @Binding var weekdays: Set<Int>

  var body: some View {
    let first = Calendar.current.firstWeekday - 1
    HStack(spacing: 6) {
      ForEach(0..<7, id: \.self) { row in
        let day = (first + row) % 7
        let on = weekdays.contains(day)
        Button {
          if on { weekdays.remove(day) } else { weekdays.insert(day) }
        } label: {
          Text(Words.dayLetters[day])
            .font(.subheadline.weight(.semibold))
            .frame(maxWidth: .infinity, minHeight: 36)
            .foregroundStyle(on ? Color.white : Color.primary)
            .background(Circle().fill(on ? AnyShapeStyle(Theme.accent) : AnyShapeStyle(.quaternary)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Words.dayNames[day])
        .accessibilityAddTraits(on ? .isSelected : [])
      }
    }
    .animation(.snappy(duration: 0.2), value: weekdays)
    .sensoryFeedback(.selection, trigger: weekdays)
  }
}
