import ScentEngine
import ScentProtocol
import SwiftUI

/// One diffuser: its room at the top, then its schedule, what it last reported, how the app
/// reaches it, and its details.
struct DiffuserScreen: View {
  @Environment(AppModel.self) private var model
  let id: String
  var scrollToDetails = false
  let open: (Route) -> Void
  /// True once the room has scrolled up under the navigation bar.
  @State private var pastRoom = false

  var body: some View {
    if let diffuser = model.diffuser(id) {
      GeometryReader { proxy in
        ScrollView {
          VStack(spacing: 28) {
            RoomHero(
              diffuser: diffuser, now: model.now, topInset: proxy.safeAreaInsets.top,
              setPower: { model.send(.setPower(id: id, on: $0)) },
              setLevel: { model.send(.setLevel(id: id, level: $0)) })
            .onScrollVisibilityChange(threshold: 0.3) { visible in
              withAnimation(.smooth(duration: 0.25)) { pastRoom = !visible }
            }
            VStack(spacing: 28) {
              if let target = diffuser.setUpTarget, !diffuser.onOurBroker {
                NotOnBroker { open(.add(target, name: diffuser.name ?? "", room: diffuser.room ?? "")) }
              }
              ScheduleSection(diffuser: diffuser, now: model.now, open: open)
              StatusSection(diffuser: diffuser, now: model.now)
              ConnectionSection(diffuser: diffuser, open: open)
              DetailsSection(diffuser: diffuser, open: open)
            }
            .padding(.horizontal, 16)
          }
          .padding(.bottom, 32)
        }
        .ignoresSafeArea(edges: .top)
        .scrollEdgeEffectHidden(true, for: .top)
        // The scroll view runs under the bar, so the bar has no edge effect of its own: once
        // the room has gone, this soft material keeps the title readable over the cards.
        .overlay(alignment: .top) {
          Rectangle()
            .fill(.regularMaterial)
            .mask {
              LinearGradient(
                stops: [.init(color: .black, location: 0.7), .init(color: .clear, location: 1)],
                startPoint: .top, endPoint: .bottom)
            }
            .frame(height: proxy.safeAreaInsets.top + 18)
            .opacity(pastRoom ? 1 : 0)
            .allowsHitTesting(false)
            .ignoresSafeArea(edges: .top)
        }
        .defaultScrollAnchor(scrollToDetails ? .bottom : .top)
      }
      .toolbarBackgroundVisibility(.hidden, for: .navigationBar)
      .background(Color(.systemGroupedBackground))
      .navigationTitle(diffuser.title)
      .navigationSubtitle(diffuser.room ?? "")
      .navigationBarTitleDisplayMode(.inline)
    } else {
      ContentUnavailableView("Diffuser not found", systemImage: "questionmark.circle")
    }
  }
}

/// The last schedule sent, as today's strip and the week, with Edit and Copy.
private struct ScheduleSection: View {
  let diffuser: DiffuserState
  let now: Date
  let open: (Route) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      SectionTitle(title: "Schedule") {
        Button(diffuser.schedule == nil ? "Create" : "Edit") { open(.schedule(diffuser.id)) }
          .font(.body.weight(.semibold))
      }
      Card {
        if let profile = diffuser.schedule {
          VStack(alignment: .leading, spacing: 14) {
            Label(Words.scheduleNow(profile, now: now) ?? "", systemImage: "clock")
              .font(.subheadline.weight(.semibold))
              .labelStyle(TightLabel())
            TodayStrip(profile: profile, now: now)
          }
          .padding(16)
          RowDivider()
          WeekGrid(profile: profile, now: now)
            .padding(16)
          RowDivider()
          Button {
            open(.copy(diffuser.id))
          } label: {
            HStack {
              Label("Copy to Another Diffuser", systemImage: "square.on.square")
              Spacer()
              Image(systemName: "chevron.forward").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 13)
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .foregroundStyle(.tint)
        } else {
          VStack(alignment: .leading, spacing: 8) {
            Text("No schedule known")
              .font(.headline)
            Text(
              "A diffuser can't be asked for its schedule, so Scent Control shows the last one sent from this iPhone or the Mac."
            )
            .font(.subheadline)
            .foregroundStyle(.secondary)
          }
          .padding(16)
        }
      }
      if let sent = diffuser.scheduleSentAt {
        Text("The diffuser runs this on its own. Last sent from Scent Control \(Words.ago(sent, now: now)).")
          .font(.footnote)
          .foregroundStyle(.secondary)
          .padding(.horizontal, 6)
      }
    }
  }
}

