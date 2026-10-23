import Foundation
import Testing

@testable import ScentBluetooth

// Stand-ins for Core Bluetooth and the clock, so that the central's whole flow runs in a
// test: a radio that answers like a healthy diffuser unless told otherwise, and a clock
// that only moves when the test moves it.

/// A clock that stands still until `advance` moves it.
@MainActor final class FakeTiming: Timing {
  private final class Timer: Cancelling {
    let due: Duration
    let fire: @MainActor () -> Void
    var cancelled = false

    init(due: Duration, fire: @escaping @MainActor () -> Void) {
      self.due = due
      self.fire = fire
    }

    func cancel() { cancelled = true }
  }

  let start = ContinuousClock.now
  private(set) var elapsed: Duration = .zero
  private var timers: [Timer] = []

  var now: ContinuousClock.Instant { start + elapsed }
  var date: Date {
    let (seconds, attoseconds) = elapsed.components
    return Date(timeIntervalSince1970: 1_800_000_000 + Double(seconds) + Double(attoseconds) / 1e18)
  }

  func schedule(after delay: Duration, _ fire: @escaping @MainActor () -> Void) -> any Cancelling {
    let timer = Timer(due: elapsed + delay, fire: fire)
    timers.append(timer)
    return timer
  }

  /// Moves the clock on by `step`, firing every timer that falls due on the way, in order.
  func advance(by step: Duration) {
    let target = elapsed + step
    while let next = timers.filter({ !$0.cancelled && $0.due <= target }).min(by: {
      $0.due < $1.due
    }) {
      elapsed = max(elapsed, next.due)
      next.cancelled = true
      next.fire()
    }
    timers.removeAll { $0.cancelled }
    elapsed = target
  }
}

/// Core Bluetooth as a healthy diffuser would answer it, a moment later. A test can hold
/// back any kind of answer and give it itself, or have one fail.
@MainActor final class FakeRadio: Radio {
  enum Call: Equatable {
    case start
    case startScan
    case stopScan
    case retrieve(UUID)
    case connect(UUID)
    case cancel(UUID)
    case discoverServices(UUID)
    case discoverCharacteristics(UUID)
    case subscribe(UUID)
    case read(UUID, GattCharacteristic)
    case write(UUID, Data)
  }

  /// A kind of request whose answers can be held back.
  enum Kind: Hashable {
    case connect, services, characteristics, subscribe, read, write
  }

  var onEvent: (@MainActor (RadioEvent) -> Void)?
  private(set) var calls: [Call] = []
  private(set) var isStarted = false
  private(set) var state: BluetoothAvailability = .unknown
  var authorizationPending = false

  /// What Bluetooth says once started; `nil` says nothing, as the simulator does.
  var stateOnStart: BluetoothAvailability? = .poweredOn
  /// What `retrieve` finds: peripherals Bluetooth remembers.
  var remembered: Set<UUID> = []
  /// Peripherals this launch has.
  var known: Set<UUID> = []
  /// What a scan hears.
  var advertising: [UUID: Sighting] = [:]
  var connections: [UUID: PeripheralConnection] = [:]
  /// The characteristics the diffuser's service has.
  var gatt: Set<GattCharacteristic> = Set(GattCharacteristic.allCases)
  /// What reading each characteristic returns.
  var values: [GattCharacteristic: Data] = [:]
  /// Answers that are held back, for the test to give.
  var holding: Set<Kind> = []
  /// Failures given instead of success.
  var failures: [Kind: RadioFailure] = [:]
  /// Whether a disconnect this app asks for is confirmed. Core Bluetooth may never
  /// confirm one.
  var confirmsDisconnect = true

  /// Says `event` now, as Core Bluetooth's delegate would.
  func emit(_ event: RadioEvent) {
    if case .stateChanged(let state) = event { self.state = state }
    onEvent?(event)
  }

  /// Says `event` a moment later, after anything already queued on the main actor.
  private func later(_ event: RadioEvent, before: @escaping @MainActor () -> Void = {}) {
    Task { @MainActor in
      before()
      self.emit(event)
    }
  }

  func start() {
    calls.append(.start)
    isStarted = true
    if let stateOnStart { later(.stateChanged(stateOnStart)) }
  }

  func startScan() {
    calls.append(.startScan)
    for (id, sighting) in advertising {
      later(.discovered(id, name: sighting.name, rssi: sighting.rssi)) { self.known.insert(id) }
    }
  }

  func stopScan() { calls.append(.stopScan) }

  func retrieve(_ id: UUID) -> Bool {
    calls.append(.retrieve(id))
    guard remembered.contains(id) else { return false }
    known.insert(id)
    return true
  }

  func isKnown(_ id: UUID) -> Bool { known.contains(id) }

  func connection(_ id: UUID) -> PeripheralConnection { connections[id] ?? .disconnected }

  func connect(_ id: UUID) {
    calls.append(.connect(id))
    connections[id] = .connecting
    guard !holding.contains(.connect) else { return }
    if let failure = failures[.connect] {
      later(.failedToConnect(id, failure)) { self.connections[id] = .disconnected }
    } else {
      later(.connected(id)) { self.connections[id] = .connected }
    }
  }

  func cancelConnection(_ id: UUID) {
    calls.append(.cancel(id))
    switch connection(id) {
    case .connecting:
      connections[id] = .disconnected
    case .connected:
      connections[id] = .disconnecting
      if confirmsDisconnect {
        later(.disconnected(id, nil)) { self.connections[id] = .disconnected }
      }
    case .disconnected, .disconnecting:
      break
    }
  }

