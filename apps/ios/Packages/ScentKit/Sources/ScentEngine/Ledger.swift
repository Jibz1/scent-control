import Foundation
import ScentBluetooth
import ScentProtocol

// What the engine knows about its diffusers, and how each event changes it. Ported from
// `apps/desktop/src/state.rs`, so the phone and the Mac fold the same messages into the
// same facts. Nothing here touches Bluetooth, the network or the disk. Times are seconds
// since 1970.

/// How many log entries the app keeps. The oldest go first.
let logCap = 500

/// What a diffuser says once it has taken `SAVE_MQTT`. Matched in any case.
private let credentialsSaved = "credentials saved"

/// What the log calls a schedule read from the broker that changed what the app knew.
let sharedScheduleAction = "shared schedule"
let unreadableScheduleAction = "unreadable shared schedule"
let otherVersionScheduleAction = "shared schedule from another version"

/// The most characters a diffuser's name or room may have.
let nameMaxCharacters = 40

/// One value a diffuser reported, with when and over what it arrived.
struct Reported: Equatable, Sendable {
  var value: Int
  var at: Int
  var via: Via
}

/// What a diffuser can report, one reading each. The raw values are the saved file's keys,
/// and the listener's (`fieldKeys`): the HVAC's fan, gas pump and light have no reading here.
enum Field: String, CaseIterable, Sendable {
  case power, intensity, charging, battery, rssi
}

/// The last power and intensity the app told a diffuser to take. Over WiFi the diffuser
/// often says nothing back, so they show as "sent, waiting for the diffuser" until a later
/// report.
struct SentChange: Equatable, Sendable {
  var power: Bool?
  var intensity: Int?
  /// When the newest of these was sent.
  var at: Int
}

/// Everything the engine knows about one diffuser.
struct DeviceRecord: Equatable, Sendable {
  var id: String
  /// What the person calls it; the device ID until they rename it.
  var label: String
  var room: String?
  /// The Bluetooth peripheral's identifier on this phone, once found over Bluetooth.
  var bleID: UUID?
  var info: DeviceInfo?
  var readings: [Field: Reported] = [:]
  var heardWifi: Int?
  var heardBluetooth: Int?
  var pingAt: Int?
  var sent: SentChange?
  /// The schedule the diffuser is believed to hold: the last one sent from Scent Control,
  /// by this phone or, as read from the broker, by the Mac.
  var sentProfile: Profile?
  var sentProfileAt: Int?
  /// When it last said it joined the broker. An add waits for one after the restart.
  var connectedAt: Int?
  /// When its info was last read over Bluetooth.
  var infoAt: Int?
  var credentialsSavedAt: Int?
  var sendOver: SendOver = .auto
  /// The last failure, for its room to say until a newer one or an action that works.
  var failure: String?

  init(id: String) {
    self.id = id
    label = id
  }

  /// The power sent that it hasn't reported on since.
  var pendingPower: Bool? {
    guard let sent, let power = sent.power, !reportedSince(.power, sent.at) else { return nil }
    return power
  }

  /// The intensity sent that it hasn't reported on since.
  var pendingIntensity: Int? {
    guard let sent, let intensity = sent.intensity, !reportedSince(.intensity, sent.at) else {
      return nil
    }
    return intensity
  }

  /// The reading of `field` only while it still counts: one reported before the diffuser's
  /// last reconnect does not, since it may have come back otherwise (owner, 2026-10-22). The
  /// screens show these; the readings themselves are kept.
  func current(_ field: Field) -> Reported? {
    readings[field].flatMap { isCurrent(readingAt: $0.at, connectedAt: connectedAt) ? $0 : nil }
  }

  private func reportedSince(_ field: Field, _ at: Int) -> Bool {
    (readings[field]?.at ?? Int.min) >= at
  }

