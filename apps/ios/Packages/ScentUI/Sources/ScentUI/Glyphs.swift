import ScentEngine
import SwiftUI

/// The Mac app's diffuser glyphs (`apps/desktop/src/ui/glyphs.rs`), redrawn: the Air 2 as an
/// upright capsule with its mist outlet, the HVAC 2 as a low box with two vent slits, and up
/// to three wisps of mist rising from the top. All on a grid of 28 units, with strokes 1.5
/// units wide, round ends and joins.
private struct Grid {
  let origin: CGPoint
  let unit: CGFloat

  /// `span` is how many units the square shows across: fewer zooms in, as for a body drawn
  /// alone. `lift` moves the drawing up by that many units, to centre such a body.
  init(_ rect: CGRect, span: CGFloat = 28, lift: CGFloat = 0) {
    unit = min(rect.width, rect.height) / span
    origin = CGPoint(x: rect.midX - 14 * unit, y: rect.midY - (14 + lift) * unit)
  }

  func at(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
    CGPoint(x: origin.x + x * unit, y: origin.y + y * unit)
  }

  func rect(_ left: CGFloat, _ top: CGFloat, _ right: CGFloat, _ bottom: CGFloat) -> CGRect {
    CGRect(
      origin: at(left, top), size: CGSize(width: (right - left) * unit, height: (bottom - top) * unit))
  }

  var stroke: StrokeStyle {
    StrokeStyle(lineWidth: max(1.5 * unit, 1.25), lineCap: .round, lineJoin: .round)
  }
}

/// A diffuser's body, as a filled outline.
private struct DiffuserBody: Shape {
  let kind: DiffuserKind
  let centered: Bool

  func path(in rect: CGRect) -> Path {
    let lift: CGFloat = centered ? (kind == .hvac2 ? 5.5 : 5) : 0
    let grid = Grid(rect, span: centered ? 16 : 28, lift: lift)
    var path = Path()
    switch kind {
    case .air2, .other:
      let capsule = grid.rect(11, 14, 17, 24)
      path.addRoundedRect(
        in: capsule, cornerSize: CGSize(width: capsule.width / 2, height: capsule.width / 2))
      path.move(to: grid.at(13, 16.75))
      path.addLine(to: grid.at(15, 16.75))
    case .hvac2:
      path.addRoundedRect(
        in: grid.rect(8, 15.5, 20, 23.5), cornerSize: CGSize(width: 2 * grid.unit, height: 2 * grid.unit))
      for y: CGFloat in [18, 21] {
        path.move(to: grid.at(11, y))
        path.addLine(to: grid.at(17, y))
      }
    }
    return path.strokedPath(grid.stroke)
  }
}

/// One wisp of mist: the left, middle or right one. Each sways in one gentle S; the outer
/// two start a unit higher and lean out, so the plume fans.
private struct Wisp: Shape {
  let index: Int

  func path(in rect: CGRect) -> Path {
    let grid = Grid(rect)
    let side: CGFloat = index == 0 ? -1 : 1
    let foot: CGFloat = index == 1 ? 11.5 : 10.5
    let lean: CGFloat = index == 1 ? 0 : 0.75 * side
    let line: CGFloat = [10.5, 14, 17.5][index]
    var path = Path()
    for step in 0...20 {
      let along = CGFloat(step) / 20
      let x = line + side * 0.7 * sin(2 * .pi * along) + lean * along
      let point = grid.at(x, foot - 7 * along)
      if step == 0 { path.move(to: point) } else { path.addLine(to: point) }
    }
    return path.strokedPath(grid.stroke)
  }
}

/// A diffuser drawn in the current foreground style, its mist in the tint. With a `level`
/// (0 to 3) the three wisps show, lit as the Mac lights them: one, the middle; two, the
/// outer pair; three, all; the rest as faint tracks. Without one, the body alone, larger.
struct DiffuserGlyph: View {
  let kind: DiffuserKind
  var level: Int?
  var trackOpacity: Double = 0.55

  @Environment(\.accessibilityReduceMotion) private var reduceMotion

  var body: some View {
    ZStack {
      DiffuserBody(kind: kind, centered: level == nil).fill(.primary)
      if let level {
        let shares = Self.shares(level)
        ForEach(0..<3, id: \.self) { index in
          Wisp(index: index).fill(.quaternary.opacity(trackOpacity))
          Wisp(index: index).fill(.tint)
            .opacity(shares[index])
            .offset(y: shares[index] > 0 ? 0 : 2)
        }
      }
    }
    .animation(reduceMotion ? nil : .spring(duration: 0.5, bounce: 0.3), value: level)
    .accessibilityHidden(true)
  }

  /// How much of the left, middle and right wisp shows at `level`.
  static func shares(_ level: Int) -> [Double] {
    switch level {
    case ...0: [0, 0, 0]
    case 1: [0, 1, 0]
    case 2: [1, 0, 1]
    default: [1, 1, 1]
    }
  }
}

/// Four signal bars rising left to right, the first `filled` solid. Words beside them say
/// what they show.
struct SignalBars: View {
  let filled: Int
  @ScaledMetric(relativeTo: .footnote) private var step: CGFloat = 3

  var body: some View {
    HStack(alignment: .bottom, spacing: step * 0.7) {
      ForEach(0..<4, id: \.self) { bar in
        RoundedRectangle(cornerRadius: step / 3)
          .fill(bar < filled ? AnyShapeStyle(.primary) : AnyShapeStyle(.quaternary))
          .frame(width: step, height: step * (1.4 + CGFloat(bar)))
      }
    }
    .accessibilityHidden(true)
  }

  /// Bars for a signal in dBm, as the Mac counts them.
  static func bars(_ dbm: Int) -> Int {
    switch dbm {
    case (-55)...: 4
    case (-67)...: 3
    case (-78)...: 2
    default: 1
    }
  }
}
