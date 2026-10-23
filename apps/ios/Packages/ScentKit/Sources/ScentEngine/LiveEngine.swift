import Foundation
import ScentBluetooth
import ScentMQTT
import ScentProtocol

/// The `what` of the failure for a scan that didn't run.
let scanFailedAction = "Bluetooth scan"

/// The engine that talks to real diffusers. Ported from the Mac's hub
/// (`apps/desktop/src/hub`): for each action it picks the way to the diffuser, Bluetooth
/// first when it is nearby and WiFi through the broker when that fails; it keeps a queue
/// for each diffuser, so commands to one run one at a time in the order they were made
/// while different diffusers run side by side; and it pings the diffusers on WiFi, which
/// is how the screens know they answer. What it learns goes into its `Ledger`, which it
/// saves and turns into the screens' model.
@MainActor
public final class LiveEngine: Engine {
  /// How long a scan listens.
  static let scanSeconds: TimeInterval = 10
  /// How long after a schedule the diffuser is told the time. The other order did not run
  /// on the Air 2.
  static let syncTimeAfter: Duration = .seconds(5)
  /// How often the screens' ages and online states are refreshed with no event to prompt it.
  static let refreshEvery: Duration = .seconds(5)
  /// How long a WiFi change shows as "sent, waiting for the diffuser" with no report.
  static let waitingShownFor = 120

  private weak var model: AppModel?
  private var ledger = Ledger()
  private let store: Store
  private let radios: any Radios
  private let passwords: any PasswordStore
  private let clock: () -> Int
  /// Waits within a job: the 25 s after `SAVE_MQTT`, the 5 s before the time sync.
  private let pause: (Duration) async -> Void
  /// Waits between rounds of the background loops: pings, and refreshing the screens.
  private let loopPause: (Duration) async -> Void
  private var settings = BrokerSettings()
  private var appearance: Appearance = .system
  /// Held in memory only, to log in and to hand a diffuser in `SAVE_MQTT`.
  private var password: String?
  private var notice: String?

  /// Each diffuser's queue: the last job's task, which the next one waits for.
  private var queues: [String: Task<Void, Never>] = [:]
  /// When Bluetooth last failed to reach each diffuser. While WiFi can take over, Auto
  /// stays off Bluetooth for a while after that.
  private var bluetoothFailed: [String: Int] = [:]
  /// What is being sent to each diffuser right now, and over what.
  private var inFlight: [String: Pending] = [:]
  /// The diffusers being added, by peripheral: one add at a time for each.
  private var addsUnderWay: Set<UUID> = []
  /// When the wait after `SAVE_MQTT` ends, for each add, so the screens can count it down.
  private var credentialsWait: [UUID: Date] = [:]
  private var active = true
  private var pinger: Task<Void, Never>?
  private var refresher: Task<Void, Never>?
  private var saveScheduled = false

  /// What to start from when nothing is saved yet: what the Mac app knew, copied in by a
  /// local build (`apps/ios/scripts/seed-from-mac.py`).
  private let seed: URL?

  /// The engine for the app, keeping its file in `folder`, starting from `seed` when there
  /// is no file yet.
  public convenience init(folder: URL, seed: URL? = nil) {
    self.init(
      folder: folder, seed: seed, radios: LiveRadios(), passwords: KeychainPasswords(),
      clock: { Int(Date().timeIntervalSince1970) }, pause: { try? await Task.sleep(for: $0) },
      loopPause: { try? await Task.sleep(for: $0) })
  }

  init(
    folder: URL, seed: URL? = nil, radios: any Radios, passwords: any PasswordStore,
    clock: @escaping () -> Int, pause: @escaping (Duration) async -> Void,
    loopPause: @escaping (Duration) async -> Void
  ) {
    store = Store(folder: folder)
    self.seed = seed
    self.radios = radios
    self.passwords = passwords
    self.clock = clock
    self.pause = pause
    self.loopPause = loopPause
  }

  public func attach(_ model: AppModel) {
    self.model = model
    radios.report = { [weak self] event in self?.apply(event) }
    load()
    password = passwords.password(for: settings)
    reconnect()
    startLoops()
    publish()
  }

