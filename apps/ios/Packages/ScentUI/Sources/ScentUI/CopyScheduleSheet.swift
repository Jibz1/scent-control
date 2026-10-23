import ScentEngine
import ScentProtocol
import SwiftUI

/// Copies one diffuser's last-sent schedule to others. Each target's whole timetable is
/// replaced, then it is told the time, exactly as Send Schedule does; each row ticks once
/// the broker has taken its copy.
struct CopyScheduleSheet: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let source: DiffuserState
  @State private var chosen: Set<String>
  @State private var sent = false

  init(source: DiffuserState, chosen: Set<String> = []) {
    self.source = source
    _chosen = State(initialValue: chosen)
  }

  private var targets: [DiffuserState] {
    model.diffusers.filter { $0.id != source.id }
  }

  var body: some View {
    NavigationStack {
      ScrollView {
        VStack(alignment: .leading, spacing: 26) {
          if let profile = source.schedule {
            VStack(alignment: .leading, spacing: 10) {
              HStack(spacing: 12) {
                DiffuserGlyph(kind: source.kind, level: 2)
                  .foregroundStyle(Theme.roomInk)
                  .tint(Theme.mist)
                  .frame(width: 40, height: 40)
                  .padding(4)
                  .background(
                    LinearGradient(colors: [Theme.roomTop, Theme.roomBottom], startPoint: .top, endPoint: .bottom),
                    in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                VStack(alignment: .leading, spacing: 2) {
                  Text("\(source.title)'s schedule").font(Theme.headline(.title3))
                  Text(summary(profile)).font(.subheadline).foregroundStyle(.secondary)
                }
              }
              .padding(.horizontal, 4)
              Card {
                WeekGrid(profile: profile, now: model.now)
                  .padding(16)
              }
            }

            VStack(alignment: .leading, spacing: 10) {
              SectionTitle("Copy To")
              if targets.isEmpty {
                Text("Add another diffuser to copy this schedule to it.")
                  .font(.callout)
                  .foregroundStyle(.secondary)
                  .padding(.horizontal, 6)
              } else {
                Card {
                  ForEach(Array(targets.enumerated()), id: \.element.id) { index, target in
                    if index > 0 { RowDivider(inset: 62) }
                    TargetRow(
                      target: target, chosen: chosen.contains(target.id), state: sent ? model.copies[target.id] : nil,
                      now: model.now
                    ) {
                      if chosen.contains(target.id) { chosen.remove(target.id) } else { chosen.insert(target.id) }
                    }
                    .disabled(sent)
                  }
                }
              }
              Text(
                "Each one's whole timetable is replaced with this one, then it is told the time. The times stay in \(Words.offset(profile.timezoneHours)), as \(source.title)'s are."
              )
              .font(.footnote)
              .foregroundStyle(.secondary)
              .padding(.horizontal, 6)
            }
          } else {
            ContentUnavailableView(
              "No schedule to copy", systemImage: "calendar.badge.exclamationmark",
              description: Text("Send \(source.title) a schedule first, then copy it."))
          }
        }
        .padding(16)
      }
      .background(Color(.systemGroupedBackground))
      .safeAreaInset(edge: .bottom) {
        if source.schedule != nil && !targets.isEmpty {
          Button {
            if done {
              dismiss()
            } else {
              sent = true
              model.send(.copySchedule(from: source.id, to: Array(chosen)))
            }
          } label: {
            Text(buttonTitle)
              .font(.body.weight(.semibold))
              .frame(maxWidth: .infinity, minHeight: 30)
              .contentTransition(.interpolate)
          }
          .buttonStyle(.glassProminent)
          .controlSize(.large)
          .disabled(chosen.isEmpty || (sent && !done))
          .padding(.horizontal, 20)
          .padding(.bottom, 8)
          .animation(.smooth, value: buttonTitle)
        }
      }
      .navigationTitle("Copy Schedule")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Close", systemImage: "xmark") { dismiss() }
        }
      }
    }
  }

  private var done: Bool {
    sent && chosen.allSatisfy { model.copies[$0] != nil && model.copies[$0] != .sending }
  }

  private var buttonTitle: String {
    if done { return "Done" }
    if sent { return "Copying…" }
    switch chosen.count {
    case 0: return "Choose Where to Copy It"
    case 1: return "Copy to 1 Diffuser"
    default: return "Copy to \(chosen.count) Diffusers"
    }
  }

  private func summary(_ profile: Profile) -> String {
    let count = profile.entries.count
    let times = count == 1 ? "1 time" : "\(count) times"
    let always = profile.alwaysOn.enable ? ", always on at \(Words.level(profile.alwaysOn.level))" : ""
    return "\(times)\(always), in \(Words.offset(profile.timezoneHours))"
  }
}

/// A diffuser the schedule can go to: a check to choose it, its current schedule in words,
/// and after sending, whether its copy went through.
private struct TargetRow: View {
  let target: DiffuserState
  let chosen: Bool
  let state: CopyState?
  let now: Date
  let toggle: () -> Void

  var body: some View {
    Button(action: toggle) {
      HStack(spacing: 14) {
        DiffuserGlyph(kind: target.kind)
          .foregroundStyle(.secondary)
          .frame(width: 32, height: 32)
        VStack(alignment: .leading, spacing: 2) {
          Text(target.title).font(.body.weight(.medium)).foregroundStyle(.primary)
          Text(detail)
            .font(.subheadline)
            .foregroundStyle(state.map(color) ?? .secondary)
            .contentTransition(.interpolate)
        }
        Spacer()
        mark
          .font(.title3)
          .frame(width: 28)
      }
      .padding(.horizontal, 16)
      .padding(.vertical, 12)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .animation(.smooth, value: state)
    .accessibilityAddTraits(chosen ? .isSelected : [])
  }

  @ViewBuilder private var mark: some View {
    switch state {
    case .sending?:
      ProgressView()
    case .copied?:
      Image(systemName: "checkmark.circle.fill")
        .foregroundStyle(Theme.answering)
        .transition(.symbolEffect(.drawOn))
    case .failed?:
      Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.alert)
    case nil:
      Image(systemName: chosen ? "checkmark.circle.fill" : "circle")
        .foregroundStyle(chosen ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
        .contentTransition(.symbolEffect(.replace))
    }
  }

  private var detail: String {
    switch state {
    case .sending?: return "Sending…"
    case .copied?: return "Copied"
    case .failed(let why)?: return why
    case nil:
      guard let profile = target.schedule else { return "No schedule known" }
      let count = profile.entries.count
      return "Has \(count == 1 ? "1 time" : "\(count) times") now"
    }
  }

  private func color(_ state: CopyState) -> Color {
    switch state {
    case .sending: Theme.pending
    case .copied: Theme.answering
    case .failed: Theme.alert
    }
  }
}
