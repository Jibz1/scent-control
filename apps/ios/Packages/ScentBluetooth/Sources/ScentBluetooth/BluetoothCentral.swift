import Foundation

// The Bluetooth layer: finds diffusers, and keeps one link to each diffuser it is asked
// to talk to. A port of the Mac's Bluetooth worker (`apps/desktop/src/ble.rs`) onto Core
// Bluetooth. It moves bytes and strings; what they mean is the protocol package's job.
//
// Everything runs on the main actor, where Core Bluetooth calls back. Each diffuser does
// one request at a time, in the order they came, while different diffusers go side by
// side. Nothing is written to a diffuser except the frames handed to `send`, and frames
// are never logged or put in an error: they can hold WiFi and broker passwords.

/// Talks to Scentiment diffusers over Bluetooth.
@MainActor public final class BluetoothCentral {
  private enum Request {
    case readInfo
    case readStatus
    case send(Data)
  }

  private let radio: any Radio
  private let timing: any Timing
  private let events: @MainActor (BluetoothEvent) -> Void

  private var links: [UUID: Link] = [:]
  /// Everything scans have heard this launch. A diffuser that has since gone out of range
  /// can still be listed: connecting to it fails with a clear message.
  private var seen: [UUID: Sighting] = [:]
  /// How many scans want Bluetooth scanning now: the person's, and lookups for a link.
  /// Bluetooth stops scanning only when the last one is done.
  private var scanners = 0
  /// Requests waiting for Bluetooth to say whether it can be used.
  private var settling: [Slot<Answer>] = []
  /// The person's scans in progress.
  private var scanning: [Slot<Answer>] = []
  /// Goes up with every `disconnectAll`, so that a request from before it doesn't open a
  /// link after it.
  private var generation = 0
  /// The state last reported through `events`.
  private var reported: BluetoothAvailability = .unknown

  /// `events` hears notifications and changes to Bluetooth's state. Bluetooth itself is
  /// not touched until a request needs it, so iOS asks for permission then, not now.
  public convenience init(events: @escaping @MainActor (BluetoothEvent) -> Void) {
    self.init(radio: CoreBluetoothRadio(), timing: SystemTiming(), events: events)
  }

  init(
    radio: any Radio, timing: any Timing, events: @escaping @MainActor (BluetoothEvent) -> Void
  ) {
    self.radio = radio
    self.timing = timing
    self.events = events
    radio.onEvent = { [weak self] event in self?.heard(event) }
  }

  /// Bluetooth's state. Unknown until the first request has set Bluetooth up.
  public var availability: BluetoothAvailability {
    radio.isStarted ? radio.state : .unknown
  }

  /// Scans for `seconds`, then returns the Scentiment diffusers heard this launch, by name.
  public func scan(seconds: TimeInterval) async throws(BluetoothError) -> [ScanHit] {
    try await ready()
    let slot = Slot<Answer>()
    scanning.append(slot)
    startScanning()
    let end = await slot.wait(until: timing.now + Self.duration(seconds), on: timing)
    scanning.removeAll { $0 === slot }
    stopScanning()
    switch end {
    case .cancelled: throw BluetoothError(message: Wording.cancelled)
    case .value(.stopped(let message)): throw BluetoothError(message: message)
    case .timedOut, .value: return ScanFilter.hits(seen)
    }
  }

  /// Reads the diffuser's info string from `0xFEF4`.
  public func readInfo(_ peripheral: UUID) async throws(BluetoothError) -> String {
    let value = try await perform(peripheral, .readInfo)
    return String(decoding: value ?? Data(), as: UTF8.self)
  }

  /// Writes `frame` to the diffuser, opening a link to it first if there is none. It goes
  /// in parts of 177 bytes, each written with response, and this returns once the last
  /// part is acknowledged. The link then stays open for at least 6 s, and what the
  /// diffuser notifies meanwhile arrives as `.notification` events.
  public func send(_ peripheral: UUID, frame: Data) async throws(BluetoothError) {
    _ = try await perform(peripheral, .send(frame))
  }

  /// Reads `0xFEF3`, which holds the last state the diffuser reported. The value arrives
  /// as a `.notification` event before this returns.
  public func readStatus(_ peripheral: UUID) async throws(BluetoothError) {
    _ = try await perform(peripheral, .readStatus)
  }