  public func submit(_ action: Action) {
    switch action {
    case .setPower(let id, let on):
      enqueue(id) { _ = await $0.deliver(id, Work(.turnOn(on)), $0.sendOver(id)) }
    case .setLevel(let id, let level):
      enqueue(id) { _ = await $0.deliver(id, Work(.setLevel(level)), $0.sendOver(id)) }
    case .readStatus(let id):
      enqueue(id) { _ = await $0.deliver(id, .readingStatus, $0.sendOver(id)) }
    case .setSendOver(let id, let sendOver):
      ledger.devices[id]?.sendOver = sendOver
      changed()
    case .scan:
      Task { await scan() }
    case .sendSchedule(let id, let profile):
      enqueue(id) { _ = await $0.sendSchedule(id, profile) }
    case .copySchedule(let from, let targets):
      copySchedule(from: from, to: targets)
    case .rename(let id, let name, let room):
      ledger.rename(id, to: name ?? "")
      ledger.move(id, to: room ?? "")
      changed()
    case .add(let found, let name, let room, let wifiName, let wifiPassword):
      let wifi = wifiName.flatMap { name in wifiPassword.map { (name, Secret($0)) } }
      ledger.beginAdd(bleID: found, label: name ?? "", room: room ?? "", network: wifiName)
      publish()
      Task { await add(found, wifi: wifi) }
    case .saveBroker(let newSettings, let newPassword):
      saveBroker(newSettings, password: newPassword)
    case .setAppearance(let appearance):
      self.appearance = appearance
      changed()
    case .appActive(let isActive):
      setActive(isActive)
    }
  }

  // MARK: Events and the screens

  private func apply(_ event: EngineEvent) {
    ledger.apply(event, now: clock())
    switch event {
    case .bluetoothInfo, .sent, .scanned, .adding, .mqtt, .bluetoothNotification: scheduleSave()
    default: break
    }
    publish()
  }

  /// Something the person changed: save it and show it.
  private func changed() {
    scheduleSave()
    publish()
  }

  /// Turns the ledger into what the screens draw.
  private func publish() {
    guard let model else { return }
    let now = clock()
    model.diffusers = ledger.devices.values.sorted { $0.id < $1.id }.map { state(of: $0, now: now) }
    model.found = found()
    model.broker = brokerStatus()
    model.listener = listenerStatus(now: now)
    model.brokerSettings = settings
    model.appearance = appearance
    model.notice = notice
    model.log = ledger.log.reversed().map { line(for: $0) }
    var adds: [UUID: AddProgress] = [:]
    for (bleID, add) in ledger.adding { adds[bleID] = progress(of: add) }
    model.adds = adds
  }

  /// What the screens show of `device`. Only readings that still count are shown: one from
  /// before its last reconnect is kept, not shown.
  private func state(of device: DeviceRecord, now: Int) -> DiffuserState {
    let power = device.current(.power).map { Reading($0.value != 0, at: date($0.at), via: $0.via) }
    let level = device.current(.intensity).flatMap { r in Level(r.value).map { Reading($0, at: date(r.at), via: r.via) } }
    let charging = device.current(.charging)
    let battery = device.current(.battery)
    var pluggedIn: Reading<Bool>?
    if let source = [charging, battery].compactMap({ $0 }).max(by: { $0.at < $1.at }) {
      let plugged = charging?.value == 2 || charging?.value == 1 || battery?.value == 200
      pluggedIn = Reading(plugged, at: date(source.at), via: source.via)
    }
    let batteryLevel = battery.flatMap { $0.value <= 100 ? Reading($0.value, at: date($0.at), via: $0.via) : nil }
    let signal = device.current(.rssi).map { Reading($0.value, at: date($0.at), via: $0.via) }
    let heard = [device.heardWifi, device.heardBluetooth].compactMap { $0 }.max()
    // The newest report, current or not: one from before a reconnect says it is not known.
    let newest = [device.readings[.power], device.readings[.intensity]].compactMap { $0 }.max { $0.at < $1.at }
    var notKnownSince: Date?
    if let newest, let connectedAt = device.connectedAt, !isCurrent(readingAt: newest.at, connectedAt: connectedAt) {
      notKnownSince = date(connectedAt)
    }

    var pending = inFlight[device.id]
    if pending == nil, let sent = device.sent, now - sent.at < Self.waitingShownFor,
      device.pendingPower != nil || device.pendingIntensity != nil
    {
      pending = Pending(
        power: device.pendingPower, level: device.pendingIntensity.flatMap(Level.init),
        stage: .waitingForDiffuser)
    }
    let kind =
      device.info.map { DiffuserKind(model: $0.modelName) }
      ?? ledger.scan.first { $0.id == device.bleID }.map { DiffuserKind(scanName: $0.name) } ?? .other
    return DiffuserState(
      id: device.id, name: device.label == device.id ? nil : device.label, room: device.room, kind: kind,
      variant: device.info.flatMap { $0.variantName.isEmpty ? nil : $0.variantName },
      firmware: device.info.flatMap { $0.firmware.isEmpty ? nil : $0.firmware },
      reachability: reachability(of: device, now: now), power: power, level: level, pluggedIn: pluggedIn,
      battery: batteryLevel, signal: signal, lastReport: heard.map(date), notKnownSince: notKnownSince,
      pending: pending,
      failure: device.failure, schedule: device.sentProfile, scheduleSentAt: device.sentProfileAt.map(date),
      sendOver: device.sendOver, bluetoothID: device.bleID, onOurBroker: device.onOurBroker)
  }

