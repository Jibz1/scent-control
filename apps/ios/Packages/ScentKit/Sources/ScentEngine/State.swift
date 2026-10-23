import Foundation
import ScentProtocol

/// The diffuser families Scent Control knows, from the model letter in the advertised name
/// or the `0xFEF4` info, and any other, which is drawn as an Air 2.
public enum DiffuserKind: Hashable, Sendable {
  case air2
  case hvac2
  case other

  /// From what a diffuser says about itself: "Diffuser" (or "Diffuser Air 2") is the Air 2,
  /// "Hvac" the HVAC 2.
  public init(model: String) {
    let lower = model.lowercased()
    if lower.contains("hvac") {
      self = .hvac2
    } else if lower.contains("air") || lower == "diffuser" {
      self = .air2
    } else {
      self = .other
    }
  }

  /// From an advertised name, which says it by a letter.
  public init(scanName: String) {
    switch parseName(scanName).modelCode {
    case "D": self = .air2
    case "H": self = .hvac2
    default: self = .other
    }
  }

  public var modelName: String {
    switch self {
    case .air2: "Air 2"
    case .hvac2: "HVAC 2"
    case .other: "Diffuser"
    }
  }
}

extension Via {
  public var name: String {
    switch self {
    case .bluetooth: "Bluetooth"
    case .wifi: "WiFi"
    }
  }
}

/// The Mac's Send Over choice: Bluetooth first when in range, or one transport only.
public enum SendOver: String, CaseIterable, Hashable, Sendable {
  case auto
  case wifi
  case bluetooth
}

/// Whether a diffuser answers its pings (`scent_core::Online`).
public enum Reachability: Hashable, Sendable {
  /// Heard from recently, last over this transport.
  case answering(Via)
  /// Pinged, and the answer isn't due yet.
  case waiting
  /// Pinged, and no answer came.
  case notAnswering
  /// No recent evidence either way.
  case unknown
}

/// A value a diffuser reported, with when and over what.
public struct Reading<Value: Hashable & Sendable>: Hashable, Sendable {
  public var value: Value
  public var at: Date
  public var via: Via

  public init(_ value: Value, at: Date, via: Via) {
    self.value = value
    self.at = at
    self.via = via
  }
}

/// A change on its way to a diffuser.
public struct Pending: Hashable, Sendable {
  public enum Stage: Hashable, Sendable {
    /// Being sent; `nil` while the route is still being chosen.
    case sending(Via?)
    /// Sent over WiFi, and no report has confirmed it yet.
    case waitingForDiffuser
  }

  /// The power it was asked for, if that is what changed.
  public var power: Bool?
  /// The intensity it was asked for, if that is what changed.
  public var level: Level?
  public var stage: Stage

  public init(power: Bool? = nil, level: Level? = nil, stage: Stage) {
    self.power = power
    self.level = level
    self.stage = stage
  }
}

/// Everything the screens show about one known diffuser.
public struct DiffuserState: Identifiable, Hashable, Sendable {
  /// The device ID: 12 upper-case letters or digits.
  public var id: String
  public var name: String?
  public var room: String?
  public var kind: DiffuserKind
  public var variant: String?
  public var firmware: String?
  public var reachability: Reachability
  public var power: Reading<Bool>?
  public var level: Reading<Level>?
  /// True when plugged in.
  public var pluggedIn: Reading<Bool>?
  public var battery: Reading<Int>?
  /// WiFi signal in dBm.
  public var signal: Reading<Int>?
  public var lastReport: Date?
  /// When it last rejoined the broker, if that came after its last report of power or
  /// intensity: what it is doing is then not known, since it may have come back otherwise
  /// (owner, 2026-10-22). `power` and `level` hold only reports made since.
  public var notKnownSince: Date?
  public var pending: Pending?
  /// The last failure, kept until a newer one replaces it or an action succeeds.
  public var failure: String?
  /// The last schedule sent from Scent Control (this phone or the Mac), if known.
  public var schedule: Profile?
  public var scheduleSentAt: Date?
  public var sendOver: SendOver
  /// The Bluetooth peripheral linked to this diffuser on this phone, once one is.
  public var bluetoothID: UUID?
  /// Whether it has been heard on our broker. One that hasn't may be on another broker or
  /// not on WiFi at all, so WiFi commands may not reach it until it is set up.
  public var onOurBroker: Bool