  /// Lets go of every diffuser, for when the app goes to the background. Requests in
  /// progress or waiting fail, and scans end. The next request opens a fresh link.
  public func disconnectAll() {
    generation += 1
    let stopped = Answer.stopped(Wording.stopped)
    for slot in settling + scanning { slot.settle(.value(stopped)) }
    for link in links.values {
      link.turns.refuseWaiting(BluetoothError(message: Wording.stopped))
      link.answerAny(stopped)
      let waits = link.closingWaits
      link.closingWaits.removeAll()
      for slot in waits { slot.settle(.value(stopped)) }
      close(link)
    }
  }

  /// Whether Bluetooth has been set up. For tests.
  var isRadioStarted: Bool { radio.isStarted }

  // MARK: Requests

  private func perform(_ id: UUID, _ request: Request) async throws(BluetoothError) -> Data? {
    let generation = self.generation
    let link = link(id)
    guard link.turns.waitingCount < Limits.queue else {
      throw BluetoothError(message: Wording.busy)
    }
    switch await link.turns.take() {
    case .value(.granted): break
    case .value(.refused(let error)): throw error
    case .cancelled, .timedOut: throw BluetoothError(message: Wording.cancelled)
    }
    defer {
      link.turns.release()
      scheduleIdle(link)
    }
    // Skipped if the caller gave up while it waited, or Bluetooth was stopped: something
    // newer may have happened since, and doing this now could undo it.
    try stillWanted(since: generation)
    do {
      try await open(link, generation: generation)
    } catch {
      // As on the Mac, the requests queued behind one that couldn't open the link fail
      // with it, unless it only failed because its caller gave up.
      if error.message != Wording.cancelled { link.turns.refuseWaiting(error) }
      throw error
    }
    // Connecting can take long enough for the caller to give up. Look again before
    // anything is written or read.
    try stillWanted(since: generation)
    do {
      let value = try await exchange(link, request)
      if case .open(var timer) = link.phase {
        if case .send = request { timer.sent(timing.now) } else { timer.touch(timing.now) }
        link.phase = .open(timer)
      }
      return value
    } catch {
      // Where the link stands is unknown; the next request opens a fresh one.
      close(link)
      throw error
    }
  }

  private func stillWanted(since generation: Int) throws(BluetoothError) {
    if Task.isCancelled { throw BluetoothError(message: Wording.cancelled) }
    if generation != self.generation { throw BluetoothError(message: Wording.stopped) }
  }

  private func link(_ id: UUID) -> Link {
    if let link = links[id] { return link }
    let link = Link(id: id)
    links[id] = link
    return link
  }

  /// Sets Bluetooth up if it isn't yet, and gives it time to say whether it can be used:
  /// up to 3 s, or up to a minute while iOS is asking the person for permission.
  private func ready() async throws(BluetoothError) {
    if !radio.isStarted { radio.start() }
    if !isSettled(radio.state) {
      let limit = radio.authorizationPending ? Limits.prompt : Limits.settle
      let slot = Slot<Answer>()
      settling.append(slot)
      let end = await slot.wait(until: timing.now + limit, on: timing)
      settling.removeAll { $0 === slot }
      switch end {
      case .cancelled: throw BluetoothError(message: Wording.cancelled)
      case .value(.stopped(let message)): throw BluetoothError(message: message)
      case .timedOut, .value: break
      }
    }
    if let message = Wording.notReady(radio.state) { throw BluetoothError(message: message) }
  }

  /// The open link, or a new one if there is none or the diffuser dropped it.
  private func open(_ link: Link, generation: Int) async throws(BluetoothError) {
    if link.isOpen {
      if radio.connection(link.id) == .connected { return }
      close(link)
    }
    try await ready()
    try await closed(link)
    try stillWanted(since: generation)
    link.phase = .opening
    // The clock starts once Bluetooth is ready, as it can wait for the person to answer
    // the permission prompt.
    let deadline = timing.now + Limits.connect
    do {
      try await handshake(link, until: deadline)
    } catch {
      // Also cancels a connection that is still being made.
      close(link)
      throw error
    }
    link.phase = .open(LinkTimer(openedAt: timing.now))
  }