  /// Answering when heard recently over either transport; otherwise what the pings say.
  private func reachability(of device: DeviceRecord, now: Int) -> Reachability {
    let wifi = ledger.online(device.id, now: now)
    let bluetoothRecently = device.heardBluetooth.map { now - $0 <= onlineForSeconds } ?? false
    if bluetoothRecently, (device.heardBluetooth ?? 0) >= (device.heardWifi ?? 0) || wifi != .online {
      return .answering(.bluetooth)
    }
    switch wifi {
    case .online: return .answering(.wifi)
    case .waiting: return .waiting
    case .offline: return .notAnswering
    case .unknown: return .unknown
    }
  }

  /// What the last scan found that the app doesn't know yet.
  private func found() -> [FoundDiffuser] {
    let linked = Set(ledger.devices.values.compactMap(\.bleID))
    return ledger.scan
      .filter { hit in
        !linked.contains(hit.id) && deviceIDFromName(hit.name).map { ledger.devices[$0] == nil } ?? true
      }
      .map { FoundDiffuser(id: $0.id, kind: DiffuserKind(scanName: $0.name), variant: parseName($0.name).variant, rssi: $0.rssi) }
  }

  private func listenerStatus(now: Int) -> ListenerStatus {
    if listenerRunning(ledger.listener, now: now) { return .running }
    if ledger.listener == nil && ledger.listenerSeenAt == nil { return .notSeen }
    return .notRunning(lastSeen: ledger.listenerSeenAt.map(date))
  }

  private func brokerStatus() -> BrokerStatus {
    guard settings.isSet, password != nil else { return .notSetUp }
    switch ledger.broker {
    case .connecting: return .connecting
    case .connected: return .connected
    case .loginRefused: return .loginRefused
    case .disconnected(let why): return .unreachable(why)
    }
  }

  private func line(for entry: LogEntry) -> LogLine {
    let label: String
    if let id = entry.deviceID, let device = ledger.devices[id] {
      label = device.label == id ? (device.info.map { DiffuserKind(model: $0.modelName).modelName } ?? id) : device.label
    } else {
      label = entry.action == "broker" ? "Broker" : "the diffuser"
    }
    let mark: LogLine.Mark =
      entry.direction == .toDevice && entry.via == nil ? .failure : entry.direction == .toDevice ? .sent : .heard
    return LogLine(
      id: entry.id, at: Date(timeIntervalSince1970: TimeInterval(entry.at)), mark: mark,
      sentence: LogWords.sentence(entry, label: label), detail: entry.text.isEmpty ? nil : entry.text)
  }

  private func progress(of add: Adding) -> AddProgress {
    let hit = ledger.scan.first { $0.id == add.bleID }
    let device = ledger.device(bluetooth: add.bleID)
    var order: [AddProgress.Step] = [.read, .moveToBroker]
    if add.network != nil { order.append(.joinWifi) }
    order += [.restart, .waitForConnect]
    let current: AddProgress.Step? =
      switch add.reached ?? add.step {
      case .readInfo: .read
      case .saveMqtt: .moveToBroker
      case .wifiConnect: .joinWifi
      case .restart: .restart
      case .waitConnected, .ping: .waitForConnect
      default: nil
      }
    let reachedIndex = current.flatMap { order.firstIndex(of: $0) } ?? 0
    let steps = order.enumerated().map { index, step -> (step: AddProgress.Step, state: AddProgress.StepState) in
      switch add.step {
      case .done: return (step, .done)
      case .needWifi: return (step, step == .read ? .done : .waiting)
      case .failed(let why):
        return (step, index < reachedIndex ? .done : index == reachedIndex ? .failed(why) : .waiting)
      default:
        if index < reachedIndex { return (step, .done) }
        if index > reachedIndex { return (step, .waiting) }
        if step == .moveToBroker, let until = credentialsWait[add.bleID] {
          return (step, .countdown(until: until, total: TimeInterval(credentialsWaitSeconds)))
        }
        return (step, .working)
      }
    }
    var progress = AddProgress(
      deviceID: device?.id, kind: hit.map { DiffuserKind(scanName: $0.name) } ?? .other,
      variant: hit.flatMap { parseName($0.name).variant }, name: add.label.isEmpty ? nil : add.label,
      room: add.room?.isEmpty == false ? add.room : nil, steps: steps, needsWifi: add.step == .needWifi)
    if case .done = add.step { progress.finished = true }
    return progress
  }