  /// Takes in what a message reported. Only the fields it carries are touched.
  mutating func merge(_ status: Status, rssi: Int?, at: Int, via: Via) {
    let reported: [(Field, Int?)] = [
      (.power, status.power), (.intensity, status.intensity), (.charging, status.charging),
      (.battery, status.battery), (.rssi, rssi),
    ]
    for case (let field, let value?) in reported {
      readings[field] = Reported(value: value, at: at, via: via)
    }
  }

  /// A `device_connected` at `at`: it joined the broker. It fills in what the message
  /// carries and keeps the rest of what Bluetooth said.
  mutating func noteConnected(_ connected: Connected, at: Int) {
    connectedAt = at
    var info =
      self.info
      ?? DeviceInfo(
        manufacturerID: "", modelName: "", variantName: "", deviceID: id, firmware: "",
        wifiConfigured: true, wifiStatus: 3)
    info.wifiConfigured = true
    if !connected.manufacturer.isEmpty { info.manufacturerID = connected.manufacturer }
    if !connected.model.isEmpty { info.modelName = connected.model }
    if !connected.variant.isEmpty { info.variantName = connected.variant }
    if !connected.fw.isEmpty { info.firmware = connected.fw }
    self.info = info
  }

  /// Whether it has been heard on our broker: a message from it, or a reading that came
  /// over WiFi. This, not whether its WiFi is set up, decides whether WiFi is a way to it:
  /// one with WiFi set up may still be on Scentiment's cloud.
  var onOurBroker: Bool {
    heardWifi != nil || readings.values.contains { $0.via == .wifi }
  }

  /// Whether to ping it: it is on our broker, its info says WiFi is set up, or a schedule
  /// was sent to it (which goes over the broker only).
  var worthPinging: Bool {
    info?.wifiConfigured == true || sentProfile != nil || onOurBroker
  }
}

/// Something that happened, reported by Bluetooth, the broker or the engine.
enum EngineEvent: Sendable {
  case mqtt(topic: String, body: Data, at: Int)
  case bluetoothNotification(bleID: UUID, bytes: Data, at: Int)
  case bluetoothInfo(bleID: UUID, info: DeviceInfo, at: Int)
  case scanned([ScanHit])
  /// A command went out. `summary` is its redacted frame; the rest says what it asked for.
  case sent(
    deviceID: String, summary: String, via: Via, at: Int, power: Bool?, intensity: Int?,
    profile: Profile?)
  case adding(bleID: UUID, step: AddStep)
  case pinged(deviceID: String, at: Int)
  /// Something failed: `what` was being attempted, `error` is why it didn't work.
  case failed(deviceID: String?, what: String, error: String, at: Int)
  case brokerConnecting
  case brokerConnected
  case brokerLoginRefused(String)
  case brokerDisconnected(String)
}

enum BrokerPhase: Equatable, Sendable {
  case connecting
  case connected
  case loginRefused(String)
  case disconnected(String)
}

/// One line of the log. Notes the app makes itself carry no device: broker changes are
/// logged as heard over WiFi with the action `broker`, and failures as sent with no `via`,
/// the action being what was attempted. The text never holds a password.
struct LogEntry: Equatable, Sendable {
  var id: Int
  var at: Int
  var deviceID: String?
  var via: Via?
  var direction: Direction
  var action: String
  var text: String
  /// Whether this app sent it, as against something heard on the broker.
  var sent: Bool
}

/// An add the person started, and where it is. Not saved.
struct Adding: Equatable, Sendable {
  var bleID: UUID
  var label: String
  var room: String?
  /// Its name and room as the add began, so a rename made meanwhile isn't undone.
  var hadLabel: String?
  var hadRoom: String?
  /// The network it was given, if any: never its password.
  var network: String?
  var step: AddStep
  /// The last step the engine worked on, which is where a failure happened.
  var reached: AddStep?
}