  /// Finds the diffuser, connects, finds its characteristics, and turns on notifications.
  private func handshake(_ link: Link, until deadline: ContinuousClock.Instant)
    async throws(BluetoothError)
  {
    let id = link.id
    // Bluetooth hands out a peripheral it remembers, from this launch or an earlier one,
    // with no scan. Failing that, one a scan heard; and last, a scan that runs until the
    // diffuser advertises.
    if !radio.retrieve(id) && !radio.isKnown(id) {
      defer { stopScanning() }
      try await opening(link, .find, "looking for the diffuser", until: deadline) {
        startScanning()
      }
    }
    try await opening(link, .connect, "connecting", until: deadline) { radio.connect(id) }
    try await opening(link, .services, "reading the services", until: deadline) {
      radio.discoverServices(id)
    }
    try await opening(link, .characteristics, "reading the services", until: deadline) {
      radio.discoverCharacteristics(id)
    }
    if let missing = Gatt.firstMissing(from: radio.characteristics(id)) {
      throw Wording.notADiffuser(missing)
    }
    try await opening(link, .subscribe, "turning on notifications", until: deadline) {
      radio.subscribe(id)
    }
  }

  private func opening(
    _ link: Link, _ kind: StepKind, _ activity: String,
    until deadline: ContinuousClock.Instant, _ action: () -> Void
  ) async throws(BluetoothError) {
    let end = await step(link, kind, until: deadline, action)
    if let error = Wording.opening(end, activity: activity) { throw error }
  }

  /// Does the read or the writes of a request on the open link, within 10 s in all.
  private func exchange(_ link: Link, _ request: Request) async throws(BluetoothError) -> Data? {
    let id = link.id
    let deadline = timing.now + Limits.io
    switch request {
    case .readInfo:
      return try await read(link, .info, failure: Wording.readInfoFailed, until: deadline)
    case .readStatus:
      return try await read(link, .status, failure: Wording.readStatusFailed, until: deadline)
    case .send(let frame):
      // One write at a time, each acknowledged, so the parts arrive in order.
      let parts = Gatt.chunks(frame)
      for (index, part) in parts.enumerated() {
        let end = await step(link, .write, until: deadline) { radio.write(id, part) }
        let last = index == parts.count - 1
        if let error = Wording.exchange(end, failure: Wording.writeFailed, mayHaveGoneThrough: last)
        {
          throw error
        }
      }
      return nil
    }
  }

  private func read(
    _ link: Link, _ which: GattCharacteristic, failure: String,
    until deadline: ContinuousClock.Instant
  ) async throws(BluetoothError) -> Data? {
    let end = await step(link, .read(which), until: deadline) { radio.read(link.id, which) }
    if let error = Wording.exchange(end, failure: failure, mayHaveGoneThrough: false) {
      throw error
    }
    guard case .value(.done(let value)) = end else { return nil }
    return value
  }

  /// Asks Bluetooth for something with `action` and waits for the answer, until
  /// `deadline`. The step is in place before `action` runs, so no answer is missed.
  private func step(
    _ link: Link, _ kind: StepKind, until deadline: ContinuousClock.Instant,
    _ action: () -> Void
  ) async -> WaitEnd<Answer> {
    let slot = Slot<Answer>()
    link.step = (kind, slot)
    action()
    let end = await slot.wait(until: deadline, on: timing)
    if link.step?.slot === slot { link.step = nil }
    return end
  }

  // MARK: Closing

  /// Disconnects, and lets the next request open a fresh link once Bluetooth confirms it,
  /// or after 3 s if it never does. Nothing waits here: a close neither fails nor hangs.
  private func close(_ link: Link) {
    link.stopIdleTimer()
    guard link.phase == .opening || link.isOpen else { return }
    // A link the diffuser dropped has nothing left to disconnect.
    guard radio.connection(link.id) != .disconnected else {
      link.phase = .closed
      return
    }
    link.phase = .closing
    radio.cancelConnection(link.id)
    // A connection that was still being made can end at once, with no callback.
    guard radio.connection(link.id) != .disconnected else {
      link.finishClosing()
      return
    }
    link.closingTimer = timing.schedule(after: Limits.disconnect) { [weak link] in
      link?.finishClosing()
    }
  }

  /// Waits for a close in progress to finish.
  private func closed(_ link: Link) async throws(BluetoothError) {
    guard link.phase == .closing else { return }
    let slot = Slot<Answer>()
    link.closingWaits.append(slot)
    switch await slot.wait() {
    case .cancelled, .timedOut: throw BluetoothError(message: Wording.cancelled)
    case .value(.stopped(let message)): throw BluetoothError(message: message)
    case .value: return
    }
  }

  /// Closes the link once it has sat idle until its timer's deadline.
  private func scheduleIdle(_ link: Link) {
    link.stopIdleTimer()
    guard case .open(let timer) = link.phase else { return }
    let delay = max(timer.deadline - timing.now, .zero)
    link.idleTimer = timing.schedule(after: delay) { [weak self, weak link] in
      guard let self, let link else { return }
      self.idleCheck(link)
    }
  }