/// What it last reported, each with its age and transport, greyed out once stale.
private struct StatusSection: View {
  let diffuser: DiffuserState
  let now: Date

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      SectionTitle("Status")
      Card {
        FactRow(
          label: "Power source", value: powerSource,
          note: diffuser.pluggedIn.map { Words.note($0, now: now) },
          stale: isStale(diffuser.pluggedIn?.at), symbol: diffuser.pluggedIn?.value == false ? "battery.75percent" : "powerplug")
        RowDivider(inset: 50)
        HStack {
          FactRow(
            label: "WiFi signal", value: signal,
            note: diffuser.signal.map { Words.note($0, now: now) },
            stale: isStale(diffuser.signal?.at), symbol: "wifi")
        }
        RowDivider(inset: 50)
        FactRow(
          label: "Last report", value: diffuser.lastReport.map { Words.ago($0, now: now).capitalizedFirst } ?? "Nothing heard yet",
          symbol: "waveform")
      }
    }
  }

  private func isStale(_ at: Date?) -> Bool {
    guard let at else { return false }
    return now.timeIntervalSince(at) > 300
  }

  private var powerSource: String {
    switch diffuser.pluggedIn?.value {
    case true?: return "Plugged in"
    case false?: return diffuser.battery.map { "Battery, \($0.value)%" } ?? "Battery"
    case nil: return "Not reported yet"
    }
  }

  private var signal: String {
    guard let dbm = diffuser.signal?.value else { return "Not reported yet" }
    let words = ["Weak", "Fair", "Good", "Excellent"][SignalBars.bars(dbm) - 1]
    return "\(words), \(dbm) dBm"
  }
}

/// A diffuser found over Bluetooth that hasn't been heard on the broker: an add that
/// stopped partway, or one still on another broker. As the Mac says, with its Add to Broker…
private struct NotOnBroker: View {
  let add: () -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Label {
        Text(
          "This diffuser hasn't been heard on your broker, so WiFi commands may not reach it. Bluetooth works while this iPhone is in range."
        )
      } icon: {
        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Theme.pending)
      }
      .font(.callout)
      Button("Add to Broker…", action: add)
        .buttonStyle(.glassProminent)
        .fontWeight(.semibold)
    }
    .padding(16)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Theme.pending.opacity(0.12), in: RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous))
  }
}

/// How the app reaches it: Send Over, Read Status, and Set Up Again for one on the broker.
private struct ConnectionSection: View {
  @Environment(AppModel.self) private var model
  let diffuser: DiffuserState
  let open: (Route) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      SectionTitle("Connection")
      Card {
        VStack(alignment: .leading, spacing: 10) {
          Text("Send over")
          Picker(
            "Send over",
            selection: Binding(
              get: { diffuser.sendOver },
              set: { model.send(.setSendOver(id: diffuser.id, $0)) })
          ) {
            Text("Auto").tag(SendOver.auto)
            Text("WiFi").tag(SendOver.wifi)
            Text("Bluetooth").tag(SendOver.bluetooth)
          }
          .pickerStyle(.segmented)
          .labelsHidden()
          Text(sendOverNote)
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
        .padding(16)
        RowDivider()
        Button {
          model.send(.readStatus(id: diffuser.id))
        } label: {
          Label("Read Status", systemImage: "arrow.clockwise")
            .padding(.horizontal, 16)
            .padding(.vertical, 13)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.tint)
        if let target = diffuser.setUpTarget, diffuser.onOurBroker {
          RowDivider()
          Button {
            open(.add(target, name: diffuser.name ?? "", room: diffuser.room ?? ""))
          } label: {
            Label("Set Up Again…", systemImage: "arrow.triangle.2.circlepath")
              .padding(.horizontal, 16)
              .padding(.vertical, 13)
              .frame(maxWidth: .infinity, alignment: .leading)
              .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .foregroundStyle(.tint)
        }
      }
      if diffuser.setUpTarget != nil, diffuser.onOurBroker {
        Text("Set Up Again points it at the broker in Settings and restarts it.")
          .font(.footnote)
          .foregroundStyle(.secondary)
          .padding(.horizontal, 6)
      }
    }
  }

  private var sendOverNote: String {
    switch diffuser.sendOver {
    case .auto:
      diffuser.bluetoothLinked
        ? "Bluetooth when this iPhone is in range, otherwise WiFi through your broker."
        : "WiFi through your broker. Bluetooth joins in once a scan finds this diffuser nearby."
    case .wifi: "Always through your broker, from anywhere."
    case .bluetooth: "Only over Bluetooth, while this iPhone is in range."
    }
  }
}

/// Its name, room and identity, and Set Up Again.
private struct DetailsSection: View {
  let diffuser: DiffuserState
  let open: (Route) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      SectionTitle(title: "Details") {
        Button("Rename") { open(.rename(diffuser.id)) }
          .font(.body.weight(.semibold))
      }
      Card {
        FactRow(label: "Name", value: diffuser.name ?? "None")
        RowDivider()
        FactRow(label: "Room", value: diffuser.room ?? "No room")
        RowDivider()
        FactRow(label: "Model", value: [diffuser.kind.modelName, diffuser.variant].compactMap { $0 }.joined(separator: ", "))
        RowDivider()
        FactRow(label: "Firmware", value: diffuser.firmware ?? "Not read yet")
        RowDivider()
        FactRow(label: "Device ID", value: diffuser.id)
          .textSelection(.enabled)
          .monospaced()
      }
    }
  }
}

/// A label with its icon close to its title.
struct TightLabel: LabelStyle {
  func makeBody(configuration: Configuration) -> some View {
    HStack(spacing: 6) {
      configuration.icon.foregroundStyle(.tint)
      configuration.title
    }
  }
}
