import SwiftUI

#if canImport(UIKit)
  import UIKit
#endif

/// The look. A diffuser's screen opens on its room: a lavender dawn in light mode and a deep
/// indigo night in dark mode while it diffuses, greyed out when it's off, with mist rising
/// to match. Everything around the room is a plain iOS grouped list. Indigo is the accent,
/// as on the Mac. SF Pro Rounded for headlines and state words, SF Pro for the rest.
enum Theme {
  static let accent = adaptive(light: 0x5856D6, dark: 0x5E5CE6)

  /// The room while the diffuser is on.
  static let roomTop = adaptive(light: 0xF4F3FF, dark: 0x13113A)
  static let roomBottom = adaptive(light: 0xD9D5FF, dark: 0x2D2884)
  /// The room while it is off or not answering: the same light with the color drained.
  static let dimTop = adaptive(light: 0xF5F5F8, dark: 0x141418)
  static let dimBottom = adaptive(light: 0xE3E3EA, dark: 0x2A2A33)
  /// Text and the diffuser's body in the room.
  static let roomInk = adaptive(light: 0x1C1A4A, dark: 0xFFFFFF)
  static let roomInkSecondary = adaptive(light: 0x55538A, dark: 0xC9C7F2)
  /// The mist rising in the room.
  static let mist = adaptive(light: 0x6F6AEE, dark: 0xBEB9FF)
  /// A failure: a rose that sits apart from the indigo.
  static let alert = adaptive(light: 0xD93A63, dark: 0xFF7A93)
  /// Something on its way, as the Mac's orange "Sending…".
  static let pending = adaptive(light: 0xC76A00, dark: 0xFFA94D)
  static let answering = adaptive(light: 0x248A3D, dark: 0x30D158)

  static let roomRadius: CGFloat = 38
  static let tileRadius: CGFloat = 26
  static let cardRadius: CGFloat = 22

  /// Rounded bold, for titles and the words that say what a diffuser is doing.
  static func headline(_ style: Font.TextStyle, weight: Font.Weight = .bold) -> Font {
    .system(style, design: .rounded, weight: weight)
  }

  static func adaptive(light: UInt32, dark: UInt32) -> Color {
    #if canImport(UIKit)
      Color(
        UIColor { traits in
          UIColor(hex: traits.userInterfaceStyle == .dark ? dark : light)
        })
    #else
      Color(hex: light)
    #endif
  }
}

#if canImport(UIKit)
  extension UIColor {
    fileprivate convenience init(hex: UInt32) {
      self.init(
        red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
    }
  }
#endif

extension Color {
  fileprivate init(hex: UInt32) {
    self.init(
      red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255,
      blue: Double(hex & 0xFF) / 255)
  }
}

/// A button that sinks a little while pressed, for tiles.
struct PressableStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .scaleEffect(configuration.isPressed ? 0.965 : 1)
      .animation(.spring(duration: 0.28, bounce: 0.35), value: configuration.isPressed)
  }
}

/// Rows on one rounded surface, as an inset grouped list draws them, for screens that
/// scroll freely rather than in a `List`.
struct Card<Content: View>: View {
  @ViewBuilder var content: Content

  var body: some View {
    VStack(alignment: .leading, spacing: 0) { content }
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(
        Color(.secondarySystemGroupedBackground),
        in: RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous))
  }
}

/// A section's title above a card: rounded, with an optional control at the trailing end.
struct SectionTitle<Trailing: View>: View {
  let title: String
  @ViewBuilder var trailing: Trailing

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title).font(Theme.headline(.title3))
      Spacer()
      trailing
    }
    .padding(.horizontal, 6)
  }
}

extension SectionTitle where Trailing == EmptyView {
  init(_ title: String) {
    self.init(title: title) { EmptyView() }
  }
}

/// A label and its value on one row of a card, with an optional note under the value.
struct FactRow: View {
  let label: String
  let value: String
  var note: String?
  var stale = false
  var symbol: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 12) {
      if let symbol {
        Image(systemName: symbol)
          .foregroundStyle(.secondary)
          .frame(width: 22)
      }
      Text(label)
      Spacer(minLength: 12)
      VStack(alignment: .trailing, spacing: 2) {
        Text(value)
          .foregroundStyle(stale ? .secondary : .primary)
        if let note {
          Text(note).font(.caption).foregroundStyle(.secondary)
        }
      }
      .multilineTextAlignment(.trailing)
    }
    .padding(.horizontal, 16)
    .padding(.vertical, 12)
    .accessibilityElement(children: .combine)
  }
}

/// The hairline between rows of a card, inset like a grouped list's.
struct RowDivider: View {
  var inset: CGFloat = 16
  var body: some View {
    Divider().padding(.leading, inset)
  }
}
