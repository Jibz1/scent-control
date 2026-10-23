import ScentEngine
import ScentProtocol
import SwiftUI

/// Adds a diffuser a scan found: name it and pick its room, then watch the Mac's add flow
/// run as a list of steps. Each step's mark draws itself in when it is done; the restart's
/// wait drains as a ring. Closing the sheet doesn't stop the add.
struct AddSheet: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let found: FoundDiffuser
  @State private var name = ""
  @State private var room = ""
  @State private var wifiName = ""
  @State private var wifiPassword = ""

  private var progress: AddProgress? { model.adds[found.id] }

  private var shownName: String { progress?.name ?? name }

  /// Whether the add stopped at a step that failed, so it can be tried again.
  private var failed: Bool {
    progress?.steps.contains { if case .failed = $0.state { true } else { false } } ?? false
  }

  private var buttonTitle: String {
    if progress?.needsWifi == true { return "Continue" }
    return failed ? "Try Again" : "Add"
  }

  /// Starts the add, or starts it again: with the network when the diffuser asked for one.
  /// The password goes to the engine once and is cleared here.
  private func start() {
    let wifi = progress?.needsWifi == true && !wifiName.isEmpty
    model.send(
      .add(
        found: found.id, name: (progress?.name ?? name).isEmpty ? nil : progress?.name ?? name,
        room: (progress?.room ?? room).isEmpty ? nil : progress?.room ?? room,
        wifiName: wifi ? wifiName : nil, wifiPassword: wifi ? wifiPassword : nil))
    wifiPassword = ""
  }

  /// Its model unless that is already the title, then its color.
  private var subtitle: String {
    let parts = shownName.isEmpty ? [found.variant] : [found.kind.modelName, found.variant]
    return parts.compactMap { $0 }.joined(separator: ", ")
  }

  var body: some View {
    NavigationStack {
      Form {
        Section {
          HStack(spacing: 16) {
            DiffuserGlyph(kind: found.kind, level: progress?.finished == true ? 2 : 0)
              .foregroundStyle(Theme.roomInk)
              .tint(Theme.mist)
              .frame(width: 52, height: 52)
              .padding(6)
              .background(
                LinearGradient(colors: [Theme.roomTop, Theme.roomBottom], startPoint: .top, endPoint: .bottom),
                in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            VStack(alignment: .leading, spacing: 3) {
              Text(shownName.isEmpty ? found.kind.modelName : shownName).font(Theme.headline(.title2))
              HStack(spacing: 6) {
                Text(subtitle)
                if let rssi = found.rssi {
                  SignalBars(filled: SignalBars.bars(rssi))
                }
              }
              .font(.subheadline)
              .foregroundStyle(.secondary)
            }
          }
          .listRowBackground(Color.clear)
          .listRowInsets(EdgeInsets())
        }

        Section {
          if let progress {
            LabeledContent("Name", value: progress.name ?? "None")
            LabeledContent("Room", value: progress.room ?? "No room")
          } else {
            TextField("Name", text: $name, prompt: Text("Name, such as Bedside"))
            RoomField(room: $room, rooms: model.roomNames)
          }
        } footer: {
          Text("What it is called in the list, and the room it is listed under.")
        }

        if progress?.needsWifi == true {
          Section {
            TextField("Network", text: $wifiName, prompt: Text("Network name"))
              .textInputAutocapitalization(.never)
              .autocorrectionDisabled()
            SecureField("WiFi password", text: $wifiPassword)
          } header: {
            Text("WiFi")
          } footer: {
            Text("It isn't connected to WiFi right now. Diffusers join 2.4 GHz networks only. The password goes to the diffuser over Bluetooth and is never stored.")
          }
        }

        if let progress {
          Section {
            ForEach(Array(progress.steps.enumerated()), id: \.offset) { _, item in
              StepRow(step: item.step, state: item.state, now: model.now)
            }
          } footer: {
            Text(progress.finished ? "It's on your broker, and will answer pings shortly." : "Closing this sheet doesn't stop the add.")
          }
        } else {
          Section {
          } footer: {
            Text(
              "Scent Control reads the diffuser over Bluetooth, points it at your broker and restarts it. Keep this iPhone near it, with Scentiment's app closed."
            )
          }
        }
      }
      .navigationTitle(progress?.finished == true ? "Added" : "Add Diffuser")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Close", systemImage: "xmark") { dismiss() }
        }
        ToolbarItem(placement: .confirmationAction) {
          if progress?.finished == true {
            Button("Done", systemImage: "checkmark") { dismiss() }
              .buttonStyle(.glassProminent)
          } else if progress == nil || progress?.needsWifi == true || failed {
            Button(buttonTitle) { start() }
              .buttonStyle(.glassProminent)
              .fontWeight(.semibold)
              .disabled(progress?.needsWifi == true && (wifiName.isEmpty || wifiPassword.isEmpty))
          }
        }
      }
      .animation(.smooth, value: progress)
    }
  }
}