/// The engine's knowledge, folded from events.
struct Ledger: Equatable, Sendable {
  var devices: [String: DeviceRecord] = [:]
  var scan: [ScanHit] = []
  var broker: BrokerPhase = .connecting
  /// Oldest first, at most `logCap` entries.
  var log: [LogEntry] = []
  var adding: [UUID: Adding] = [:]
  /// The listener's last heartbeat or will (see `listenerRunning`). Not saved.
  var listener: Heartbeat?
  /// When a heartbeat last said the listener was running. Not saved.
  var listenerSeenAt: Int?
  private var nextLogID = 1

  /// Folds one event in, at `now` for events that carry no time.
  mutating func apply(_ event: EngineEvent, now: Int) {
    switch event {
    case .mqtt(let topic, let body, let at): onMqtt(topic, body, at)
    case .bluetoothNotification(let bleID, let bytes, let at): onNotification(bleID, bytes, at)
    case .bluetoothInfo(let bleID, let info, let at): onInfo(bleID, info, at)
    case .scanned(let hits): onScan(hits)
    case .sent(let deviceID, let summary, let via, let at, let power, let intensity, let profile):
      push(
        LogEntry(
          id: 0, at: at, deviceID: deviceID, via: via, direction: .toDevice,
          action: action(of: summary), text: summary, sent: true))
      guard var device = devices[deviceID] else { return }
      if power != nil || intensity != nil {
        device.sent = SentChange(
          power: power ?? device.pendingPower, intensity: intensity ?? device.pendingIntensity, at: at)
      }
      if let profile {
        device.sentProfile = remember(device.sentProfile, profile)
        device.sentProfileAt = at
      }
      device.failure = nil
      devices[deviceID] = device
    case .adding(let bleID, let step): onAdding(bleID, step, now)
    case .pinged(let deviceID, let at): devices[deviceID]?.pingAt = at
    case .failed(let deviceID, let what, let error, let at):
      push(
        LogEntry(
          id: 0, at: at, deviceID: deviceID, via: nil, direction: .toDevice, action: what, text: error,
          sent: false))
      if let deviceID, let note = LogWords.failureNote(what: what, error: error) {
        devices[deviceID]?.failure = note
      }
    case .brokerConnecting: setBroker(.connecting, now)
    case .brokerConnected: setBroker(.connected, now)
    case .brokerLoginRefused(let why): setBroker(.loginRefused(why), now)
    case .brokerDisconnected(let why): setBroker(.disconnected(why), now)
    }
  }

  /// Starts following an add of the diffuser at `bleID`. One already under way is left.
  mutating func beginAdd(bleID: UUID, label: String, room: String, network: String?) {
    if adding[bleID]?.step.isRunning == true { return }
    let known = devices.values.first { $0.bleID == bleID }
    adding[bleID] = Adding(
      bleID: bleID, label: label, room: room, hadLabel: known?.label, hadRoom: known?.room,
      network: network, step: .readInfo, reached: nil)
  }

  /// Renames a diffuser. An empty name takes its name away: the screens call it by its model.
  mutating func rename(_ deviceID: String, to name: String) {
    let name = cleanName(name)
    devices[deviceID]?.label = name.isEmpty ? deviceID : name
  }

  /// Puts a diffuser in a room, spelled as a room in use when it is one in other capitals.
  /// An empty room takes it out of any.
  mutating func move(_ deviceID: String, to room: String) {
    let room = cleanName(room)
    guard !room.isEmpty else {
      devices[deviceID]?.room = nil
      return
    }
    let inUse = devices.values
      .filter { $0.id != deviceID }
      .compactMap(\.room)
      .first { $0.lowercased() == room.lowercased() }
    devices[deviceID]?.room = inUse ?? room
  }

  /// Whether the diffuser answers pings. Only WiFi counts: the pings go over WiFi.
  func online(_ deviceID: String, now: Int) -> Online {
    guard let device = devices[deviceID] else { return .unknown }
    return onlineStatus(pingAt: device.pingAt, heardAt: device.heardWifi, now: now)
  }

