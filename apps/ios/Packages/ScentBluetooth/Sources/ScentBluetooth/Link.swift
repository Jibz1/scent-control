import Foundation

// One diffuser's link: where its connection stands, the request it is doing now, and the
// requests waiting behind it. The Mac's `Link` task, as state the central drives.

/// A step that waits for Bluetooth's answer.
enum StepKind: Equatable {
  /// A scan, until the diffuser advertises.
  case find
  case connect
  case services
  case characteristics
  case subscribe
  case read(GattCharacteristic)
  case write
}

@MainActor final class Link {
  enum Phase: Equatable {
    case closed
    /// Being found, connected and set up.
    case opening
    case open(LinkTimer)
    /// A disconnect was asked for and Bluetooth hasn't confirmed it yet.
    case closing
  }

  let id: UUID
  var phase: Phase = .closed
  /// One request at a time, so writes are never interleaved.
  let turns = TurnQueue()
  /// The step in flight, if any. There is at most one, as there is one request at a time.
  var step: (kind: StepKind, slot: Slot<Answer>)?
  /// Closes the link once it has sat idle.
  var idleTimer: (any Cancelling)?
  /// Ends a close that Bluetooth never confirms.
  var closingTimer: (any Cancelling)?
  /// Requests waiting for a close to finish before they open a fresh link.
  var closingWaits: [Slot<Answer>] = []

  init(id: UUID) {
    self.id = id
  }

  /// Whether notifications are being listened to: from the moment the link is set up
  /// until it starts to close.
  var isListening: Bool {
    switch phase {
    case .opening, .open: true
    case .closed, .closing: false
    }
  }

  var isOpen: Bool {
    if case .open = phase { true } else { false }
  }

  /// Answers the step in flight, if it is of this kind.
  func answer(_ kind: StepKind, _ answer: Answer) {
    guard let step, step.kind == kind else { return }
    step.slot.settle(.value(answer))
  }

  /// Answers the step in flight, whatever its kind.
  func answerAny(_ answer: Answer) {
    step?.slot.settle(.value(answer))
  }

  /// The close is over, confirmed or not: requests waiting for it may open a fresh link.
  func finishClosing() {
    guard phase == .closing else { return }
    phase = .closed
    closingTimer?.cancel()
    closingTimer = nil
    let waits = closingWaits
    closingWaits.removeAll()
    for slot in waits { slot.settle(.value(.done(nil))) }
  }

  func stopIdleTimer() {
    idleTimer?.cancel()
    idleTimer = nil
  }
}
