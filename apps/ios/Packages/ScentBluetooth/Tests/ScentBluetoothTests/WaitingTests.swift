import Foundation
import Testing

@testable import ScentBluetooth

// Waiting for answers, time limits and turns, with a clock the test moves.

@MainActor @Suite struct SlotTests {
  @Test func anAnswerEndsTheWaitAndLaterOnesAreIgnored() async {
    let timing = FakeTiming()
    let slot = Slot<Answer>()
    let waiter = Task { await slot.wait(until: timing.now + .seconds(10), on: timing) }
    await Task.yield()
    #expect(slot.settle(.value(.done(Data([1])))))
    #expect(!slot.settle(.timedOut))
    guard case .value(.done(let value)) = await waiter.value else {
      Issue.record("not answered")
      return
    }
    #expect(value == Data([1]))
    // The time limit was called off with the answer.
    timing.advance(by: .seconds(20))
  }

  @Test func anAnswerGivenBeforeTheWaitIsKept() async {
    let slot = Slot<Answer>()
    slot.settle(.value(.failed("no")))
    guard case .value(.failed("no")) = await slot.wait() else {
      Issue.record("lost")
      return
    }
  }

  @Test func theWaitEndsAtTheDeadline() async {
    let timing = FakeTiming()
    let slot = Slot<Answer>()
    let waiter = Task { await slot.wait(until: timing.now + .seconds(3), on: timing) }
    await Task.yield()
    timing.advance(by: .milliseconds(2_999))
    #expect(!slot.isSettled)
    timing.advance(by: .milliseconds(1))
    guard case .timedOut = await waiter.value else {
      Issue.record("not timed out")
      return
    }
  }

  @Test func aDeadlineAlreadyPassedEndsTheWaitAtOnce() async {
    let timing = FakeTiming()
    let slot = Slot<Answer>()
    guard case .timedOut = await slot.wait(until: timing.now, on: timing) else {
      Issue.record("not timed out")
      return
    }
  }

  @Test func cancellingTheCallerEndsTheWait() async {
    let slot = Slot<Answer>()
    let waiter = Task { await slot.wait() }
    await Task.yield()
    waiter.cancel()
    guard case .cancelled = await waiter.value else {
      Issue.record("not cancelled")
      return
    }
  }
}

@MainActor @Suite struct TurnQueueTests {
  @Test func turnsGoInTheOrderTheyWereAskedFor() async {
    let turns = TurnQueue()
    var order: [Int] = []
    guard case .value(.granted) = await turns.take() else { return }
    var waiters: [Task<Void, Never>] = []
    for n in 1...3 {
      waiters.append(
        Task {
          guard case .value(.granted) = await turns.take() else { return }
          order.append(n)
          turns.release()
        })
      await Task.yield()
    }
    #expect(turns.waitingCount == 3)
    #expect(order.isEmpty, "nobody goes while the turn is held")
    turns.release()
    for waiter in waiters { await waiter.value }
    #expect(order == [1, 2, 3])
    #expect(!turns.isBusy)
  }

  @Test func aCallerCancelledWhileWaitingLeavesAndTheNextGoes() async {
    let turns = TurnQueue()
    _ = await turns.take()
    let gaveUp = Task { await turns.take() }
    await Task.yield()
    let next = Task { await turns.take() }
    await Task.yield()
    gaveUp.cancel()
    guard case .cancelled = await gaveUp.value else {
      Issue.record("not cancelled")
      return
    }
    #expect(turns.waitingCount == 1)
    turns.release()
    guard case .value(.granted) = await next.value else {
      Issue.record("the turn was lost")
      return
    }
    #expect(turns.isBusy)
  }

  @Test func refusingFailsEveryoneWaitingButKeepsTheTurn() async {
    let turns = TurnQueue()
    _ = await turns.take()
    let waiter = Task { await turns.take() }
    await Task.yield()
    let error = BluetoothError(message: "Timed out while connecting.")
    turns.refuseWaiting(error)
    guard case .value(.refused(let refused)) = await waiter.value else {
      Issue.record("not refused")
      return
    }
    #expect(refused == error)
    #expect(turns.isBusy)
    turns.release()
    #expect(!turns.isBusy)
  }
}
