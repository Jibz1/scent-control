import ScentEngine
import ScentProtocol
import SwiftUI

/// What was sent and heard, newest first, as sentences. Tapping a line shows the message
/// behind it, passwords masked.
struct LogScreen: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var open: Set<Int> = []

  var body: some View {
    NavigationStack {
      List {
        if model.log.isEmpty {
          ContentUnavailableView(
            "Nothing yet", systemImage: "text.bubble",
            description: Text("Commands and reports show here as they happen."))
        }
        ForEach(model.log) { line in
          Button {
            withAnimation(.snappy) {
              if open.contains(line.id) { open.remove(line.id) } else { open.insert(line.id) }
            }
          } label: {
            VStack(alignment: .leading, spacing: 8) {
              HStack(alignment: .firstTextBaseline, spacing: 10) {
                Image(systemName: symbol(line.mark))
                  .font(.footnote.weight(.bold))
                  .foregroundStyle(color(line.mark))
                  .frame(width: 16)
                Text(line.sentence)
                  .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(line.at, format: .dateTime.hour().minute())
                  .font(.caption)
                  .monospacedDigit()
                  .foregroundStyle(.secondary)
              }
              if open.contains(line.id), let detail = line.detail {
                Text(detail)
                  .font(.caption.monospaced())
                  .foregroundStyle(.secondary)
                  .textSelection(.enabled)
                  .padding(10)
                  .frame(maxWidth: .infinity, alignment: .leading)
                  .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                  .transition(.opacity.combined(with: .move(edge: .top)))
              }
            }
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .accessibilityHint(line.detail == nil ? "" : "Shows the message")
        }
      }
      .navigationTitle("Activity")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done", systemImage: "checkmark") { dismiss() }
        }
      }
    }
  }

  private func symbol(_ mark: LogLine.Mark) -> String {
    switch mark {
    case .sent: "arrow.up.right"
    case .heard: "arrow.down.left"
    case .failure: "exclamationmark"
    }
  }

  private func color(_ mark: LogLine.Mark) -> Color {
    switch mark {
    case .sent: Theme.accent
    case .heard: .secondary
    case .failure: Theme.alert
    }
  }
}

/// The broker and its password, the appearance, and About.
struct SettingsScreen: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var settings = BrokerSettings()
  @State private var password = ""

  var body: some View {
    @Bindable var model = model
    NavigationStack {
      Form {
        Section {
          LabeledContent("Status") {
            Text(status).foregroundStyle(statusColor)
          }
          Field("Address") {
            TextField("Address", text: $settings.url, prompt: Text("wss://broker.example.com/mqtt"))
              .keyboardType(.URL)
          }
          Field("Username") {
            TextField("Username", text: $settings.username, prompt: Text("Required"))
          }
          Field("Route ID") {
            TextField("Route ID", text: $settings.routeID, prompt: Text("Required"))
          }
          Field("Password") {
            SecureField("Password", text: $password, prompt: Text("Saved in the Keychain"))
          }
        } header: {
          Text("Broker")
        } footer: {
          Text(
            "WiFi control goes through this broker, so it works away from home. The password stays in this iPhone's Keychain and is never shown again."
          )
        }

        Section {
          Button("Save and Connect") {
            model.send(.saveBroker(settings, password: password.isEmpty ? nil : password))
            password = ""
          }
          .disabled(settings == model.brokerSettings && password.isEmpty)
        }

        Section("Appearance") {
          Picker(
            "Appearance",
            selection: Binding(get: { model.appearance }, set: { model.send(.setAppearance($0)) })
          ) {
            Text("System").tag(Appearance.system)
            Text("Light").tag(Appearance.light)
            Text("Dark").tag(Appearance.dark)
          }
          .pickerStyle(.segmented)
          .labelsHidden()
        }

        Section("About") {
          LabeledContent("Scent Control", value: "Version \(version)")
          Text("Controls Scentiment diffusers through your own broker. It never contacts Scentiment.")
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
      }
      .navigationTitle("Settings")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done", systemImage: "checkmark") { dismiss() }
        }
      }
      .onAppear { settings = model.brokerSettings }
    }
  }

  private var version: String {
    let info = Bundle.main.infoDictionary
    let short = info?["CFBundleShortVersionString"] as? String ?? "0.1.0"
    let build = info?["CFBundleVersion"] as? String
    return build.map { "\(short) (\($0))" } ?? short
  }

  private var status: String {
    switch model.broker {
    case .connected: "Connected"
    case .connecting: "Connecting…"
    case .loginRefused: "Login refused"
    case .notSetUp: "Not set up"
    case .unreachable: "Can't reach it"
    }
  }

  private var statusColor: Color {
    switch model.broker {
    case .connected: Theme.answering
    case .connecting: Theme.pending
    case .loginRefused, .unreachable: Theme.alert
    case .notSetUp: .secondary
    }
  }
}

/// A setting's name with its field after it, the field's text trailing, as Settings does.
private struct Field<Content: View>: View {
  let label: String
  @ViewBuilder var content: Content

  init(_ label: String, @ViewBuilder content: () -> Content) {
    self.label = label
    self.content = content()
  }

  var body: some View {
    LabeledContent(label) {
      content
        .multilineTextAlignment(.trailing)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
    }
  }
}

/// Renames a diffuser and moves it between rooms. Nothing is sent to the diffuser.
struct RenameSheet: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let diffuser: DiffuserState
  @State private var name: String
  @State private var room: String

  init(diffuser: DiffuserState) {
    self.diffuser = diffuser
    _name = State(initialValue: diffuser.name ?? "")
    _room = State(initialValue: diffuser.room ?? "")
  }

  var body: some View {
    NavigationStack {
      Form {
        Section {
          TextField("Name", text: $name, prompt: Text(diffuser.kind.modelName))
          TextField("Room", text: $room, prompt: Text("No room"))
        } footer: {
          Text("Only this iPhone keeps these. The diffuser isn't told.")
        }
      }
      .navigationTitle("Rename")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Cancel", systemImage: "xmark") { dismiss() }
        }
        ToolbarItem(placement: .confirmationAction) {
          Button("Save", systemImage: "checkmark") {
            model.send(
              .rename(
                id: diffuser.id, name: name.isEmpty ? nil : name, room: room.isEmpty ? nil : room))
            dismiss()
          }
          .buttonStyle(.glassProminent)
        }
      }
    }
    .presentationDetents([.medium])
  }
}