  /// The device a Bluetooth peripheral belongs to.
  func device(bluetooth bleID: UUID) -> DeviceRecord? {
    devices.values.first { $0.bleID == bleID }
  }

  // MARK: Events

  private mutating func onMqtt(_ topic: String, _ body: Data, _ at: Int) {
    if topic.hasPrefix(Topics.appPrefix) {
      onAppMessage(topic, body, at)
      return
    }
    let message = parseMqtt(topic: topic, body: body)
    // The ping round (our PING, then the diffuser's "already added") happens every 30 s
    // for each diffuser. Logged, it would push everything else out of the log.
    let routine =
      message.ack == .alreadyAdded || (message.direction == .toDevice && message.action == "PING")
    let saved = message.action == "acknowledgement" && message.raw.lowercased().contains(credentialsSaved)
    if !routine {
      push(
        LogEntry(
          id: 0, at: at, deviceID: message.deviceID, via: .wifi, direction: message.direction,
          action: message.action ?? "(no action)", text: message.raw, sent: false))
    }
    // Our own commands come back to us too; they say nothing about the diffuser.
    guard message.direction == .fromDevice, let id = message.deviceID, Topics.validDeviceID(id) else {
      return
    }
    var device = devices[id] ?? DeviceRecord(id: id)
    device.heardWifi = at
    if let connected = message.connected { device.noteConnected(connected, at: at) }
    if saved { device.credentialsSavedAt = at }
    device.merge(message.status ?? Status(), rssi: message.rssi, at: at, via: .wifi)
    if message.status != nil { device.failure = nil }
    devices[id] = device
  }

  /// A message under our own prefix: the copy of a diffuser's last schedule that a copy of
  /// the app stored on the broker. The newer of it and what the app knows wins. Logged only
  /// when it changes the schedule the app knew.
  private mutating func onAppMessage(_ topic: String, _ body: Data, _ at: Int) {
    // An empty retained message is how a stored one is taken off the broker.
    guard !body.isEmpty else { return }
    if topic == Topics.listenerTopic {
      onHeartbeat(body, at)
      return
    }
    if topic.hasPrefix(Topics.statePrefix) {
      onSharedState(topic, body, at)
      return
    }
    let text = String(decoding: body, as: UTF8.self)
    let deviceID = Topics.scheduleDevice(topic)
    let decoded = deviceID == nil ? .failure(.invalid) : decodeShared(body, now: at)
    guard let deviceID, case .success(let shared) = decoded else {
      let action =
        if case .failure(.version) = decoded { otherVersionScheduleAction } else { unreadableScheduleAction }
      push(
        LogEntry(
          id: 0, at: at, deviceID: deviceID, via: .wifi, direction: .fromDevice, action: action,
          text: text, sent: false))
      return
    }
    var device = devices[deviceID] ?? DeviceRecord(id: deviceID)
    if let known = device.sentProfileAt, shared.sentAt <= known {
      devices[deviceID] = device
      return
    }
    let changed = device.sentProfile != shared.profile
    device.sentProfile = shared.profile
    device.sentProfileAt = shared.sentAt
    devices[deviceID] = device
    if changed {
      push(
        LogEntry(
          id: 0, at: at, deviceID: deviceID, via: .wifi, direction: .fromDevice,
          action: sharedScheduleAction, text: text, sent: false))
    }
  }

  /// The listener's heartbeat, or its will. One that can't be used changes nothing and is
  /// not logged: a heartbeat comes every 30 s and would fill the log.
  private mutating func onHeartbeat(_ body: Data, _ at: Int) {
    guard case .success(let beat) = decodeHeartbeat(body, now: at) else { return }
    if beat.running { listenerSeenAt = later(listenerSeenAt, beat.at) }
    listener = beat
  }

