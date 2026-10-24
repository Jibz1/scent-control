import ScentEngine
import ScentProtocol
import SwiftUI

/// The top of a diffuser's screen: its room. Dusk while it diffuses, drained of color when
/// it's off; mist rising from the diffuser, thicker for higher intensity; what it is doing in
/// words; and the power button and intensity control at the bottom, on glass.
struct RoomHero: View {
  let diffuser: DiffuserState
  let now: Date
  let topInset: CGFloat
  let setPower: (Bool) -> Void
  let setLevel: (Level) -> Void

  @ScaledMetric(relativeTo: .largeTitle) private var glyphSize: CGFloat = 150
  @Environment(\.accessibilityReduceMotion) private var reduceMotion

  private var on: Bool { diffuser.shownPower == true }
  /// An old report is not shown as the room's state: the room stays grey and says how old
  /// it is, as the Mac's does.
  private var stale: Bool { diffuser.stateIsStale(now: now) }
  private var lit: Bool { on && diffuser.reachability != .notAnswering && !stale }
  private var mistIntensity: Double { lit ? Double(diffuser.shownLevel?.value ?? 1) : 0 }

  var body: some View {
    VStack(spacing: 0) {
      ZStack(alignment: .bottom) {
        // The room's light, gathered where the mist leaves the diffuser.
        RadialGradient(
          colors: [Theme.mist.opacity(0.42), Theme.mist.opacity(0)], center: .center, startRadius: 0,
          endRadius: glyphSize * 1.05
        )
        .frame(width: glyphSize * 2.2, height: glyphSize * 2.2)
        .offset(y: glyphSize * 0.55)
        .opacity(lit ? 0.35 + 0.22 * mistIntensity : 0)
        MistField(
          intensity: mistIntensity, color: Theme.mist,
          alert: diffuser.failure != nil ? Theme.alert : nil)
          .frame(width: 340, height: 300)
          .offset(y: -glyphSize * 0.42)
        DiffuserGlyph(kind: diffuser.kind, level: lit ? diffuser.shownLevel?.value ?? 1 : 0)
          .foregroundStyle(Theme.roomInk)
          .tint(Theme.mist)
          .frame(width: glyphSize, height: glyphSize)
      }
      .frame(height: glyphSize + 70, alignment: .bottom)
      .padding(.top, topInset)

      VStack(spacing: 6) {
        Text(Words.state(diffuser))
          .font(Theme.headline(.largeTitle))
          .foregroundStyle(
            diffuser.pending != nil ? Theme.pending : stale ? Theme.roomInkSecondary : Theme.roomInk
          )
          .contentTransition(.interpolate)
        Text(subline)
          .font(.callout)
          .foregroundStyle(Theme.roomInkSecondary)
          .contentTransition(.interpolate)
        if let note {
          Text(note)
            .font(.footnote)
            .foregroundStyle(Theme.roomInkSecondary)
            .contentTransition(.interpolate)
        }
        if let failure = diffuser.failure {
          Label(failure, systemImage: "exclamationmark.circle.fill")
            .font(.footnote.weight(.medium))
            .foregroundStyle(Theme.alert)
            .padding(.top, 4)
            .transition(.opacity.combined(with: .scale(scale: 0.95)))
        }
      }
      .multilineTextAlignment(.center)
      .padding(.horizontal, 24)
      .padding(.top, 14)
      .accessibilityElement(children: .combine)

      GlassEffectContainer(spacing: 14) {
        HStack(spacing: 12) {
          PowerButton(on: on, sending: diffuser.pending?.power != nil) { setPower(!on) }
          IntensityControl(level: diffuser.shownLevel, active: on, select: setLevel)
        }
      }
      .padding(.horizontal, 20)
      .padding(.top, 26)
      .padding(.bottom, 24)
    }
    .frame(maxWidth: .infinity)
    .background {
      ZStack {
        LinearGradient(colors: [Theme.dimTop, Theme.dimBottom], startPoint: .top, endPoint: .bottom)
        LinearGradient(colors: [Theme.roomTop, Theme.roomBottom], startPoint: .top, endPoint: .bottom)
          .opacity(lit ? 1 : 0)
      }
    }
    .clipShape(
      UnevenRoundedRectangle(
        bottomLeadingRadius: Theme.roomRadius, bottomTrailingRadius: Theme.roomRadius, style: .continuous)
    )
    .animation(reduceMotion ? .linear(duration: 0.01) : .smooth(duration: 1.1), value: lit)
    .animation(reduceMotion ? nil : .smooth(duration: 1.6), value: mistIntensity)
    .animation(.smooth(duration: 0.35), value: diffuser.pending)
    .animation(.smooth(duration: 0.35), value: diffuser.failure)
  }