  // MARK: Saving

  private func load() {
    do {
      guard let stored = try store.load() ?? seeded() else { return }
      settings = BrokerSettings(url: stored.broker.url, username: stored.broker.username, routeID: stored.broker.routeID)
      appearance = Appearance(rawValue: stored.ui?.appearance ?? "") ?? .system
      for saved in stored.devices where Topics.validDeviceID(saved.id) {
        var device = DeviceRecord(id: saved.id)
        device.label = saved.label.isEmpty ? saved.id : saved.label
        device.room = saved.room
        device.bleID = saved.bleID.flatMap(UUID.init(uuidString:))
        device.info = saved.info.map {
          DeviceInfo(
            manufacturerID: $0.manufacturerID, modelName: $0.modelName, variantName: $0.variantName,
            deviceID: $0.deviceID, firmware: $0.firmware, wifiConfigured: $0.wifiConfigured,
            wifiStatus: $0.wifiStatus)
        }
        for (key, reading) in saved.readings {
          guard let field = Field(rawValue: key) else { continue }
          device.readings[field] = Reported(
            value: reading.value, at: reading.at, via: reading.via == "bluetooth" ? .bluetooth : .wifi)
        }
        device.sentProfile = saved.sentProfile?.profile
        device.sentProfileAt = device.sentProfile == nil ? nil : saved.sentProfileAt
        device.sendOver = saved.sendOver.flatMap(SendOver.init(rawValue:)) ?? .auto
        ledger.devices[saved.id] = device
      }
    } catch {
      notice = error.message
    }
  }

  /// The seed, read as a saved file, or nil.
  private func seeded() -> StoredFile? {
    guard let seed, let data = try? Data(contentsOf: seed) else { return nil }
    return try? JSONDecoder().decode(StoredFile.self, from: data)
  }

  private func scheduleSave() {
    guard !saveScheduled else { return }
    saveScheduled = true
    Task {
      await pause(.milliseconds(500))
      saveScheduled = false
      save()
    }
  }

  private func save() {
    let devices = ledger.devices.values.sorted { $0.id < $1.id }.map { device in
      StoredDevice(
        id: device.id, label: device.label, room: device.room, bleID: device.bleID?.uuidString,
        info: device.info.map {
          StoredDevice.StoredInfo(
            manufacturerID: $0.manufacturerID, modelName: $0.modelName, variantName: $0.variantName,
            deviceID: $0.deviceID, firmware: $0.firmware, wifiConfigured: $0.wifiConfigured,
            wifiStatus: $0.wifiStatus)
        },
        readings: Dictionary(
          uniqueKeysWithValues: device.readings.map {
            ($0.key.rawValue, StoredDevice.StoredReading(value: $0.value.value, at: $0.value.at, via: $0.value.via == .bluetooth ? "bluetooth" : "wifi"))
          }),
        sentProfile: device.sentProfile.map(StoredProfile.init), sentProfileAt: device.sentProfileAt,
        sendOver: device.sendOver == .auto ? nil : device.sendOver.rawValue)
    }
    let file = StoredFile(
      devices: devices,
      broker: .init(url: settings.url, username: settings.username, routeID: settings.routeID),
      ui: appearance == .system ? nil : .init(appearance: appearance.rawValue))
    do {
      try store.save(file)
    } catch {
      notice = error.message
      publish()
    }
  }

  // MARK: The broker

  private func saveBroker(_ newSettings: BrokerSettings, password newPassword: String?) {
    settings = BrokerSettings(
      url: newSettings.url.trimmingCharacters(in: .whitespaces),
      username: newSettings.username.trimmingCharacters(in: .whitespaces),
      routeID: newSettings.routeID.trimmingCharacters(in: .whitespaces))
    if let newPassword, !newPassword.isEmpty {
      do {
        try passwords.save(newPassword, for: settings)
        password = newPassword
        notice = nil
      } catch {
        password = newPassword
        notice = error.message
      }
    } else {
      password = passwords.password(for: settings)
    }
    scheduleSave()
    reconnect()
    publish()
  }