  /// The listener's record of a diffuser (spec 2026-10-22-listener-design §6.8): each
  /// reading the app knows of is taken when it is newer than the app's own, the heard and
  /// ping times when they are later, and a later reconnect as one heard here. A diffuser not
  /// known yet becomes known. A record that can't be used changes nothing and is not logged:
  /// records come every 30 s.
  private mutating func onSharedState(_ topic: String, _ body: Data, _ at: Int) {
    guard let id = Topics.stateDevice(topic), case .success(let shared) = decodeState(body, now: at) else {
      return
    }
    var device = devices[id] ?? DeviceRecord(id: id)
    for (key, stamped) in shared.fields {
      // The HVAC's fan, gas pump and light have no reading here.
      guard let field = Field(rawValue: key) else { continue }
      if device.readings[field].map({ stamped.at > $0.at }) ?? true {
        device.readings[field] = Reported(value: stamped.value, at: stamped.at, via: .wifi)
      }
    }
    device.heardWifi = later(device.heardWifi, shared.heardAt)
    device.pingAt = later(device.pingAt, shared.pingAt)
    if let connected = shared.connected, device.connectedAt.map({ connected.at > $0 }) ?? true {
      let said = Connected(
        fw: connected.fw, model: connected.model, variant: connected.variant, manufacturer: connected.manufacturer)
      device.noteConnected(said, at: connected.at)
    }
    devices[id] = device
  }

  private mutating func onNotification(_ bleID: UUID, _ bytes: Data, _ at: Int) {
    let note = parseNotification(bytes)
    let device = devices.values.first { $0.bleID == bleID }
    if var device {
      device.heardBluetooth = at
      device.merge(note.status, rssi: nil, at: at, via: .bluetooth)
      if note.raw.lowercased().contains(credentialsSaved) { device.credentialsSavedAt = at }
      if note.kind == .statusText || note.kind == .statusArray { device.failure = nil }
      devices[device.id] = device
    }
    let action =
      switch note.kind {
      case .statusText, .statusArray: "status"
      case .value: "value"
      case .json: "json"
      case .unknown: "unknown"
      }
    push(
      LogEntry(
        id: 0, at: at, deviceID: device?.id, via: .bluetooth, direction: .fromDevice, action: action,
        text: note.raw, sent: false))
  }

  private mutating func onInfo(_ bleID: UUID, _ info: DeviceInfo, _ at: Int) {
    let valid = Topics.validDeviceID(info.deviceID)
    if valid {
      // A peripheral belongs to one diffuser: unlink it from any other first.
      for (id, device) in devices where device.bleID == bleID && id != info.deviceID {
        devices[id]?.bleID = nil
      }
      var device = devices[info.deviceID] ?? DeviceRecord(id: info.deviceID)
      device.bleID = bleID
      device.info = info
      device.infoAt = at
      device.heardBluetooth = at
      devices[info.deviceID] = device
    }
    push(
      LogEntry(
        id: 0, at: at, deviceID: valid ? info.deviceID : nil, via: .bluetooth, direction: .fromDevice,
        action: "info", text: infoText(info), sent: false))
  }

  /// A scan's result replaces the last. A diffuser the app knows is recognized by the device
  /// ID at the end of its advertised name, and its peripheral linked to it: nothing is sent
  /// to it, and from then on Bluetooth control works.
  private mutating func onScan(_ hits: [ScanHit]) {
    scan = hits
    for hit in hits {
      guard let id = deviceIDFromName(hit.name), devices[id] != nil, devices[id]?.bleID != hit.id else {
        continue
      }
      for (other, device) in devices where device.bleID == hit.id { devices[other]?.bleID = nil }
      devices[id]?.bleID = hit.id
    }
  }