  func discoverServices(_ id: UUID) {
    calls.append(.discoverServices(id))
    answer(.services, .discoveredServices(id, failures[.services]))
  }

  func discoverCharacteristics(_ id: UUID) {
    calls.append(.discoverCharacteristics(id))
    answer(.characteristics, .discoveredCharacteristics(id, failures[.characteristics]))
  }

  func characteristics(_ id: UUID) -> Set<GattCharacteristic> { gatt }

  func subscribe(_ id: UUID) {
    calls.append(.subscribe(id))
    answer(.subscribe, .subscribed(id, failures[.subscribe]))
  }

  func read(_ id: UUID, _ characteristic: GattCharacteristic) {
    calls.append(.read(id, characteristic))
    let failure = failures[.read]
    answer(
      .read,
      .valueUpdated(id, characteristic, failure == nil ? values[characteristic] : nil, failure))
  }

  func write(_ id: UUID, _ chunk: Data) {
    calls.append(.write(id, chunk))
    answer(.write, .wrote(id, failures[.write]))
  }

  private func answer(_ kind: Kind, _ event: RadioEvent) {
    guard !holding.contains(kind) else { return }
    later(event)
  }

  // What tests ask about the calls.

  func count(_ call: Call) -> Int { calls.count { $0 == call } }

  func writes(to id: UUID) -> [Data] {
    calls.compactMap {
      if case .write(id, let chunk) = $0 { chunk } else { nil }
    }
  }

  /// Completes a connection the test held back.
  func connected(_ id: UUID) {
    connections[id] = .connected
    emit(.connected(id))
  }

  /// The diffuser drops a connected link on its own, as it does when it restarts.
  func drop(_ id: UUID, reason: String = "The specified device has disconnected from us.") {
    connections[id] = .disconnected
    emit(.disconnected(id, RadioFailure(reason, linkLost: true)))
  }
}

/// A central wired to the fakes, with what it reported.
@MainActor final class Rig {
  let radio = FakeRadio()
  let timing = FakeTiming()
  private(set) var events: [BluetoothEvent] = []
  private(set) var central: BluetoothCentral!

  init() {
    central = BluetoothCentral(radio: radio, timing: timing) { [weak self] event in
      self?.events.append(event)
    }
  }

  /// The notifications reported, as (peripheral, bytes).
  var notifications: [(UUID, Data)] {
    events.compactMap {
      if case .notification(let id, let bytes, _) = $0 { (id, bytes) } else { nil }
    }
  }

  var availabilities: [BluetoothAvailability] {
    events.compactMap {
      if case .availability(let state) = $0 { state } else { nil }
    }
  }

  /// Lets everything queued on the main actor run.
  func settle() async {
    for _ in 0..<200 { await Task.yield() }
  }

  /// Lets the main actor run until `condition` holds.
  func until(
    _ what: Comment, _ condition: () -> Bool, sourceLocation: SourceLocation = #_sourceLocation
  ) async {
    for _ in 0..<20_000 {
      if condition() { return }
      await Task.yield()
    }
    Issue.record("Never happened: \(what)", sourceLocation: sourceLocation)
  }

  /// Moves the clock on by `duration` in steps of 100 ms, letting everything run between.
  func pass(_ duration: Duration) async {
    var left = duration
    while left > .zero {
      let step = min(left, .milliseconds(100))
      timing.advance(by: step)
      left -= step
      await settle()
    }
  }

  /// A diffuser Bluetooth remembers, so a request reaches it with no scan.
  func diffuser(_ n: UInt8 = 1) -> UUID {
    let id = UUID(uuid: (n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    radio.remembered.insert(id)
    return id
  }
}

/// A request started in the background. A test looks at how it ended through the rig,
/// which never waits for it forever.
@MainActor final class Pending<Value> {
  private(set) var outcome: Result<Value, BluetoothError>?
  private var task: Task<Void, Never>?

  init(_ body: @escaping @MainActor () async throws -> Value) {
    task = Task {
      do {
        outcome = .success(try await body())
      } catch let error as BluetoothError {
        outcome = .failure(error)
      } catch {
        outcome = .failure(BluetoothError(message: "Not a BluetoothError: \(error)"))
      }
    }
  }

  var isDone: Bool { outcome != nil }

  /// Cancels the task the request runs in.
  func cancel() { task?.cancel() }
}

extension Rig {
  /// Lets the main actor run until `pending` has ended, and says how. One that never ends
  /// is recorded as an issue, and counts as a failure.
  func outcome<Value>(
    _ pending: Pending<Value>, sourceLocation: SourceLocation = #_sourceLocation
  ) async -> Result<Value, BluetoothError> {
    await until("answered", { pending.isDone }, sourceLocation: sourceLocation)
    return pending.outcome ?? .failure(BluetoothError(message: "Never answered."))
  }

  /// The error `pending` ended with, or `nil` when it worked.
  func error<Value>(
    _ pending: Pending<Value>, sourceLocation: SourceLocation = #_sourceLocation
  ) async -> BluetoothError? {
    switch await outcome(pending, sourceLocation: sourceLocation) {
    case .success: nil
    case .failure(let error): error
    }
  }

  /// What `pending` returned; it throws what it threw.
  func value<Value>(
    _ pending: Pending<Value>, sourceLocation: SourceLocation = #_sourceLocation
  ) async throws -> Value {
    try await outcome(pending, sourceLocation: sourceLocation).get()
  }
}
