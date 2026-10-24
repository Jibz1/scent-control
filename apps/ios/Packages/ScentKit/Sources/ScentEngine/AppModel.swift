import Foundation
import Observation
import ScentProtocol

/// Something a person asked for. The screens send these; an engine carries them out and
/// reports back by changing the model.
public enum Action: Sendable {
  case setPower(id: String, on: Bool)
  case setLevel(id: String, level: Level)
  case readStatus(id: String)
  case setSendOver(id: String, SendOver)
  case scan
  case sendSchedule(id: String, Profile)
  case copySchedule(from: String, to: [String])
  case rename(id: String, name: String?, room: String?)
  case add(found: UUID, name: String?, room: String?, wifiName: String?, wifiPassword: String?)
  case saveBroker(BrokerSettings, password: String?)
  case setAppearance(Appearance)
  /// The app came to the front (true) or went to the background (false).
  case appActive(Bool)
}

/// Carries out actions. The real one talks to diffusers; `SampleEngine` pretends to.
@MainActor
public protocol Engine: AnyObject {
  func attach(_ model: AppModel)
  func submit(_ action: Action)
}

/// What the screens draw, in one observable place.
@MainActor
@Observable
public final class AppModel {
  public var diffusers: [DiffuserState]
  public var found: [FoundDiffuser]
  public var scanning: Bool
  public var scanProblem: String?
  public var broker: BrokerStatus
  /// Whether the always-on listener is running.
  public var listener: ListenerStatus
  public var brokerSettings: BrokerSettings
  public var appearance: Appearance
  public var log: [LogLine]
  /// Something wrong that isn't about one diffuser, such as a file that couldn't be read.
  public var notice: String?
  /// Adds under way or just finished, by the found diffuser's identifier.
  public var adds: [UUID: AddProgress]
  /// Schedule copies under way or just finished, by the target's device ID.
  public var copies: [String: CopyState]
  /// The time the screens treat as now. Ticks once a second while the app is open.
  public var now: Date

  @ObservationIgnored private let engine: any Engine
  @ObservationIgnored private var clock: Task<Void, Never>?

  public init(
    engine: any Engine, diffusers: [DiffuserState] = [], found: [FoundDiffuser] = [],
    broker: BrokerStatus = .notSetUp, listener: ListenerStatus = .notSeen,
    brokerSettings: BrokerSettings = BrokerSettings(),
    appearance: Appearance = .system, log: [LogLine] = [], now: Date = .now
  ) {
    self.engine = engine
    self.diffusers = diffusers
    self.found = found
    self.scanning = false
    self.broker = broker
    self.listener = listener
    self.brokerSettings = brokerSettings
    self.appearance = appearance
    self.log = log
    self.adds = [:]
    self.copies = [:]
    self.now = now
    engine.attach(self)
  }

  public func send(_ action: Action) {
    engine.submit(action)
  }

  /// Keeps `now` current, for ages and the Today strip. `frozen` keeps the given time,
  /// for screenshots.
  public func startClock(frozen: Bool = false) {
    guard !frozen, clock == nil else { return }
    clock = Task { [weak self] in
      while !Task.isCancelled {
        try? await Task.sleep(for: .seconds(1))
        self?.now = .now
      }
    }
  }

  public func diffuser(_ id: String) -> DiffuserState? {
    diffusers.first { $0.id == id }
  }

  public func update(_ id: String, _ change: (inout DiffuserState) -> Void) {
    guard let index = diffusers.firstIndex(where: { $0.id == id }) else { return }
    change(&diffusers[index])
  }

  /// The rooms in the order the list shows them: named rooms alphabetically, then
  /// diffusers with no room.
  public var rooms: [(room: String?, diffusers: [DiffuserState])] {
    let grouped = Dictionary(grouping: diffusers, by: \.room)
    let named = grouped.keys.compactMap { $0 }.sorted {
      $0.localizedStandardCompare($1) == .orderedAscending
    }
    var rooms: [(room: String?, diffusers: [DiffuserState])] = named.map { room in
      (room, grouped[room, default: []].sorted { $0.title < $1.title })
    }
    if let loose = grouped[nil], !loose.isEmpty {
      rooms.append((nil, loose.sorted { $0.title < $1.title }))
    }
    return rooms
  }

  public var roomNames: [String] {
    Set(diffusers.compactMap(\.room)).sorted()
  }
}