  /// Drops the broker connection and makes a new one, unless there is nothing to log in
  /// with. Also how a refused login is tried again.
  private func reconnect() {
    let password = settings.isSet ? password : nil
    radios.reconnect(settings, password: password)
    if !settings.isSet {
      apply(.brokerDisconnected("No broker set. Add it in Settings."))
    } else if password == nil {
      apply(.brokerDisconnected("No broker password saved. Add it in Settings."))
    }
  }

  // MARK: Lifecycle and pings

  private func setActive(_ isActive: Bool) {
    guard isActive != active else { return }
    active = isActive
    if isActive {
      // Coming back, the connection may have died while the app was suspended. A refused
      // login waits for new settings instead.
      if !radios.brokerUp && !loginRefused { reconnect() }
      startLoops()
    } else {
      pinger?.cancel()
      refresher?.cancel()
      pinger = nil
      refresher = nil
      radios.idle()
    }
  }

  private var loginRefused: Bool {
    if case .loginRefused = ledger.broker { true } else { false }
  }

  private func startLoops() {
    pinger?.cancel()
    pinger = Task { [weak self] in
      while !Task.isCancelled {
        // Not held across the wait, so the engine can go while the loop sleeps.
        guard let wait = self?.pingRound(), let pause = self?.loopPause else { return }
        await pause(wait ? .seconds(pingEverySeconds) : .seconds(1))
      }
    }
    refresher?.cancel()
    refresher = Task { [weak self] in
      // The engine is looked up afresh each round, and the loop ends once it is gone:
      // `self?.` alone would skip the wait and spin.
      while !Task.isCancelled {
        guard let pause = self?.loopPause else { return }
        await pause(Self.refreshEvery)
        self?.publish()
      }
    }
  }

  /// Pings every diffuser worth pinging. False, having sent nothing, while the broker
  /// isn't connected.
  private func pingRound() -> Bool {
    guard radios.brokerUp, !settings.routeID.isEmpty else { return false }
    let targets = ledger.devices.values.filter { Topics.validDeviceID($0.id) && $0.worthPinging }.map(\.id)
    for id in targets {
      let routeID = settings.routeID
      Task {
        do throws(MQTTError) {
          try await radios.wifiSend(id, .ping(userid: routeID))
          apply(.pinged(deviceID: id, at: clock()))
        } catch {
          apply(.failed(deviceID: id, what: "WiFi PING", error: error.message, at: clock()))
        }
      }
    }
    return true
  }

  // MARK: Queues and delivery

  private func sendOver(_ id: String) -> SendOver {
    ledger.devices[id]?.sendOver ?? .auto
  }

  /// Runs `job` after the jobs already queued for this diffuser.
  private func enqueue(_ deviceID: String, _ job: @escaping @MainActor (LiveEngine) async -> Void) {
    guard Topics.validDeviceID(deviceID) else {
      apply(.failed(deviceID: nil, what: "Command", error: "\(deviceID) is not a valid device ID.", at: clock()))
      return
    }
    let before = queues[deviceID]
    queues[deviceID] = Task { [weak self] in
      await before?.value
      guard let self else { return }
      await job(self)
    }
  }

  /// What one job asks of a diffuser.
  struct Work {
    let command: Command
    /// Bluetooth reads a status from the diffuser instead of writing to it.
    let readsStatus: Bool

    init(_ command: Command) {
      self.command = command
      readsStatus = false
    }

    private init(command: Command, readsStatus: Bool) {
      self.command = command
      self.readsStatus = readsStatus
    }

    /// Asks for its state: Bluetooth reads what it last reported, WiFi sends `GET_STATUS`.
    static let readingStatus = Work(command: .getStatus, readsStatus: true)

    /// What the log calls it: the action's name only, never the frame.
    func name(_ via: Via) -> String {
      readsStatus ? "read status" : command.action(via: via)
    }
  }

