import ScentEngine
import ScentProtocol
import SwiftUI

/// Where a tap can take you: a diffuser's screen, pushed with a zoom out of its tile, or one
/// of the sheets.
enum Route: Identifiable, Hashable {
  case diffuser(String)
  case schedule(String)
  case copy(String)
  /// Adds a found diffuser, or sets up a known one again with its name and room.
  case add(FoundDiffuser, name: String? = nil, room: String? = nil)
  case rename(String)
  case log
  case settings

  var id: Self { self }
}

/// A screen to open straight away, for screenshots: `-screen copy` and so on.
public enum StartScreen: String, Sendable {
  case home
  case diffuser
  case diffuserDetails
  case diffuserOff
  case schedule
  case copy
  case add
  case log
  case settings
  /// Plays the animations on sample data, for a screen recording.
  case demo
}

/// The whole app: the diffusers, and everything opened from them.
public struct ScentControlRoot: View {
  @State private var model: AppModel
  @State private var path: [String]
  @State private var sheet: Route?
  private let start: StartScreen
  @Namespace private var transition

  public init(model: AppModel, start: StartScreen = .home) {
    _model = State(initialValue: model)
    self.start = start
    switch start {
    case .diffuser, .diffuserDetails, .schedule, .copy: _path = State(initialValue: ["TESTDEVICE01"])
    case .diffuserOff: _path = State(initialValue: ["TESTDEVICE02"])
    default: _path = State(initialValue: [])
    }
    switch start {
    case .schedule: _sheet = State(initialValue: .schedule("TESTDEVICE01"))
    case .copy: _sheet = State(initialValue: .copy("TESTDEVICE01"))
    case .add: _sheet = State(initialValue: model.found.first.map { .add($0) })
    case .log: _sheet = State(initialValue: .log)
    case .settings: _sheet = State(initialValue: .settings)
    default: _sheet = State(initialValue: nil)
    }
  }

  public var body: some View {
    NavigationStack(path: $path) {
      HomeScreen(transition: transition, open: open)
        .navigationDestination(for: String.self) { id in
          DiffuserScreen(id: id, scrollToDetails: start == .diffuserDetails, open: open)
            .navigationTransition(.zoom(sourceID: id, in: transition))
        }
    }
    .sheet(item: $sheet) { route in
      sheetContent(route)
        .environment(model)
        .tint(Theme.accent)
    }
    .environment(model)
    .tint(Theme.accent)
    .preferredColorScheme(colorScheme)
    .task {
      if start == .demo { await playDemo() }
      // The first launch with no broker opens Settings, as the Mac does.
      if start == .home, model.broker == .notSetUp || model.brokerSettings.url.isEmpty, sheet == nil {
        sheet = .settings
      }
    }
  }

  /// Opens Bedside out of its tile, turns it up, off, on again at Low, and goes back.
  private func playDemo() async {
    let id = "TESTDEVICE01"
    let steps: [(Double, () -> Void)] = [
      (1.2, { path.append(id) }),
      (2.4, { model.send(.setLevel(id: id, level: .high)) }),
      (2.6, { model.send(.setPower(id: id, on: false)) }),
      (2.6, { model.send(.setPower(id: id, on: true)) }),
      (1.4, { model.send(.setLevel(id: id, level: .low)) }),
      (2.2, { path.removeAll() }),
    ]
    for (wait, step) in steps {
      try? await Task.sleep(for: .seconds(wait))
      step()
    }
  }

  private var colorScheme: ColorScheme? {
    switch model.appearance {
    case .system: nil
    case .light: .light
    case .dark: .dark
    }
  }

  private func open(_ route: Route) {
    if case .diffuser(let id) = route {
      path.append(id)
    } else {
      sheet = route
    }
  }

  @ViewBuilder private func sheetContent(_ route: Route) -> some View {
    switch route {
    case .schedule(let id):
      if let diffuser = model.diffuser(id) {
        ScheduleEditor(diffuser: diffuser, expandFirst: start == .schedule)
      }
    case .copy(let id):
      if let diffuser = model.diffuser(id) {
        CopyScheduleSheet(source: diffuser, chosen: start == .copy ? ["TESTDEVICE02"] : [])
      }
    case .add(let found, let name, let room):
      AddSheet(found: found, name: name, room: room)
    case .rename(let id):
      if let diffuser = model.diffuser(id) {
        RenameSheet(diffuser: diffuser)
      }
    case .log:
      LogScreen()
    case .settings:
      SettingsScreen()
    case .diffuser:
      EmptyView()
    }
  }
}