  private var subline: String {
    if let pending = diffuser.pending { return Words.pending(pending) }
    return Words.detail(diffuser)
  }

  /// Where the state comes from: "Reported 3 min ago over WiFi", or "Not known since it
  /// reconnected 2 min ago" when its last report came before that. None while a change is on
  /// its way, which the line above says.
  private var note: String? {
    guard diffuser.pending == nil else { return nil }
    if let since = diffuser.notKnownSince {
      return "Not known since it reconnected \(Words.ago(since, now: now))"
    }
    guard let reported = diffuser.stateReported else { return nil }
    return "Reported \(Words.ago(reported.at, now: now)) over \(reported.via.name)"
  }
}

/// The big round power button, filled with the accent while on.
private struct PowerButton: View {
  let on: Bool
  let sending: Bool
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      Image(systemName: "power")
        .font(.system(size: 24, weight: .bold))
        .foregroundStyle(on ? Color.white : Theme.roomInk)
        .frame(width: 62, height: 62)
        .symbolEffect(.bounce, value: on)
        .opacity(sending ? 0.6 : 1)
    }
    .buttonStyle(.plain)
    .glassEffect(.regular.tint(on ? Theme.accent : nil).interactive(), in: .circle)
    .sensoryFeedback(.impact(weight: .medium), trigger: on)
    .accessibilityLabel(on ? "Turn Off" : "Turn On")
  }
}

/// Low, Medium and High on one glass capsule, the chosen one marked by a pill that slides
/// to it. Dimmed while the diffuser is off; choosing one then still sets it.
struct IntensityControl: View {
  let level: Level?
  let active: Bool
  let select: (Level) -> Void

  @Namespace private var pill

  var body: some View {
    HStack(spacing: 4) {
      ForEach([Level.low, .medium, .high], id: \.self) { option in
        let chosen = option == level
        Button {
          select(option)
        } label: {
          VStack(spacing: 3) {
            Wisps(count: option.value, lit: chosen && active)
            Text(Words.level(option))
              .font(Theme.headline(.subheadline, weight: .semibold))
          }
          .foregroundStyle(chosen && active ? Color.white : Theme.roomInk.opacity(active ? 0.85 : 0.5))
          .frame(maxWidth: .infinity, minHeight: 54)
          .background {
            if chosen {
              Capsule()
                .fill(active ? AnyShapeStyle(Theme.accent) : AnyShapeStyle(Theme.roomInk.opacity(0.12)))
                .matchedGeometryEffect(id: "pill", in: pill)
            }
          }
          .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(Words.level(option)) intensity")
        .accessibilityAddTraits(chosen ? .isSelected : [])
      }
    }
    .padding(4)
    .glassEffect(.regular.interactive(), in: .capsule)
    .animation(.spring(duration: 0.4, bounce: 0.25), value: level)
    .animation(.smooth(duration: 0.4), value: active)
    .sensoryFeedback(.selection, trigger: level)
  }
}

/// One, two or three small wisps, for the intensity choices.
private struct Wisps: View {
  let count: Int
  let lit: Bool

  var body: some View {
    HStack(spacing: 2.5) {
      ForEach(0..<count, id: \.self) { _ in
        Capsule().frame(width: 2.5, height: 9)
      }
    }
    .opacity(lit ? 1 : 0.6)
    .frame(height: 9)
  }
}
