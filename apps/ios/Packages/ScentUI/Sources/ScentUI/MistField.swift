import SwiftUI

/// Mist rising from the diffuser: plumes that drift up, swell and fade as they climb, drawn
/// blurred. `intensity` runs from 0 (none) to 3 (High)
/// and is animatable, so a change of level thickens or thins the mist rather than jumping,
/// and switching off lets the last plumes rise away. `alert` tints every third plume.
struct MistField: View, Animatable {
  var intensity: Double
  var color: Color
  var alert: Color?

  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.colorScheme) private var colorScheme

  nonisolated var animatableData: Double {
    get { intensity }
    set { intensity = newValue }
  }

  /// Plumes at High; lower intensities show fewer, the rest fading out one by one.
  private let plumes = 44

  var body: some View {
    let moving = !reduceMotion && intensity > 0.01
    TimelineView(.animation(minimumInterval: 1 / 30, paused: !moving)) { timeline in
      // Held still, the mist is drawn at a moment that shows a full plume.
      let now = moving ? timeline.date.timeIntervalSinceReferenceDate : 4.2
      Canvas { context, size in
        guard intensity > 0.01 else { return }
        let base = CGPoint(x: size.width / 2, y: size.height)
        context.addFilter(.blur(radius: 16))
        if colorScheme == .dark { context.blendMode = .screen }
        let strength = 0.12 + 0.11 * intensity
        for plume in 0..<plumes {
          let share = min(max(intensity / 3 * Double(plumes) - Double(plume), 0), 1)
          guard share > 0 else { continue }
          let (pace, start, lean) = (noise(plume, 1), noise(plume, 2), noise(plume, 3))
          let period = 6.5 + 6 * pace
          let rise = (now / period + start).truncatingRemainder(dividingBy: 1)
          let sway = sin(now / (2.2 + 2.4 * lean) + Double(plume)) * 22 * rise
          let spread = (lean - 0.5) * size.width * 0.9 * rise
          let radius = (9 + 52 * rise) * (0.65 + 0.6 * pace)
          let center = CGPoint(x: base.x + spread + sway, y: size.height * (1 - rise * 0.98))
          context.opacity = min(rise / 0.08, 1) * pow(1 - rise, 1.4) * strength * share
          let fill = alert != nil && plume % 3 == 0 ? alert! : color
          context.fill(
            Path(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2)),
            with: .color(fill))
        }
      }
    }
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }

  /// The same pseudo-random number in 0..<1 for the same plume and key, every frame.
  private func noise(_ plume: Int, _ key: Double) -> Double {
    let x = sin(Double(plume) * 12.9898 + key * 78.233) * 43758.5453
    return x - x.rounded(.down)
  }
}