  /// Takes `work` to the diffuser the way `sendOver` says, trying the next transport when
  /// one fails. Logs what went out and one failure for whatever failed on the way. The
  /// time it went out, or nil.
  @discardableResult
  private func deliver(_ id: String, _ work: Work, _ sendOver: SendOver) async -> Int? {
    let device = ledger.devices[id]
    let bleID = device?.bleID
    let wifi = wifiAvailable(
      sendOver, brokerUp: radios.brokerUp, onOurBroker: device?.onOurBroker ?? false, hasBluetooth: bleID != nil)
    let bluetooth = bluetoothWorthIt(
      sendOver, hasBluetooth: bleID != nil, failedAt: bluetoothFailed[id], heardBluetooth: device?.heardBluetooth,
      wifi: wifi, now: clock())
    let ways = route(sendOver, bluetooth: bluetooth, wifi: wifi)
    guard !ways.isEmpty else {
      apply(.failed(deviceID: id, what: work.name(sendOver.firstChoice), error: whyNoRoute(sendOver), at: clock()))
      return nil
    }
    let (power, intensity, _) = effects(work.command)
    var failed: [(Via, String)] = []
    for via in ways {
      if !work.readsStatus {
        inFlight[id] = Pending(power: power, level: intensity.flatMap(Level.init), stage: .sending(via))
        publish()
      }
      let problem: String?
      switch via {
      case .bluetooth: problem = await overBluetooth(id, bleID, work)
      case .wifi:
        do {
          try await radios.wifiSend(id, work.command)
          problem = nil
        } catch {
          problem = error.message
        }
      }
      if let problem {
        failed.append((via, problem))
        continue
      }
      inFlight[id] = nil
      reportFailure(id, work, failed, deliveredBy: via)
      return sent(id, work, via)
    }
    inFlight[id] = nil
    reportFailure(id, work, failed, deliveredBy: nil)
    return nil
  }

  /// One Bluetooth attempt, which also keeps the cool-down: a failure starts it, a success
  /// ends it. The error, or nil when it got there.
  private func overBluetooth(_ id: String, _ bleID: UUID?, _ work: Work) async -> String? {
    guard let bleID else { return "This diffuser hasn't been found over Bluetooth on this iPhone yet." }
    do {
      if work.readsStatus {
        try await radios.readStatus(bleID)
      } else {
        try await radios.bluetoothSend(bleID, work.command)
      }
      bluetoothFailed[id] = nil
      return nil
    } catch {
      bluetoothFailed[id] = clock()
      return error.message
    }
  }

  /// The command went out over `via`: log it, and what it asked for. Returns its time.
  @discardableResult
  private func sent(_ id: String, _ work: Work, _ via: Via) -> Int {
    let at = clock()
    // Reading a status over Bluetooth writes nothing, so there is nothing to log.
    if via == .bluetooth && work.readsStatus { return at }
    let (power, intensity, profile) = effects(work.command)
    apply(
      .sent(
        deviceID: id, summary: work.command.redacted(via: via), via: via, at: at, power: power,
        intensity: intensity, profile: profile))
    return at
  }

  private func reportFailure(_ id: String, _ work: Work, _ failed: [(Via, String)], deliveredBy: Via?) {
    guard let (what, error) = failureLine(work.name, failed, deliveredBy) else { return }
    apply(.failed(deviceID: id, what: what, error: error, at: clock()))
  }

  // MARK: Schedules

  /// Sends the schedule, stores its copy on the broker for the app's other copies, waits,
  /// then tells the diffuser the time. All over WiFi. The copy is stored only once the
  /// broker has taken the table; a copy it didn't take is tried once more after the time
  /// sync, unless a newer one was adopted meanwhile. The error, or nil when it all went:
  /// a copy that still wasn't stored is an error too, so a copy to another diffuser says so
  /// instead of looking done.
  @discardableResult
  private func sendSchedule(_ id: String, _ profile: Profile) async -> String? {
    let held = remember(ledger.devices[id]?.sentProfile, profile)
    guard let at = await deliver(id, Work(.updateProfile(profile)), .wifi) else {
      return ledger.devices[id]?.failure ?? "The schedule couldn't be sent."
    }
    var shared = await share(id, held, at)
    await pause(Self.syncTimeAfter)
    let synced = await deliver(id, Work(.syncTime(serverTime: UInt64(max(clock(), 0)))), .wifi) != nil
    var notStored: String?
    if shared != nil, !newerScheduleKnown(id, than: at) {
      shared = await share(id, held, at)
      if let shared {
        let message =
          "The schedule went out, but the copy that other copies of Scent Control read wasn't stored on the broker. \(shared) Send the schedule again to store it."
        apply(.failed(deviceID: id, what: "Schedule", error: message, at: clock()))
        notStored = message
      }
    }
    if !synced {
      let message =
        "The schedule went out, but the time sync after it didn't, so the diffuser may not run it yet. Send the schedule again."
      apply(.failed(deviceID: id, what: "Schedule", error: message, at: clock()))
      return message
    }
    return notStored
  }

  private func newerScheduleKnown(_ id: String, than at: Int) -> Bool {
    (ledger.devices[id]?.sentProfileAt ?? Int.min) > at
  }

