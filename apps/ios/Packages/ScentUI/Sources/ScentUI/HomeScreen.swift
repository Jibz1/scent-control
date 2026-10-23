import ScentEngine
import ScentProtocol
import SwiftUI

/// The first screen: every known diffuser as a tile, grouped by room, with what each is
/// doing in words. A tile that is diffusing is lit with its room's dusk; its power button
/// switches it without opening it. A scan adds a Found Nearby section.
struct HomeScreen: View {
  @Environment(AppModel.self) private var model
  let transition: Namespace.ID
  let open: (Route) -> Void

  private let columns = [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)]

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 26) {
        BrokerLine(status: model.broker) { open(.settings) }
          .padding(.top, 2)
        if let notice = model.notice {
          Label(notice, systemImage: "exclamationmark.triangle.fill")
            .font(.callout)
            .foregroundStyle(Theme.alert)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.alert.opacity(0.12), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        if model.diffusers.isEmpty {
          EmptyHome(scanning: model.scanning) { model.send(.scan) }
        }
        ForEach(model.rooms, id: \.room) { group in
          VStack(alignment: .leading, spacing: 10) {
            SectionTitle(group.room ?? "No Room")
            LazyVGrid(columns: columns, spacing: 12) {
              ForEach(group.diffusers) { diffuser in
                DiffuserTile(diffuser: diffuser, now: model.now) {
                  model.send(.setPower(id: diffuser.id, on: !(diffuser.shownPower ?? false)))
                } open: {
                  open(.diffuser(diffuser.id))
                }
                .matchedTransitionSource(id: diffuser.id, in: transition) { source in
                  source.clipShape(RoundedRectangle(cornerRadius: Theme.tileRadius, style: .continuous))
                }
              }
            }
          }
        }
        if model.scanning || !model.found.isEmpty || model.scanProblem != nil {
          FoundNearby(open: open)
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
      }
      .padding(.horizontal, 16)
      .padding(.bottom, 32)
      .animation(.spring(duration: 0.45, bounce: 0.2), value: model.found)
      .animation(.spring(duration: 0.45, bounce: 0.2), value: model.scanning)
    }
    .background(Color(.systemGroupedBackground))
    .navigationTitle("Diffusers")
    .toolbar {
      ToolbarItem(placement: .topBarLeading) {
        Button("Activity", systemImage: "list.bullet.rectangle.portrait") { open(.log) }
      }
      ToolbarItemGroup(placement: .topBarTrailing) {
        Button("Find Nearby", systemImage: "dot.radiowaves.left.and.right") { model.send(.scan) }
          .symbolEffect(.variableColor.iterative.reversing, isActive: model.scanning)
          .disabled(model.scanning)
        Button("Settings", systemImage: "gearshape") { open(.settings) }
      }
    }
  }
}

/// The broker connection in words, with a dot that is never the only signal.
private struct BrokerLine: View {
  let status: BrokerStatus
  let fix: () -> Void

  var body: some View {
    Button(action: fix) {
      HStack(spacing: 7) {
        Circle().fill(color).frame(width: 8, height: 8)
          .symbolEffect(.pulse, isActive: status == .connecting)
        Text(words)
          .font(.subheadline)
          .foregroundStyle(.secondary)
        if needsAttention {
          Image(systemName: "chevron.forward").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
        }
      }
      .padding(.horizontal, 6)
    }
    .buttonStyle(.plain)
    .allowsHitTesting(needsAttention)
    .accessibilityLabel(words)
  }

  private var needsAttention: Bool {
    status != .connected && status != .connecting
  }

  private var words: String {
    switch status {
    case .connected: "Connected to your broker"
    case .connecting: "Connecting to your broker…"
    case .loginRefused: "Broker login refused. Check the password in Settings."
    case .notSetUp: "Add your broker to control diffusers from anywhere"
    case .unreachable(let why): "Can't reach your broker: \(why)"
    }
  }

  private var color: Color {
    switch status {
    case .connected: Theme.answering
    case .connecting: Theme.pending
    case .loginRefused, .unreachable: Theme.alert
    case .notSetUp: Color.gray
    }
  }
}

/// One known diffuser. Lit with the room's dusk while it diffuses.
struct DiffuserTile: View {
  let diffuser: DiffuserState
  let now: Date
  let togglePower: () -> Void
  let open: () -> Void

  @Environment(\.colorScheme) private var colorScheme

  private var lit: Bool {
    diffuser.shownPower == true && diffuser.reachability != .notAnswering && !diffuser.stateIsStale(now: now)
  }

