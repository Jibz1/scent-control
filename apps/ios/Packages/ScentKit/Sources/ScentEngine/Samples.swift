import Foundation
import ScentProtocol

/// Made-up diffusers, schedules and log lines for previews and screenshots. The IDs are
/// the repo's test IDs; nothing here is a real device or a real login.
public enum Samples {
  /// A Thursday evening, 19:42 on this phone's clock.
  public static let now: Date = {
    var parts = DateComponents()
    (parts.year, parts.month, parts.day, parts.hour, parts.minute) = (2026, 10, 22, 19, 42)
    return Calendar.current.date(from: parts) ?? .now
  }()

  /// This phone's offset from UTC at `now`, in hours, so the sample times read as local.
  static var offsetHours: Double {
    Double(TimeZone.current.secondsFromGMT(for: now)) / 3600
  }


  static func time(_ text: String) -> HhMm { HhMm(parsing: text)! }

  public static let bedsideSchedule = Profile(
    alwaysOn: AlwaysOn(enable: false, level: .low),
    entries: [
      Entry(start: time("06:30"), end: time("08:00"), weekdays: [1, 2, 3, 4, 5], level: .low),
      Entry(start: time("08:30"), end: time("10:00"), weekdays: [0, 6], level: .low),
      Entry(start: time("19:00"), end: time("22:30"), weekdays: [0, 1, 2, 3, 4, 5, 6], level: .medium),
    ],
    timezoneHours: offsetHours)

  public static let hallwaySchedule = Profile(
    alwaysOn: AlwaysOn(enable: false, level: .low),
    entries: [
      Entry(start: time("06:00"), end: time("08:30"), weekdays: [1, 2, 3, 4, 5], level: .high),
      Entry(start: time("16:30"), end: time("18:30"), weekdays: [1, 2, 3, 4, 5], level: .medium),
      Entry(start: time("09:00"), end: time("12:00"), weekdays: [0, 6], level: .medium),
    ],
    timezoneHours: offsetHours)

  public static let foundAir2 = FoundDiffuser(
    id: UUID(uuidString: "6B0F5A0C-2D7E-4C55-9C64-0A1B2C3D4E5F")!, kind: .air2, variant: "Black",
    rssi: -61)

  public static func diffusers(at now: Date) -> [DiffuserState] {
    func ago(_ minutes: Double) -> Date { now - minutes * 60 }
    return [
      DiffuserState(
        id: "TESTDEVICE01", name: "Bedside", room: "Bedroom", kind: .air2,
        variant: "Graphite Gold", firmware: "2.30", reachability: .answering(.bluetooth),
        power: Reading(true, at: ago(1), via: .bluetooth),
        level: Reading(.medium, at: ago(1), via: .bluetooth),
        pluggedIn: Reading(true, at: ago(1), via: .bluetooth),
        signal: Reading(-58, at: ago(3), via: .wifi), lastReport: ago(1),
        schedule: bedsideSchedule, scheduleSentAt: ago(60 * 26),
        bluetoothID: UUID(uuidString: "0B5E7C1D-0000-4000-8000-000000000001")),
      DiffuserState(
        id: "TESTDEVICE02", name: "Hallway", room: "Downstairs", kind: .hvac2,
        variant: "White", firmware: "2.30", reachability: .answering(.wifi),
        power: Reading(false, at: ago(4), via: .wifi),
        level: Reading(.medium, at: ago(4), via: .wifi),
        pluggedIn: Reading(true, at: ago(4), via: .wifi),
        signal: Reading(-71, at: ago(4), via: .wifi), lastReport: ago(4),
        schedule: hallwaySchedule, scheduleSentAt: ago(60 * 24 * 3)),
      DiffuserState(
        id: "TESTDEVICE03", name: "Reading Nook", room: "Downstairs", kind: .air2,
        variant: "White", firmware: "2.30", reachability: .notAnswering,
        power: Reading(true, at: ago(60 * 3 + 12), via: .wifi),
        level: Reading(.low, at: ago(60 * 3 + 12), via: .wifi),
        pluggedIn: Reading(false, at: ago(60 * 3 + 12), via: .wifi),
        battery: Reading(64, at: ago(60 * 3 + 12), via: .wifi), lastReport: ago(60 * 3 + 12)),
    ]
  }

  public static func log(at now: Date) -> [LogLine] {
    func ago(_ minutes: Double) -> Date { now - minutes * 60 }
    return [
      LogLine(
        id: 9, at: ago(1), mark: .heard, sentence: "Bedside reported on, Medium, plugged in",
        detail: #"S:1,L:2,C:2,Bat:200"#),
      LogLine(
        id: 8, at: ago(1.2), mark: .sent, sentence: "Set intensity to Medium on Bedside, over Bluetooth",
        detail: #"{"action":"SET_LEVEL","payload":{"intensity":2}}-|"#),
      LogLine(
        id: 7, at: ago(4), mark: .heard, sentence: "Hallway reported off, plugged in",
        detail: #"{"action":"notification","payload":{"deviceId":"TESTDEVICE02","data":"enable:0"}}"#),
      LogLine(
        id: 6, at: ago(4.3), mark: .sent, sentence: "Turned off Hallway, over WiFi",
        detail: #"{"action":"TURN_ON","payload":{"on":0}}"#),
      LogLine(
        id: 5, at: ago(12), mark: .failure, sentence: "Reading Nook didn't answer its ping",
        detail: #"{"action":"PING"}"#),
      LogLine(id: 4, at: ago(31), mark: .sent, sentence: "Pinged 3 diffusers over WiFi"),
      LogLine(
        id: 3, at: ago(60 * 26), mark: .sent, sentence: "Sent Bedside a schedule with 3 times",
        detail: #"{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[…]},"timezone":-5.0,"schedule_flag":1}}"#),
    ]
  }

  /// An add halfway through: read, and pointed at the broker, waiting out its test login.
  public static func addUnderWay(at now: Date) -> AddProgress {
    AddProgress(
      deviceID: "TESTDEVICE04", kind: .air2, variant: "Black", name: "Kitchen Counter",
      room: "Kitchen",
      steps: [
        (.read, .done), (.moveToBroker, .countdown(until: now + 17, total: 25)), (.restart, .waiting),
        (.waitForConnect, .waiting),
      ])
  }

  public static let brokerSettings = BrokerSettings(
    url: "wss://broker.example.test/mqtt", username: "example-user", routeID: "example-route")

  /// A model full of sample data, run by a `SampleEngine`.
  @MainActor
  public static func model(frozen: Bool = true) -> AppModel {
    let at = frozen ? now : .now
    return AppModel(
      engine: SampleEngine(), diffusers: diffusers(at: at), broker: .connected,
      brokerSettings: brokerSettings, log: log(at: at), now: at)
  }
}
