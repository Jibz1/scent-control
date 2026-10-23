import Foundation

// How long each Bluetooth step may take, and when a link closes. The numbers are the
// Mac's (`apps/desktop/src/ble.rs`).

enum Limits {
  /// How long it may take to find a diffuser and open a link to it. The clock starts once
  /// Bluetooth is ready.
  static let connect: Duration = .seconds(10)
  /// How long one read, or the writes of one command, may take on an open link.
  static let io: Duration = .seconds(10)
  /// How long to wait for Core Bluetooth to confirm a disconnect. It confirms only
  /// through the callback that says the peripheral disconnected, and may send none for a
  /// connection that was still being made, so the wait must end on its own.
  static let disconnect: Duration = .seconds(3)
  /// How long to give Bluetooth to say it is usable when it first says it doesn't know.
  static let settle: Duration = .seconds(3)
  /// The same while iOS is asking the person for permission, which only they can answer.
  static let prompt: Duration = .seconds(60)
  /// How many requests can wait for one diffuser's link.
  static let queue = 16
}

/// When a link is due to be closed, as long as nothing uses it. A port of the Mac's
/// `LinkTimer`.
struct LinkTimer: Equatable {
  /// How long a link may sit unused before it is closed.
  static let idleAfter: Duration = .seconds(10)
  /// How long to keep listening to a diffuser after a command went out.
  static let listenAfterSend: Duration = .seconds(6)

  /// When the link is due to close if nothing uses it before then.
  private(set) var deadline: ContinuousClock.Instant

  /// A link opened at `now`.
  init(openedAt now: ContinuousClock.Instant) {
    deadline = now + Self.idleAfter
  }

  /// The link was used at `now`: it stays open 10 s from here.
  mutating func touch(_ now: ContinuousClock.Instant) {
    hold(until: now + Self.idleAfter)
  }

  /// A command went out at `now`: the link stays open at least long enough to hear the
  /// diffuser's answer.
  mutating func sent(_ now: ContinuousClock.Instant) {
    hold(until: now + max(Self.idleAfter, Self.listenAfterSend))
  }

  func shouldDisconnect(_ now: ContinuousClock.Instant) -> Bool {
    now >= deadline
  }

  /// Never brings the deadline forward.
  private mutating func hold(until instant: ContinuousClock.Instant) {
    deadline = max(deadline, instant)
  }
}