  /// Stores `held`, sent at `at`, as the retained copy the app's other copies read. The
  /// error, or nil.
  private func share(_ id: String, _ held: Profile, _ at: Int) async -> String? {
    guard let body = encodeShared(held, sentAt: at) else { return "Its time zone isn't a number." }
    do {
      try await radios.storeSchedule(id, body)
      return nil
    } catch {
      return error.message
    }
  }

  /// Sends one diffuser's last schedule to others, each as its own Send Schedule in its own
  /// queue: the whole table is replaced, then it is told the time.
  private func copySchedule(from source: String, to targets: [String]) {
    guard var profile = ledger.devices[source]?.sentProfile else { return }
    profile.flag = .replace
    for target in targets where target != source {
      model?.copies[target] = .sending
      enqueue(target) { engine in
        let error = await engine.sendSchedule(target, profile)
        engine.model?.copies[target] = error.map { .failed($0) } ?? .copied
      }
    }
  }

  // MARK: Scanning

  private func scan() async {
    model?.scanning = true
    model?.scanProblem = nil
    do {
      let hits = try await radios.scan(seconds: Self.scanSeconds)
      apply(.scanned(hits))
    } catch {
      model?.scanProblem = error.message
      apply(.failed(deviceID: nil, what: scanFailedAction, error: error.message, at: clock()))
    }
    model?.scanning = false
  }

  // MARK: Adding

  private func adding(_ bleID: UUID, _ step: AddStep) {
    apply(.adding(bleID: bleID, step: step))
  }

  /// The first part of an add: no other add of it may be under way, and there must be a
  /// broker login to hand it. Then its info is read over Bluetooth, which gives its device
  /// ID, and the rest runs as one job in its queue.
  private func add(_ bleID: UUID, wifi: (String, Secret)?) async {
    guard !addsUnderWay.contains(bleID) else {
      apply(
        .failed(
          deviceID: ledger.device(bluetooth: bleID)?.id, what: "Add",
          error: "This diffuser is already being added. Wait for that to finish, then try again.", at: clock()))
      return
    }
    let stop: String? =
      if !settings.isSet { "Add the broker in Settings first." }
      else if case .loginRefused = ledger.broker {
        "The broker refused this app's login, so the diffuser would be refused too. Fix it in Settings first."
      } else if password == nil { noBrokerPasswordToGive } else { nil }
    if let stop {
      adding(bleID, .failed(stop))
      return
    }
    addsUnderWay.insert(bleID)
    adding(bleID, .readInfo)
    let info: DeviceInfo
    do {
      let text = try await radios.readInfo(bleID)
      info = try parseInfo(text)
    } catch let error as BluetoothError {
      addsUnderWay.remove(bleID)
      adding(bleID, .failed(error.message))
      return
    } catch {
      addsUnderWay.remove(bleID)
      adding(bleID, .failed("The diffuser answered, but its info couldn't be read, so its device ID isn't known. Try again."))
      return
    }
    apply(.bluetoothInfo(bleID: bleID, info: info, at: clock()))
    let next = nextAddStep(.readInfo, info: info, wifiGiven: wifi != nil, connectedDeviceID: nil)
    guard next == .saveMqtt, Topics.validDeviceID(info.deviceID) else {
      addsUnderWay.remove(bleID)
      adding(bleID, next == .saveMqtt ? .failed("Its device ID isn't one Scent Control can use.") : next)
      return
    }
    enqueue(info.deviceID) { engine in
      await engine.runAdd(info.deviceID, bleID, info, wifi)
      engine.addsUnderWay.remove(bleID)
      engine.credentialsWait[bleID] = nil
    }
  }

  /// How a write to a diffuser being added ended.
  private enum Written {
    case yes
    /// No answer in time: it may still have gone through.
    case unknown
    case no(String)

    var isRefused: Bool {
      if case .no = self { true } else { false }
    }
  }