  public init(
    id: String, name: String? = nil, room: String? = nil, kind: DiffuserKind,
    variant: String? = nil, firmware: String? = nil, reachability: Reachability = .unknown,
    power: Reading<Bool>? = nil, level: Reading<Level>? = nil, pluggedIn: Reading<Bool>? = nil,
    battery: Reading<Int>? = nil, signal: Reading<Int>? = nil, lastReport: Date? = nil,
    notKnownSince: Date? = nil, pending: Pending? = nil, failure: String? = nil, schedule: Profile? = nil,
    scheduleSentAt: Date? = nil, sendOver: SendOver = .auto, bluetoothID: UUID? = nil,
    onOurBroker: Bool = true
  ) {
    self.id = id
    self.name = name
    self.room = room
    self.kind = kind
    self.variant = variant
    self.firmware = firmware
    self.reachability = reachability
    self.power = power
    self.level = level
    self.pluggedIn = pluggedIn
    self.battery = battery
    self.signal = signal
    self.lastReport = lastReport
    self.notKnownSince = notKnownSince
    self.pending = pending
    self.failure = failure
    self.schedule = schedule
    self.scheduleSentAt = scheduleSentAt
    self.sendOver = sendOver
    self.bluetoothID = bluetoothID
    self.onOurBroker = onOurBroker
  }

  /// What the list calls it: its name, or its model, never its raw ID.
  public var title: String { name ?? kind.modelName }

  /// The power to show: the one on its way, else the last reported.
  public var shownPower: Bool? { pending?.power ?? power?.value }

  /// The intensity to show: the one on its way, else the last reported.
  public var shownLevel: Level? { pending?.level ?? level?.value }

  /// True once a Bluetooth peripheral is linked to this diffuser.
  public var bluetoothLinked: Bool { bluetoothID != nil }

  /// The newest power or intensity reading's time and transport: where the state shown
  /// comes from.
  public var stateReported: (at: Date, via: Via)? {
    [power.map { ($0.at, $0.via) }, level.map { ($0.at, $0.via) }].compactMap { $0 }.max { $0.0 < $1.0 }
  }

  /// Whether the power shown is a report more than 5 minutes old with nothing on its way,
  /// as the Mac's room says: answering pings doesn't make an old report current, since a
  /// diffuser reports its state only when it changes.
  public func stateIsStale(now: Date) -> Bool {
    guard pending == nil, let at = power?.at else { return false }
    return now.timeIntervalSince(at) > TimeInterval(staleAfterSeconds)
  }

  /// The peripheral to set it up again with, as the add sheet takes one: it points it at
  /// the broker in Settings and restarts it.
  public var setUpTarget: FoundDiffuser? {
    bluetoothID.map { FoundDiffuser(id: $0, kind: kind, variant: variant) }
  }
}

/// A Scentiment diffuser a scan found that the app doesn't know yet.
public struct FoundDiffuser: Identifiable, Hashable, Sendable {
  /// The Bluetooth peripheral's identifier on this phone.
  public var id: UUID
  public var kind: DiffuserKind
  public var variant: String?
  /// Signal strength in dBm.
  public var rssi: Int?

  public init(id: UUID, kind: DiffuserKind, variant: String? = nil, rssi: Int? = nil) {
    self.id = id
    self.kind = kind
    self.variant = variant
    self.rssi = rssi
  }
}

/// The connection to the broker.
public enum BrokerStatus: Hashable, Sendable {
  case connected
  case connecting
  /// The broker turned the login down; the app stops trying until it changes.
  case loginRefused
  /// No broker address, or no password, has been saved.
  case notSetUp
  case unreachable(String)
}

