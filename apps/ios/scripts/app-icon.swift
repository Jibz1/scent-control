// Draws the iPhone app icon into App/Assets.xcassets/AppIcon.appiconset: the Mac icon's
// design (`apps/desktop/src/ui/icon.rs`), redrawn full-bleed, since iOS rounds the corners
// itself. An indigo ground, and the Air 2 of the app's glyph in white, its capsule filled
// out to the outer edge of the glyph's stroke with its outlet a slot, and its three wisps of
// mist rising, thinning as they go. Three appearances: light (the Mac's), dark (the room at
// night) and tinted (grey, for the system to tint). Run from apps/ios after changing it:
//
//   swift scripts/app-icon.swift
//
// and commit the PNGs it writes.
import AppKit
import CoreGraphics

let canvas: CGFloat = 1024
/// The glyph's 28-unit grid on the canvas: the Mac's 640 px on an 824 px tile, scaled to
/// the full canvas.
let grid: CGFloat = 640 / 824 * canvas
let unit = grid / 28
/// Moves the drawing so the diffuser and its mist sit in the middle of the canvas: the
/// glyph spans 3.5 to 24.75 units down its grid.
let lift: CGFloat = (3.5 + 24.75) / 2 - 14

struct RGB {
  var r, g, b: CGFloat
  init(_ hex: UInt32) {
    r = CGFloat((hex >> 16) & 0xFF) / 255
    g = CGFloat((hex >> 8) & 0xFF) / 255
    b = CGFloat(hex & 0xFF) / 255
  }
  init(r: CGFloat, g: CGFloat, b: CGFloat) { (self.r, self.g, self.b) = (r, g, b) }
  func mix(_ other: RGB, _ t: CGFloat) -> RGB {
    RGB(r: r + (other.r - r) * t, g: g + (other.g - g) * t, b: b + (other.b - b) * t)
  }
  var cg: CGColor { CGColor(srgbRed: r, green: g, blue: b, alpha: 1) }
  var grey: RGB {
    let l = 0.2126 * r + 0.7152 * g + 0.0722 * b
    return RGB(r: l, g: l, b: l)
  }
}

let white = RGB(0xFFFFFF)
let black = RGB(0x000000)

struct Look {
  let name: String
  /// The ground, top and bottom.
  let top: RGB
  let bottom: RGB
  /// The diffuser's body, top and foot, and the mist at a wisp's foot.
  let body: RGB
  let bodyFoot: RGB
  let mist: RGB
  /// A soft light behind the diffuser, as the room's glow.
  let glow: RGB?
  let greyscale: Bool
}

let looks = [
  // The Mac's: a touch lighter than the dark accent at the top, a touch darker than the
  // light accent at the bottom.
  Look(
    name: "AppIcon", top: RGB(0x5E5CE6).mix(white, 0.08), bottom: RGB(0x5856D6).mix(black, 0.12),
    body: white, bodyFoot: white.mix(RGB(0x5856D6), 0.1), mist: white, glow: RGB(0x8E8BFF),
    greyscale: false),
  // The room at night, as the diffuser's screen draws it in dark mode, with lavender mist.
  Look(
    name: "AppIcon-Dark", top: RGB(0x13113A), bottom: RGB(0x2D2884), body: white,
    bodyFoot: white.mix(RGB(0xBEB9FF), 0.25), mist: RGB(0xBEB9FF), glow: RGB(0x5E5CE6),
    greyscale: false),
  // For the system to tint: grey on black, light where the color should go.
  Look(
    name: "AppIcon-Tinted", top: RGB(0x0A0A0A), bottom: RGB(0x000000), body: white,
    bodyFoot: RGB(0xD8D8D8), mist: RGB(0xF2F2F2), glow: RGB(0x3A3A3A), greyscale: true),
]

/// A point on the glyph's grid, on the canvas. CoreGraphics runs up; the grid runs down.
func at(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
  let origin = CGPoint(x: canvas / 2 - 14 * unit, y: canvas / 2 - (14 + lift) * unit)
  return CGPoint(x: origin.x + x * unit, y: canvas - (origin.y + y * unit))
}

