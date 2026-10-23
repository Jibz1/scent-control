// Stand-ins for the network and the clock, so that the client's behavior is tested without
// either: a broker played by the test, and time that moves only when the test says so.
import Foundation
import Synchronization
import Testing

@testable import ScentMQTT

/// What a test waited for and never saw.
struct TimedOut: Error, CustomStringConvertible {
  let what: String
  var description: String { "Timed out waiting for \(what)." }
}

/// A first-in, first-out queue that a test can wait on.
final class AsyncQueue<Element: Sendable>: Sendable {
  private struct State {
    var items: [Element] = []
    var waiters: [(id: Int, continuation: CheckedContinuation<Element?, Never>)] = []
    var finished = false
    var nextWaiter = 0
  }

  private let state = Mutex(State())

  func push(_ item: Element) {
    let waiter: CheckedContinuation<Element?, Never>? = state.withLock { state in
      guard !state.finished else { return nil }
      guard !state.waiters.isEmpty else {
        state.items.append(item)
        return nil
      }
      return state.waiters.removeFirst().continuation
    }
    waiter?.resume(returning: item)
  }

  /// Ends the queue: waiters, and later calls to `next` once it is empty, get nil.
  func finish() {
    let waiters = state.withLock { state in
      state.finished = true
      defer { state.waiters.removeAll() }
      return state.waiters.map(\.continuation)
    }
    for waiter in waiters { waiter.resume(returning: nil) }
  }

  /// What is waiting, without taking it.
  var pending: [Element] { state.withLock { $0.items } }

  /// The next item, waiting for it. Nil once the queue is finished, or when the task is
  /// cancelled.
  func next() async -> Element? {
    let id = state.withLock { state in
      state.nextWaiter += 1
      return state.nextWaiter
    }
    return await withTaskCancellationHandler {
      await withCheckedContinuation { continuation in
        let ready: Element?? = state.withLock { state in
          if !state.items.isEmpty { return .some(state.items.removeFirst()) }
          if state.finished || Task.isCancelled { return .some(nil) }
          state.waiters.append((id, continuation))
          return .none
        }
        if let ready { continuation.resume(returning: ready) }
      }
    } onCancel: {
      let waiter: CheckedContinuation<Element?, Never>? = state.withLock { state in
        guard let index = state.waiters.firstIndex(where: { $0.id == id }) else { return nil }
        return state.waiters.remove(at: index).continuation
      }
      waiter?.resume(returning: nil)
    }
  }

  /// The next item within `seconds` of real time, or a thrown `TimedOut`.
  func next(_ what: String, within seconds: Double = 5) async throws -> Element {
    let item = await withTaskGroup(of: Element?.self) { group in
      group.addTask { await self.next() }
      group.addTask {
        try? await Task.sleep(for: .seconds(seconds))
        return nil
      }
      let first = await group.next() ?? nil
      group.cancelAll()
      // A cancelled `next` may have taken an item after all; put it back in front.
      for await late in group {
        if let late { self.pushFront(late) }
      }
      return first
    }
    guard let item else { throw TimedOut(what: what) }
    return item
  }

  private func pushFront(_ item: Element) {
    state.withLock { $0.items.insert(item, at: 0) }
  }
}

/// A clock that moves only when the test advances it.
final class TestClock: MQTTSleeper {
  private struct Sleeper {
    let id: Int
    let deadline: Duration
    let continuation: CheckedContinuation<Void, any Error>
  }

  private struct State {
    var now = Duration.zero
    var sleepers: [Sleeper] = []
    var cancelled: Set<Int> = []
    var nextID = 0
  }

  private let state = Mutex(State())

  var now: Duration { state.withLock { $0.now } }

  /// How long each sleeper still has to wait, soonest first.
  var waits: [Duration] {
    state.withLock { state in state.sleepers.map { $0.deadline - state.now }.sorted() }
  }

  func sleep(for duration: Duration) async throws {
    let id = state.withLock { state in
      state.nextID += 1
      return state.nextID
    }
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation {
        (continuation: CheckedContinuation<Void, any Error>) in
        let cancelled: Bool = state.withLock { state in
          if state.cancelled.remove(id) != nil { return true }
          state.sleepers.append(
            Sleeper(id: id, deadline: state.now + duration, continuation: continuation))
          return false
        }
        if cancelled { continuation.resume(throwing: CancellationError()) }
      }
    } onCancel: {
      let sleeper: Sleeper? = state.withLock { state in
        guard let index = state.sleepers.firstIndex(where: { $0.id == id }) else {
          state.cancelled.insert(id)
          return nil
        }
        return state.sleepers.remove(at: index)
      }
      sleeper?.continuation.resume(throwing: CancellationError())
    }
  }

  /// Moves time on and wakes every sleeper that is due, soonest first.
  func advance(by duration: Duration) {
    let due: [Sleeper] = state.withLock { state in
      state.now += duration
      let now = state.now
      let due = state.sleepers.filter { $0.deadline <= now }.sorted { $0.deadline < $1.deadline }
      state.sleepers.removeAll { $0.deadline <= now }
      return due
    }
    for sleeper in due { sleeper.continuation.resume() }
  }

  /// Waits, in real time, until a sleeper has exactly `wait` left to go.
  func waitForSleeper(_ wait: Duration, within seconds: Double = 5) async throws {
    let clock = ContinuousClock()
    let deadline = clock.now + .seconds(seconds)
    while !waits.contains(wait) {
      guard clock.now < deadline else {
        throw TimedOut(what: "a sleeper with \(wait) to go (waits: \(waits))")
      }
      try await Task.sleep(for: .milliseconds(1))
    }
  }
}