  var body: some View {
    Button(action: open) {
      VStack(alignment: .leading, spacing: 0) {
        HStack(alignment: .top) {
          DiffuserGlyph(kind: diffuser.kind, level: lit ? diffuser.shownLevel?.value ?? 1 : 0, trackOpacity: lit ? 0.5 : 0)
            .foregroundStyle(lit ? Theme.roomInk : Color.secondary)
            .tint(Theme.mist)
            .frame(width: 50, height: 50)
          Spacer()
          // A diffuser that isn't answering shows its last reported power, faded.
          PowerChip(on: diffuser.shownPower == true, sending: diffuser.pending != nil, action: togglePower)
            .opacity(diffuser.reachability == .notAnswering ? 0.4 : 1)
        }
        Spacer(minLength: 14)
        Text(diffuser.title)
          .font(Theme.headline(.headline, weight: .semibold))
          .foregroundStyle(lit ? Theme.roomInk : Color.primary)
          .lineLimit(1)
        Text(Words.summary(diffuser, now: now))
          .font(.subheadline)
          .foregroundStyle(summaryStyle)
          .contentTransition(.interpolate)
          .lineLimit(1)
        if let hint = Words.scheduleHint(diffuser.schedule, now: now) {
          Text(hint)
            .font(.caption)
            .foregroundStyle(lit ? Theme.roomInkSecondary : Color.secondary)
            .lineLimit(1)
            .padding(.top, 3)
        }
      }
      .padding(14)
      .frame(maxWidth: .infinity, minHeight: 152, alignment: .topLeading)
      .background {
        ZStack {
          RoundedRectangle(cornerRadius: Theme.tileRadius, style: .continuous)
            .fill(Color(.secondarySystemGroupedBackground))
          RoundedRectangle(cornerRadius: Theme.tileRadius, style: .continuous)
            .fill(LinearGradient(colors: [Theme.roomTop, Theme.roomBottom], startPoint: .top, endPoint: .bottom))
            .opacity(lit ? 1 : 0)
        }
      }
      .contentShape(RoundedRectangle(cornerRadius: Theme.tileRadius, style: .continuous))
    }
    .buttonStyle(PressableStyle())
    .animation(.smooth(duration: 0.5), value: lit)
    .animation(.smooth(duration: 0.3), value: diffuser.pending)
    .accessibilityElement(children: .combine)
    .accessibilityLabel("\(diffuser.title), \(Words.summary(diffuser, now: now))")
    .accessibilityAction(named: diffuser.shownPower == true ? "Turn Off" : "Turn On", togglePower)
  }

  private var summaryStyle: Color {
    if diffuser.pending != nil { return Theme.pending }
    if diffuser.reachability == .notAnswering { return Theme.alert }
    return lit ? Theme.roomInkSecondary : .secondary
  }
}

/// The round power button on a tile.
private struct PowerChip: View {
  let on: Bool
  let sending: Bool
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      Image(systemName: "power")
        .font(.system(size: 15, weight: .bold))
        .foregroundStyle(on ? Color.white : Color.secondary)
        .frame(width: 34, height: 34)
        .background(Circle().fill(on ? AnyShapeStyle(Theme.accent) : AnyShapeStyle(.quaternary)))
        .opacity(sending ? 0.55 : 1)
        .symbolEffect(.bounce, value: on)
    }
    .buttonStyle(.plain)
    .sensoryFeedback(.impact(weight: .medium), trigger: on)
    .accessibilityLabel(on ? "Turn Off" : "Turn On")
  }
}

/// Before any diffuser is known: what to do, with a Find Nearby button.
private struct EmptyHome: View {
  let scanning: Bool
  let scan: () -> Void

  var body: some View {
    VStack(spacing: 14) {
      DiffuserGlyph(kind: .air2, level: 0)
        .foregroundStyle(.secondary)
        .tint(Theme.mist)
        .frame(width: 84, height: 84)
      Text("No diffusers yet").font(Theme.headline(.title2))
      Text("Stand near your diffusers with Bluetooth on and Scentiment's app closed, then find them.")
        .font(.callout)
        .foregroundStyle(.secondary)
        .multilineTextAlignment(.center)
      Button(action: scan) {
        Text(scanning ? "Looking…" : "Find Nearby").fontWeight(.semibold).padding(.horizontal, 8)
      }
      .buttonStyle(.glassProminent)
      .controlSize(.large)
      .disabled(scanning)
    }
    .frame(maxWidth: .infinity)
    .padding(.vertical, 36)
    .padding(.horizontal, 24)
  }
}

/// What a scan found that the app doesn't know yet, each with an Add button.
private struct FoundNearby: View {
  @Environment(AppModel.self) private var model
  let open: (Route) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      SectionTitle(title: "Found Nearby") {
        if model.scanning {
          ProgressView().controlSize(.small)
        }
      }
      if let problem = model.scanProblem {
        Label(problem, systemImage: "exclamationmark.triangle.fill")
          .font(.callout)
          .foregroundStyle(Theme.alert)
          .padding(.horizontal, 6)
      } else if model.found.isEmpty {
        Text("Looking for diffusers in Bluetooth range…")
          .font(.callout)
          .foregroundStyle(.secondary)
          .padding(.horizontal, 6)
      }
      ForEach(model.found) { found in
        HStack(spacing: 14) {
          DiffuserGlyph(kind: found.kind)
            .foregroundStyle(.tint)
            .frame(width: 34, height: 34)
            .padding(8)
            .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
          VStack(alignment: .leading, spacing: 3) {
            Text(found.kind.modelName).font(Theme.headline(.headline, weight: .semibold))
            HStack(spacing: 6) {
              Text(found.variant ?? "Color unknown")
              if let rssi = found.rssi {
                SignalBars(filled: SignalBars.bars(rssi))
              }
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)
          }
          Spacer()
          Button("Add") { open(.add(found)) }
            .buttonStyle(.glassProminent)
            .fontWeight(.semibold)
        }
        .padding(14)
        .background {
          RoundedRectangle(cornerRadius: Theme.tileRadius, style: .continuous)
            .strokeBorder(style: StrokeStyle(lineWidth: 1.5, dash: [6, 5]))
            .foregroundStyle(.tertiary)
        }
        .transition(.scale(scale: 0.9).combined(with: .opacity))
      }
    }
  }
}