/// The always-on listener (apps/server), by its heartbeat on the broker. While it isn't
/// running, what it shared may be out of date, and the app relies on what it hears itself.
public enum ListenerStatus: Hashable, Sendable {
  /// A heartbeat said so within the last `onlineForSeconds`.
  case running
  /// Its will came, or its last heartbeat is too old. `lastSeen` is when a heartbeat last
  /// said it was running, if one did.
  case notRunning(lastSeen: Date?)
  /// Nothing heard from it yet.
  case notSeen
}

/// The broker's address and login, minus the password, which lives in the Keychain.
public struct BrokerSettings: Hashable, Sendable {
  public var url: String
  public var username: String
  /// The `userid` a diffuser is given in `PING`, so it publishes under our route.
  public var routeID: String

  public init(url: String = "", username: String = "", routeID: String = "") {
    self.url = url
    self.username = username
    self.routeID = routeID
  }
}

public enum Appearance: String, CaseIterable, Hashable, Sendable {
  case system
  case light
  case dark
}

/// One line of the activity log, as a sentence, with the raw message behind it.
public struct LogLine: Identifiable, Hashable, Sendable {
  public enum Mark: Hashable, Sendable {
    case sent
    case heard
    case failure
  }

  public var id: Int
  public var at: Date
  public var mark: Mark
  public var sentence: String
  /// The message as sent or heard, passwords masked.
  public var detail: String?

  public init(id: Int, at: Date, mark: Mark, sentence: String, detail: String? = nil) {
    self.id = id
    self.at = at
    self.mark = mark
    self.sentence = sentence
    self.detail = detail
  }
}

/// Where adding a diffuser has got to.
public struct AddProgress: Hashable, Sendable {
  public enum Step: Hashable, Sendable {
    case read
    case joinWifi
    case moveToBroker
    case restart
    case waitForConnect
  }

  public enum StepState: Hashable, Sendable {
    case waiting
    case working
    /// Counting down, as the 25 s before the restart does.
    case countdown(until: Date, total: TimeInterval)
    case done
    case failed(String)
  }

  public var deviceID: String?
  public var kind: DiffuserKind
  public var variant: String?
  /// The name and room it was given when the add started.
  public var name: String?
  public var room: String?
  public var steps: [(step: Step, state: StepState)]
  /// True when the diffuser isn't on WiFi, so the WiFi name and password are needed.
  public var needsWifi: Bool
  public var finished: Bool

  public init(
    deviceID: String? = nil, kind: DiffuserKind, variant: String? = nil, name: String? = nil,
    room: String? = nil, steps: [(step: Step, state: StepState)], needsWifi: Bool = false,
    finished: Bool = false
  ) {
    self.deviceID = deviceID
    self.kind = kind
    self.variant = variant
    self.name = name
    self.room = room
    self.steps = steps
    self.needsWifi = needsWifi
    self.finished = finished
  }

  public static func == (lhs: AddProgress, rhs: AddProgress) -> Bool {
    lhs.deviceID == rhs.deviceID && lhs.kind == rhs.kind && lhs.variant == rhs.variant
      && lhs.name == rhs.name && lhs.room == rhs.room
      && lhs.needsWifi == rhs.needsWifi && lhs.finished == rhs.finished
      && lhs.steps.map(\.step) == rhs.steps.map(\.step)
      && lhs.steps.map(\.state) == rhs.steps.map(\.state)
  }

  public func hash(into hasher: inout Hasher) {
    hasher.combine(deviceID)
    hasher.combine(finished)
    hasher.combine(steps.map(\.step))
    hasher.combine(steps.map(\.state))
  }
}

/// Where sending a copied schedule to one diffuser has got to.
public enum CopyState: Hashable, Sendable {
  case sending
  /// The broker took it; the shared copy is updated.
  case copied
  case failed(String)
}
