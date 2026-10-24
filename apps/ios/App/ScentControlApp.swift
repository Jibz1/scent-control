import ScentEngine
import ScentUI
import SwiftUI

/// The app target is thin: it picks the engine and hands the model to the screens.
///
/// It talks to the real diffusers, keeping its file in Application Support. Launch
/// arguments switch to sample data instead, for screenshots: `-screen <name>` opens a screen
/// (see `StartScreen`), `-sample YES` runs the samples from the home screen, and `-frozen
/// YES` holds the clock at the samples' time.
@main
struct ScentControlApp: App {
  @State private var model: AppModel
  @Environment(\.scenePhase) private var scenePhase
  private let start: StartScreen

  init() {
    let defaults = UserDefaults.standard
    let frozen = defaults.bool(forKey: "frozen")
    let screen = defaults.string(forKey: "screen").flatMap(StartScreen.init(rawValue:))
    start = screen ?? .home
    let model: AppModel
    if screen != nil || defaults.bool(forKey: "sample") {
      model = Samples.model(frozen: frozen)
      if start == .add {
        model.found = [Samples.foundAir2]
        model.adds[Samples.foundAir2.id] = Samples.addUnderWay(at: model.now)
      }
      if start == .diffuserNotKnown {
        let now = model.now
        model.update("TESTDEVICE02") { Samples.reconnected(&$0, at: now) }
      }
    } else {
      let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      let seed = Bundle.main.url(forResource: "seed", withExtension: "json")
      model = AppModel(engine: LiveEngine(folder: support.appendingPathComponent("ScentControl"), seed: seed))
    }
    model.startClock(frozen: frozen)
    _model = State(initialValue: model)
  }

  var body: some Scene {
    WindowGroup {
      ScentControlRoot(model: model, start: start)
    }
    .onChange(of: scenePhase) { _, phase in
      model.send(.appActive(phase == .active))
    }
  }
}