  private func idleCheck(_ link: Link) {
    link.idleTimer = nil
    // A request in progress looks again when it ends.
    guard case .open(let timer) = link.phase, !link.turns.isBusy else { return }
    if timer.shouldDisconnect(timing.now) {
      close(link)
    } else {
      scheduleIdle(link)
    }
  }

  // MARK: Scanning

  private func startScanning() {
    scanners += 1
    if scanners == 1 { radio.startScan() }
  }

  private func stopScanning() {
    scanners -= 1
    if scanners == 0 { radio.stopScan() }
  }

  // MARK: What Bluetooth says

  private func heard(_ event: RadioEvent) {
    switch event {
    case .stateChanged(let state):
      stateChanged(state)
    case .discovered(let id, let name, let rssi):
      seen[id, default: Sighting()].update(name: name, rssi: rssi)
      links[id]?.answer(.find, .done(nil))
    case .connected(let id):
      if let link = links[id], link.step?.kind == .connect {
        link.answer(.connect, .done(nil))
      } else if links[id]?.isListening != true {
        // A connection nobody waits for, such as one that came through after its attempt
        // timed out. A diffuser takes one connection at a time, so let go of it.
        radio.cancelConnection(id)
      }
    case .failedToConnect(let id, let failure):
      links[id]?.answer(.connect, failure.map(Self.answer) ?? .failed(Wording.noReason))
    case .disconnected(let id, let failure):
      dropped(id, failure)
    case .discoveredServices(let id, let failure):
      links[id]?.answer(.services, Self.result(failure))
    case .discoveredCharacteristics(let id, let failure):
      links[id]?.answer(.characteristics, Self.result(failure))
    case .subscribed(let id, let failure):
      links[id]?.answer(.subscribe, Self.result(failure))
    case .valueUpdated(let id, let which, let value, let failure):
      guard let link = links[id] else { return }
      if which == .status, failure == nil, link.isListening {
        events(.notification(peripheral: id, bytes: value ?? Data(), at: timing.date))
      }
      link.answer(.read(which), Self.result(failure, value))
    case .wrote(let id, let failure):
      links[id]?.answer(.write, Self.result(failure))
    }
  }

  /// The diffuser's link went down.
  private func dropped(_ id: UUID, _ failure: RadioFailure?) {
    guard let link = links[id] else { return }
    switch link.phase {
    case .closing:
      link.finishClosing()
    case .opening:
      // Bluetooth reports an attempt that failed as a failed connection, never as a
      // disconnect, so one heard before connecting is left over from an earlier link.
      if let kind = link.step?.kind, kind == .find || kind == .connect { return }
      link.phase = .closed
      link.answerAny(.dropped(failure?.reason))
    case .open:
      link.stopIdleTimer()
      link.phase = .closed
      link.answerAny(.dropped(failure?.reason))
    case .closed:
      break
    }
  }

  private func stateChanged(_ state: BluetoothAvailability) {
    if state != reported {
      reported = state
      events(.availability(state))
    }
    if isSettled(state) {
      for slot in settling { slot.settle(.value(.done(nil))) }
    }
    guard let message = Wording.notReady(state) else { return }
    // Bluetooth can't be used now. Its connections are gone, with no callback for each.
    for slot in scanning { slot.settle(.value(.stopped(message))) }
    for link in links.values {
      link.stopIdleTimer()
      if link.phase == .closing { link.finishClosing() } else { link.phase = .closed }
      link.answerAny(.stopped(message))
    }
  }

  // MARK: Helpers

  /// Whether Bluetooth has said what it can do. Unknown and resetting are passing states,
  /// and so is unauthorized while iOS hasn't had the person's answer: the Mac sees that
  /// while its permission prompt is open.
  private func isSettled(_ state: BluetoothAvailability) -> Bool {
    switch state {
    case .unknown, .resetting: false
    case .unauthorized: !radio.authorizationPending
    case .unsupported, .poweredOff, .poweredOn: true
    }
  }

  static func duration(_ seconds: TimeInterval) -> Duration {
    seconds.isFinite && seconds > 0 ? .seconds(seconds) : .zero
  }

  private static func answer(_ failure: RadioFailure) -> Answer {
    failure.linkLost ? .dropped(failure.reason) : .failed(failure.reason)
  }

  private static func result(_ failure: RadioFailure?, _ value: Data? = nil) -> Answer {
    failure.map(answer) ?? .done(value)
  }
}