func linear(_ context: CGContext, _ from: RGB, _ to: RGB, start: CGPoint, end: CGPoint) {
  let gradient = CGGradient(
    colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [from.cg, to.cg] as CFArray,
    locations: [0, 1])!
  context.drawLinearGradient(
    gradient, start: start, end: end, options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
}

/// One wisp's line, from its foot up: the left, middle or right one, as the glyph draws
/// them (`Glyphs.swift`, `glyphs::wisp_line`).
func wispLine(_ index: Int) -> [CGPoint] {
  let side: CGFloat = index == 0 ? -1 : 1
  let foot: CGFloat = index == 1 ? 11.5 : 10.5
  let lean: CGFloat = index == 1 ? 0 : 0.75 * side
  let line: CGFloat = [10.5, 14, 17.5][index]
  return (0...40).map { step in
    let along = CGFloat(step) / 40
    return at(line + side * 0.7 * sin(2 * .pi * along) + lean * along, foot - 7 * along)
  }
}

func draw(_ look: Look) -> CGImage {
  let space = CGColorSpace(name: look.greyscale ? CGColorSpace.genericGrayGamma2_2 : CGColorSpace.sRGB)!
  let context = CGContext(
    data: nil, width: Int(canvas), height: Int(canvas), bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
  _ = space
  let tone = { (c: RGB) in look.greyscale ? c.grey : c }

  // The ground, full-bleed.
  linear(context, tone(look.top), tone(look.bottom), start: CGPoint(x: 0, y: canvas), end: .zero)

  // The room's glow, gathered where the mist leaves the diffuser.
  if let glow = look.glow {
    let center = at(14, 13)
    let gradient = CGGradient(
      colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
      colors: [tone(glow).cg.copy(alpha: 0.55)!, tone(glow).cg.copy(alpha: 0)!] as CFArray,
      locations: [0, 1])!
    context.drawRadialGradient(
      gradient, startCenter: center, startRadius: 0, endCenter: center, endRadius: 15 * unit,
      options: [])
  }

  // The capsule, out to the outer edge of the glyph's 1.5-unit stroke, over a soft shadow.
  let edge: CGFloat = 0.75
  let bottomLeft = at(11 - edge, 24 + edge)
  let topRight = at(17 + edge, 14 - edge)
  let body = CGRect(
    x: bottomLeft.x, y: bottomLeft.y, width: topRight.x - bottomLeft.x, height: topRight.y - bottomLeft.y)
  let capsule = CGPath(
    roundedRect: body, cornerWidth: body.width / 2, cornerHeight: body.width / 2, transform: nil)
  context.saveGState()
  context.setShadow(
    offset: CGSize(width: 0, height: -0.5 * unit), blur: 1.5 * unit, color: CGColor(gray: 0, alpha: 0.3))
  context.addPath(capsule)
  context.setFillColor(tone(look.body).cg)
  context.fillPath()
  context.restoreGState()
  context.saveGState()
  context.addPath(capsule)
  context.clip()
  linear(context, tone(look.body), tone(look.bodyFoot), start: CGPoint(x: 0, y: body.maxY), end: CGPoint(x: 0, y: body.minY))
  context.restoreGState()

  // The outlet: a slot as wide as the glyph's slit with its round ends, deeper than the
  // ground around it.
  let slotLeft = at(13 - edge, 16.75 + edge)
  let slotRight = at(15 + edge, 16.75 - edge)
  let slot = CGRect(x: slotLeft.x, y: slotLeft.y, width: slotRight.x - slotLeft.x, height: slotRight.y - slotLeft.y)
  let ground = look.top.mix(look.bottom, 1 - slot.midY / canvas).mix(black, 0.2)
  context.addPath(CGPath(roundedRect: slot, cornerWidth: slot.height / 2, cornerHeight: slot.height / 2, transform: nil))
  context.setFillColor(tone(ground).cg)
  context.fillPath()

  // The wisps: 2 units wide at the foot, 1.25 at the tip, from the mist's color into a
  // little of the ground as they rise.
  for index in 0..<3 {
    let line = wispLine(index)
    let last = line.count - 1
    for i in 0..<last {
      let t = CGFloat(i) / CGFloat(last)
      let width = (2 - 0.75 * t) * unit
      let groundHere = look.top.mix(look.bottom, 1 - line[i].y / canvas)
      let color = look.mist.mix(groundHere, 0.3 * t)
      context.setStrokeColor(tone(color).cg)
      context.setLineWidth(width)
      context.setLineCap(.round)
      context.move(to: line[i])
      context.addLine(to: line[i + 1])
      context.strokePath()
    }
  }
  return context.makeImage()!
}

let folder = URL(fileURLWithPath: "App/Assets.xcassets/AppIcon.appiconset")
for look in looks {
  let image = draw(look)
  let rep = NSBitmapImageRep(cgImage: image)
  let data = rep.representation(using: .png, properties: [:])!
  try data.write(to: folder.appendingPathComponent("\(look.name).png"))
  print("Wrote \(look.name).png")
}