/// A connection played by the test: what the client sends is kept, and what the test
/// delivers is what the client receives.
final class FakeTransport: MQTTTransport {
  /// Each message the client sent.
  let sent = AsyncQueue<Data>()
  private let inbound = AsyncQueue<Result<Data, any Error>>()
  private let state = Mutex(State())

  private struct State {
    var closed = false
    /// While set, sends wait until the test opens the gate.
    var gateClosed = false
    var gateWaiters: [CheckedContinuation<Void, Never>] = []
    var sendsFail = false
  }

  var isClosed: Bool { state.withLock { $0.closed } }

  /// How many sends are held at the closed gate.
  var sendsWaiting: Int { state.withLock { $0.gateWaiters.count } }

  func send(_ data: Data) async throws {
    if state.withLock({ $0.sendsFail }) {
      throw TransportError(message: "The send failed.")
    }
    let mustWait = state.withLock { $0.gateClosed && !$0.closed }
    if mustWait {
      await withCheckedContinuation { continuation in
        let waiting = state.withLock { state in
          guard state.gateClosed && !state.closed else { return false }
          state.gateWaiters.append(continuation)
          return true
        }
        if !waiting { continuation.resume() }
      }
    }
    guard !isClosed else { throw TransportError(message: "The fake connection is closed.") }
    sent.push(data)
  }

  func receive() async throws -> Data {
    guard let next = await inbound.next() else {
      throw TransportError(message: "The fake connection is closed.")
    }
    return try next.get()
  }

  func close() {
    let waiters = state.withLock { state in
      state.closed = true
      defer { state.gateWaiters.removeAll() }
      return state.gateWaiters
    }
    for waiter in waiters { waiter.resume() }
    inbound.finish()
  }

  // MARK: The test's side

  /// Makes every later send fail, while receiving goes on.
  func failSends() {
    state.withLock { $0.sendsFail = true }
  }

  /// Holds every send until `openGate`.
  func closeGate() {
    state.withLock { $0.gateClosed = true }
  }

  func openGate() {
    let waiters = state.withLock { state in
      state.gateClosed = false
      defer { state.gateWaiters.removeAll() }
      return state.gateWaiters
    }
    for waiter in waiters { waiter.resume() }
  }

  /// Delivers raw bytes as one WebSocket message.
  func deliver(bytes: [UInt8]) {
    inbound.push(.success(Data(bytes)))
  }

  /// Delivers packets, all in one WebSocket message.
  func deliver(_ packets: MQTTPacket...) throws {
    var data = Data()
    for packet in packets { data.append(try packet.encode()) }
    inbound.push(.success(data))
  }

  /// Fails the connection as a network would.
  func fail(_ reason: String) {
    inbound.push(.failure(TransportError(message: reason)))
  }

  /// The next packet the client sent, decoded. Each WebSocket message must hold exactly one.
  func nextPacket(_ what: String = "a packet", within seconds: Double = 5) async throws
    -> MQTTPacket
  {
    try MQTTPacketDecoder.decode(try await sent.next(what, within: seconds))
  }

  /// Answers the client's CONNECT, and its SUBSCRIBE when there is one.
  func accept(sessionPresent: Bool = false) async throws {
    guard case .connect = try await nextPacket("CONNECT") else {
      Issue.record("The first packet wasn't a CONNECT.")
      return
    }
    try deliver(.connAck(sessionPresent: sessionPresent, returnCode: 0))
  }
}

/// Opens fake connections, or fails or hangs as the test plans.
final class FakeConnector: MQTTConnector {
  enum Plan: Sendable {
    case succeed
    case fail(String)
    /// Never opens; ends only when cancelled.
    case hang
  }

  /// Each connection opened, in order.
  let connections = AsyncQueue<FakeTransport>()
  private let state = Mutex(State())

  private struct State {
    var plans: [Plan] = []
    var attempts: [URL] = []
  }

  /// What the next calls to `connect` do, in order. After these, they succeed.
  func plan(_ plans: Plan...) {
    state.withLock { $0.plans.append(contentsOf: plans) }
  }

  /// Every URL `connect` was called with.
  var attempts: [URL] { state.withLock { $0.attempts } }

  func connect(to url: URL) async throws -> any MQTTTransport {
    let plan: Plan = state.withLock { state in
      state.attempts.append(url)
      return state.plans.isEmpty ? .succeed : state.plans.removeFirst()
    }
    switch plan {
    case .succeed:
      let transport = FakeTransport()
      connections.push(transport)
      return transport
    case .fail(let reason):
      throw TransportError(message: reason)
    case .hang:
      try await Task.sleep(for: .seconds(3600))
      throw TransportError(message: "The hanging connection ended.")
    }
  }
}