/// A room typed in, or picked from the rooms already in use.
private struct RoomField: View {
  @Binding var room: String
  let rooms: [String]

  var body: some View {
    HStack {
      TextField("Room", text: $room, prompt: Text("Room, such as Bedroom"))
      if !rooms.isEmpty {
        Menu {
          ForEach(rooms, id: \.self) { name in
            Button(name) { room = name }
          }
        } label: {
          Image(systemName: "chevron.up.chevron.down").foregroundStyle(.secondary)
        }
        .accessibilityLabel("Choose a Room")
      }
    }
  }
}

/// One step of the add, with a mark for where it stands.
private struct StepRow: View {
  let step: AddProgress.Step
  let state: AddProgress.StepState
  let now: Date

  var body: some View {
    HStack(spacing: 14) {
      mark.frame(width: 28, height: 28)
      VStack(alignment: .leading, spacing: 2) {
        Text(title)
          .foregroundStyle(isWaiting ? .secondary : .primary)
          .contentTransition(.interpolate)
        if case .failed(let why) = state {
          Text(why).font(.footnote).foregroundStyle(Theme.alert)
        }
      }
    }
    .padding(.vertical, 2)
    .animation(.smooth, value: state)
  }

  private var isWaiting: Bool { state == .waiting }

  @ViewBuilder private var mark: some View {
    switch state {
    case .waiting:
      Image(systemName: "circle.dotted").foregroundStyle(.tertiary).font(.title3)
    case .working:
      ProgressView()
    case .countdown(let until, let total):
      let left = max(until.timeIntervalSince(now), 0)
      ZStack {
        Circle().stroke(.quaternary, lineWidth: 3)
        Circle()
          .trim(from: 0, to: left / total)
          .stroke(Theme.accent, style: StrokeStyle(lineWidth: 3, lineCap: .round))
          .rotationEffect(.degrees(-90))
          .animation(.linear(duration: 1), value: left)
        Text("\(Int(left.rounded(.up)))")
          .font(.system(size: 10, weight: .bold, design: .rounded))
          .monospacedDigit()
          .contentTransition(.numericText(countsDown: true))
      }
    case .done:
      Image(systemName: "checkmark.circle.fill")
        .font(.title3)
        .foregroundStyle(.tint)
        .transition(.symbolEffect(.drawOn))
    case .failed:
      Image(systemName: "exclamationmark.circle.fill").font(.title3).foregroundStyle(Theme.alert)
    }
  }

  private var title: String {
    let done = state == .done
    let working = !done && !isWaiting
    switch step {
    case .read: return done ? "Read the diffuser" : working ? "Reading the diffuser…" : "Read the diffuser"
    case .joinWifi: return done ? "Joined your WiFi" : working ? "Joining your WiFi…" : "Join your WiFi"
    case .moveToBroker:
      return done ? "Moved it to your broker" : working ? "Moving it to your broker…" : "Move it to your broker"
    case .restart: return done ? "Restarted" : working ? "Restarting…" : "Restart it"
    case .waitForConnect:
      return done ? "Connected" : working ? "Waiting for it to connect…" : "Wait for it to connect"
    }
  }
}