  /// The engine says where the add of `bleID` is. Once it is done, the diffuser takes its
  /// name and room, each unless it was changed meanwhile. How each add ended is logged.
  private mutating func onAdding(_ bleID: UUID, _ step: AddStep, _ now: Int) {
    var add =
      adding[bleID]
      ?? Adding(
        bleID: bleID, label: "", room: nil, hadLabel: nil, hadRoom: nil, network: nil, step: .readInfo,
        reached: nil)
    if step.isRunning { add.reached = step }
    add.step = step
    adding[bleID] = add
    let entry: (via: Via?, direction: Direction, text: String)
    switch step {
    case .done(let id):
      if let device = devices[id] {
        let keptLabel = device.label == (add.hadLabel ?? id)
        let keptRoom = device.room == add.hadRoom
        if keptLabel && !add.label.isEmpty { rename(id, to: add.label) }
        if keptRoom, let room = add.room { move(id, to: room) }
      }
      entry = (.wifi, .fromDevice, "Added: it joined your broker.")
    case .failed(let why):
      entry = (nil, .toDevice, why)
    default:
      return
    }
    push(
      LogEntry(
        id: 0, at: now, deviceID: device(bluetooth: bleID)?.id, via: entry.via, direction: entry.direction,
        action: "Add", text: entry.text, sent: false))
  }

  /// Records a change of broker state. A repeat isn't news: the retry loop would otherwise
  /// fill the log with it.
  private mutating func setBroker(_ phase: BrokerPhase, _ now: Int) {
    guard broker != phase else { return }
    let text =
      switch phase {
      case .connecting: "Connecting to the broker."
      case .connected: "Connected to the broker."
      case .loginRefused(let why): "Broker login refused: \(why)"
      case .disconnected(let why): "Disconnected from the broker: \(why)"
      }
    broker = phase
    push(
      LogEntry(
        id: 0, at: now, deviceID: nil, via: .wifi, direction: .fromDevice, action: "broker", text: text,
        sent: false))
  }

  /// Adds to the log, hiding any password in the text, dropping the oldest when full.
  private mutating func push(_ entry: LogEntry) {
    var entry = entry
    entry.id = nextLogID
    nextLogID += 1
    entry.text = maskPasswords(entry.text)
    if log.count >= logCap { log.removeFirst(log.count - logCap + 1) }
    log.append(entry)
  }
}

/// The later of two times, either of which may be unknown.
private func later(_ a: Int?, _ b: Int?) -> Int? {
  [a, b].compactMap { $0 }.max()
}

/// What the diffuser holds after `UPDATE_PROFILE` with `sent`: with flag 0 only the
/// always-on entry went out, so the timed entries stay what they were.
func remember(_ earlier: Profile?, _ sent: Profile) -> Profile {
  guard sent.flag == .alwaysOnOnly else { return sent }
  var held = sent
  held.entries = earlier?.entries ?? []
  return held
}

/// A name or room as typed, tidied: no spaces at either end, one between words, no control
/// characters, at most `nameMaxCharacters`.
func cleanName(_ typed: String) -> String {
  let printable = typed.unicodeScalars.filter {
    CharacterSet.whitespacesAndNewlines.contains($0) || !CharacterSet.controlCharacters.contains($0)
  }
  let words = String(String.UnicodeScalarView(printable)).split(whereSeparator: \.isWhitespace)
    .joined(separator: " ")
  return String(words.prefix(nameMaxCharacters)).trimmingCharacters(in: .whitespaces)
}

/// The `action` of a command's redacted frame (Bluetooth frames end in `-|`).
func action(of summary: String) -> String {
  let frame = summary.hasSuffix("-|") ? String(summary.dropLast(2)) : summary
  guard let data = frame.data(using: .utf8),
    let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
    let action = json["action"] as? String
  else { return "sent" }
  return action
}

/// The info string as the diffuser holds it, one field per `|`.
func infoText(_ info: DeviceInfo) -> String {
  [
    info.manufacturerID, info.modelName, info.variantName, info.deviceID, info.firmware,
    String(info.wifiConfigured), String(info.wifiStatus),
  ].joined(separator: "|")
}
