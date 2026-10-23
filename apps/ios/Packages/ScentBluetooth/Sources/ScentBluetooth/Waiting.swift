import Foundation

// Waiting on the main actor: for a Core Bluetooth callback, a time limit, the caller's
// cancellation, or a turn at a diffuser. Everything here runs on the main actor, where
// Core Bluetooth calls back, so nothing needs a lock.

/// Something scheduled that can be called off.
@MainActor protocol Cancelling {
  func cancel()
}

/// The clock and timers the Bluetooth layer uses, so that tests can drive time.
@MainActor protocol Timing: AnyObject {
  /// A monotonic now, for deadlines.
  var now: ContinuousClock.Instant { get }
  /// The wall clock, for the time stamps on events.
  var date: Date { get }
  /// Calls `fire` on the main actor once `delay` has passed, unless cancelled first.
  func schedule(after delay: Duration, _ fire: @escaping @MainActor () -> Void) -> any Cancelling
}

/// The real clock.
@MainActor final class SystemTiming: Timing {
  var now: ContinuousClock.Instant { ContinuousClock.now }
  var date: Date { Date() }

  func schedule(after delay: Duration, _ fire: @escaping @MainActor () -> Void) -> any Cancelling {
    ScheduledTask(
      task: Task { @MainActor in
        do {
          try await Task.sleep(for: delay)
        } catch {
          return
        }
        fire()
      })
  }

  private struct ScheduledTask: Cancelling {
    let task: Task<Void, Never>
    func cancel() { task.cancel() }
  }
}

/// How a wait ended.
enum WaitEnd<Value: Sendable>: Sendable {
  case value(Value)
  case timedOut
  case cancelled
}

/// What a Bluetooth step, or a wait on Bluetooth, came to.
enum Answer: Sendable, Equatable {
  /// It worked, with the value read, if any.
  case done(Data?)
  /// Bluetooth reported an error, with its reason.
  case failed(String)
  /// The diffuser's link went down, with Bluetooth's reason if it gave one.
  case dropped(String?)
  /// Bluetooth became unusable, or was told to let go of every diffuser. Said as is.
  case stopped(String)
}

/// One pending answer. It is settled once, by whichever comes first: the answer, the time
/// limit, or the caller's cancellation. Later attempts to settle it are ignored.
@MainActor final class Slot<Value: Sendable> {
  private var end: WaitEnd<Value>?
  private var continuation: CheckedContinuation<WaitEnd<Value>, Never>?
  private var timer: (any Cancelling)?

  var isSettled: Bool { end != nil }

  /// Settles the slot, unless it was settled already. True when this call settled it.
  @discardableResult
  func settle(_ end: WaitEnd<Value>) -> Bool {
    guard self.end == nil else { return false }
    self.end = end
    timer?.cancel()
    timer = nil
    if let continuation {
      self.continuation = nil
      continuation.resume(returning: end)
    }
    return true
  }

  /// Waits until the slot is settled. With a deadline, it is settled as timed out once the
  /// deadline passes. If the calling task is cancelled, it is settled as cancelled at once.
  func wait(until deadline: ContinuousClock.Instant? = nil, on timing: (any Timing)? = nil) async
    -> WaitEnd<Value>
  {
    if end == nil, let deadline, let timing {
      let delay = deadline - timing.now
      if delay <= .zero {
        settle(.timedOut)
      } else {
        timer = timing.schedule(after: delay) { [weak self] in self?.settle(.timedOut) }
      }
    }
    return await withTaskCancellationHandler {
      await withCheckedContinuation { continuation in
        if let end {
          continuation.resume(returning: end)
        } else {
          self.continuation = continuation
        }
      }
    } onCancel: {
      Task { @MainActor in self.settle(.cancelled) }
    }
  }
}

/// One diffuser's turns: one request at a time, first come first served.
@MainActor final class TurnQueue {
  /// What a caller waiting for a turn is told.
  enum Turn: Sendable {
    case granted
    /// The request ahead couldn't open the link, so this one fails with it, as on the Mac.
    case refused(BluetoothError)
  }

  /// Whether a request holds the turn.
  private(set) var isBusy = false
  private var waiting: [Slot<Turn>] = []

  /// How many requests are waiting behind the one that holds the turn.
  var waitingCount: Int { waiting.count { !$0.isSettled } }

  /// Waits for this caller's turn. A caller that is cancelled while it waits leaves the
  /// queue at once.
  func take() async -> WaitEnd<Turn> {
    guard isBusy else {
      isBusy = true
      return .value(.granted)
    }
    let slot = Slot<Turn>()
    waiting.append(slot)
    let end = await slot.wait()
    waiting.removeAll { $0 === slot }
    return end
  }

  /// Hands the turn to the next caller still waiting, or frees it.
  func release() {
    while !waiting.isEmpty {
      if waiting.removeFirst().settle(.value(.granted)) { return }
    }
    isBusy = false
  }

  /// Fails every caller waiting now with `error`. The turn itself is not released.
  func refuseWaiting(_ error: BluetoothError) {
    let refused = waiting
    waiting.removeAll()
    for slot in refused { slot.settle(.value(.refused(error))) }
  }
}