  /// The rest of an add, as one job in the diffuser's queue: the writes over Bluetooth
  /// only, then the wait for it on our broker, and the ping over WiFi. A write whose result
  /// is unknown is never sent again; the wait tells whether it worked.
  private func runAdd(_ id: String, _ bleID: UUID, _ info: DeviceInfo, _ wifi: (String, Secret)?) async {
    var restarted = clock()
    var step = AddStep.saveMqtt
    while true {
      adding(bleID, step)
      var connected: String?
      let outcome: Written
      switch step {
      case .saveMqtt:
        if let password {
          let save = Command.saveMqtt(broker: settings.url, username: settings.username, password: Secret(password))
          outcome = await addWrite(id, bleID, save)
        } else {
          outcome = .no(noBrokerPasswordToGive)
        }
        if !outcome.isRefused {
          // Always the whole wait, whatever the diffuser says meanwhile.
          credentialsWait[bleID] = Date().addingTimeInterval(TimeInterval(credentialsWaitSeconds))
          publish()
          await pause(.seconds(credentialsWaitSeconds))
          credentialsWait[bleID] = nil
        }
      case .wifiConnect:
        if let wifi {
          outcome = await addWrite(id, bleID, .wifiConnect(ssid: wifi.0, password: wifi.1))
        } else {
          outcome = .no("The add stopped at a step it doesn't run. Try again.")
        }
      case .restart:
        restarted = clock()
        // The reboot can cut the write short; whether it went through is what the wait
        // finds out.
        _ = await addWrite(id, bleID, .softRestart)
        outcome = .yes
      case .waitConnected:
        switch await waitConnected(id, since: restarted) {
        case .ping:
          connected = id
          outcome = .yes
        case .failed(let why): outcome = .no(why)
        default: outcome = .no("The add stopped at a step it doesn't run. Try again.")
        }
      case .ping:
        // Joining proved the move. If the broker won't take the ping now, the ping round
        // registers it later.
        if await deliver(id, Work(.ping(userid: settings.routeID)), .wifi) != nil {
          apply(.pinged(deviceID: id, at: clock()))
        }
        outcome = .yes
      case .done, .failed:
        return
      case .readInfo, .needWifi:
        outcome = .no("The add stopped at a step it doesn't run. Try again.")
      }
      if case .no(let why) = outcome {
        adding(bleID, .failed(why))
        return
      }
      step = nextAddStep(step, info: info, wifiGiven: wifi != nil, connectedDeviceID: connected)
      if case .done = step {
        adding(bleID, step)
        return
      }
    }
  }

  /// Writes `command` to the diffuser being added, over Bluetooth only, and logs it.
  private func addWrite(_ id: String, _ bleID: UUID, _ command: Command) async -> Written {
    let work = Work(command)
    do {
      try await radios.bluetoothSend(bleID, command)
      bluetoothFailed[id] = nil
      sent(id, work, .bluetooth)
      return .yes
    } catch {
      reportFailure(id, work, [(.bluetooth, error.message)], deliveredBy: nil)
      return error.unanswered ? .unknown : .no(error.message)
    }
  }

  /// Waits for the diffuser to say it joined the broker after the restart, for up to
  /// `waitConnectedSeconds`.
  private func waitConnected(_ id: String, since: Int) async -> AddStep {
    let started = clock()
    while true {
      let connected = (ledger.devices[id]?.connectedAt ?? Int.min) > since
      let outcome = waitOutcome(waited: clock() - started, connected: connected)
      guard outcome == .waitConnected else { return outcome }
      await pause(.milliseconds(250))
    }
  }
}

/// What `command` asks the diffuser to be, for the ledger to remember until it reports.
private func effects(_ command: Command) -> (power: Bool?, intensity: Int?, profile: Profile?) {
  switch command {
  case .turnOn(let on): (on, nil, nil)
  case .setLevel(let level): (nil, level.value, nil)
  case .updateProfile(let profile): (nil, nil, profile)
  default: (nil, nil, nil)
  }
}

/// The one failure line (`what`, `error`) for a job, or nil when nothing failed. `failed`
/// is what went wrong in the order it was tried; `deliveredBy` the transport that worked
/// afterwards, if any. Names the action only, never the frame.
func failureLine(
  _ name: (Via) -> String, _ failed: [(Via, String)], _ deliveredBy: Via?
) -> (String, String)? {
  guard let (first, firstError) = failed.first else { return nil }
  if let deliveredBy {
    return (
      "\(first.name) \(name(first))",
      "\(first.name) couldn't reach the diffuser, so this went over \(deliveredBy.name). \(firstError)"
    )
  }
  if failed.count == 1 { return ("\(first.name) \(name(first))", firstError) }
  let told = failed.map { "\($0.0.name): \($0.1)" }.joined(separator: " ")
  return (name(first), "Neither route worked. \(told)")
}

extension BrokerSettings {
  /// Whether there is a broker to log in to.
  var isSet: Bool {
    URL(string: url)?.host?.isEmpty == false && !username.isEmpty
  }
}

/// A time in seconds since 1970, as a date.
private func date(_ at: Int) -> Date {
  Date(timeIntervalSince1970: TimeInterval(at))
}
