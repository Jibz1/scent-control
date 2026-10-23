import Foundation
import ScentBluetooth
import ScentMQTT
import ScentProtocol
import Testing

@testable import ScentEngine

/// Radios that touch nothing: they record what the engine asked for and answer as told.
@MainActor
final class FakeRadios: Radios {
  var report: (EngineEvent) -> Void = { _ in }
  var brokerUp = true
  var bluetoothFails: BluetoothError?
  var hits: [ScanHit] = []
  var infoText = "01|Diffuser|Black|TESTDEVICE04|2.30|true|3"
  private(set) var bluetoothSent: [(UUID, Command)] = []
  private(set) var wifiSent: [(String, Command)] = []
  private(set) var stored: [(String, Data)] = []
  private(set) var reconnects = 0
  private(set) var idled = 0

  func scan(seconds: TimeInterval) async throws(BluetoothError) -> [ScanHit] { hits }

  func readInfo(_ bleID: UUID) async throws(BluetoothError) -> String {
    if let bluetoothFails { throw bluetoothFails }
    return infoText
  }

  func bluetoothSend(_ bleID: UUID, _ command: Command) async throws(BluetoothError) {
    if let bluetoothFails { throw bluetoothFails }
    bluetoothSent.append((bleID, command))
  }

  func readStatus(_ bleID: UUID) async throws(BluetoothError) {
    if let bluetoothFails { throw bluetoothFails }
  }

  func wifiSend(_ deviceID: String, _ command: Command) async throws(MQTTError) {
    guard brokerUp else { throw MQTTError(message: "Not connected to the broker.") }
    wifiSent.append((deviceID, command))
  }

  func storeSchedule(_ deviceID: String, _ body: Data) async throws(MQTTError) {
    guard brokerUp else { throw MQTTError(message: "Not connected to the broker.") }
    stored.append((deviceID, body))
  }

  func reconnect(_ settings: BrokerSettings, password: String?) {
    reconnects += 1
  }

  func idle() { idled += 1 }

  /// The commands sent over WiFi to one diffuser, pings left out.
  func wifiActions(_ deviceID: String) -> [String] {
    wifiSent.filter { $0.0 == deviceID }.map { $0.1.action(via: .wifi) }.filter { $0 != "PING" }
  }
}

final class MemoryPasswords: PasswordStore {
  var saved: String?
  init(_ saved: String? = nil) { self.saved = saved }
  func password(for settings: BrokerSettings) -> String? { saved }
  func save(_ password: String, for settings: BrokerSettings) throws(KeychainError) { saved = password }
}

/// An engine on fake radios, a fake clock that moves only when the engine waits, and a
/// folder of its own.
@MainActor
struct Rig {
  let radios = FakeRadios()
  let model: AppModel
  let engine: LiveEngine
  let folder: URL
  let clock: Clock

  final class Clock: @unchecked Sendable {
    var now = 1_800_000_000
  }

  init(file: StoredFile? = nil, password: String? = "example-password") {
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent("scent-tests-\(UUID().uuidString)")
    if let file { try! Store(folder: folder).save(file) }
    self.folder = folder
    let clock = Clock()
    self.clock = clock
    engine = LiveEngine(
      folder: folder, radios: radios, passwords: MemoryPasswords(password), clock: { clock.now },
      pause: { duration in
        clock.now += Int(duration.components.seconds)
        await Task.yield()
      },
      // The ping and refresh loops run their first round, then wait out the test.
      loopPause: { _ in try? await Task.sleep(for: .seconds(3600)) })
    model = AppModel(engine: engine)
  }

  /// Lets the engine's tasks run until they settle.
  func settle(_ rounds: Int = 400) async {
    for _ in 0..<rounds { await Task.yield() }
  }

  func diffuser(_ id: String) -> DiffuserState? { model.diffuser(id) }
}

let bedside = UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
let newOne = UUID(uuidString: "99999999-2222-3333-4444-555555555555")!

/// A saved file with one diffuser on the broker and linked over Bluetooth, and a second
/// one known only from the broker.
func twoDiffusers(schedule: Profile? = nil) -> StoredFile {
  let reading = StoredDevice.StoredReading(value: 1, at: 1_799_999_000, via: "wifi")
  return StoredFile(
    devices: [
      StoredDevice(
        id: "TESTDEVICE01", label: "Bedside", room: "Bedroom", bleID: bedside.uuidString, info: nil,
        readings: ["power": reading], sentProfile: schedule.map(StoredProfile.init),
        sentProfileAt: schedule == nil ? nil : 1_799_000_000, sendOver: nil),
      StoredDevice(
        id: "TESTDEVICE02", label: "Hallway", room: nil, bleID: nil, info: nil, readings: ["power": reading],
        sentProfile: nil, sentProfileAt: nil, sendOver: nil),
    ],
    broker: .init(url: "wss://broker.example.test/mqtt", username: "example-user", routeID: "example-route"),
    ui: nil)
}

let evening = Profile(
  alwaysOn: AlwaysOn(enable: false, level: .low),
  entries: [
    Entry(start: HhMm(parsing: "19:00")!, end: HhMm(parsing: "22:30")!, weekdays: [0, 1, 2, 3, 4, 5, 6], level: .medium)
  ],
  timezoneHours: -5)

@MainActor
@Suite struct Engine {
  @Test func bluetoothComesFirstWhenTheDiffuserIsLinked() async {
    let rig = Rig(file: twoDiffusers())
    rig.model.send(.setPower(id: "TESTDEVICE01", on: true))
    await rig.settle()
    #expect(rig.radios.bluetoothSent.map { $0.1 } == [.turnOn(true)])
    #expect(rig.radios.wifiActions("TESTDEVICE01").isEmpty)
    #expect(rig.diffuser("TESTDEVICE01")?.pending?.power == true)
  }

  @Test func aBluetoothFailureFallsBackToWifiAndCoolsDown() async {
    let rig = Rig(file: twoDiffusers())
    rig.radios.bluetoothFails = BluetoothError(message: "No answer.", unanswered: false)
    rig.model.send(.setLevel(id: "TESTDEVICE01", level: .high))
    await rig.settle()
    #expect(rig.radios.wifiActions("TESTDEVICE01") == ["SET_LEVEL"])
    #expect(
      rig.model.log.contains {
        $0.sentence.contains("Bluetooth couldn't reach the diffuser, so this went over WiFi. No answer.")
      })
    // Within the cool-down, Auto goes straight to WiFi.
    rig.radios.bluetoothFails = nil
    rig.model.send(.setLevel(id: "TESTDEVICE01", level: .low))
    await rig.settle()
    #expect(rig.radios.bluetoothSent.isEmpty)
    #expect(rig.radios.wifiActions("TESTDEVICE01") == ["SET_LEVEL", "SET_LEVEL"])
  }

  @Test func forcedWifiWithNoBrokerSaysWhy() async {
    let rig = Rig(file: twoDiffusers())
    rig.radios.brokerUp = false
    rig.model.send(.setSendOver(id: "TESTDEVICE02", .wifi))
    rig.model.send(.setPower(id: "TESTDEVICE02", on: false))
    await rig.settle()
    #expect(rig.radios.wifiSent.isEmpty)
    #expect(rig.diffuser("TESTDEVICE02")?.failure == "Couldn't send the power command: The broker isn't connected.")
  }

  @Test func aScheduleGoesOverWifiIsSharedThenTheTimeFollows() async throws {
    let rig = Rig(file: twoDiffusers())
    rig.model.send(.sendSchedule(id: "TESTDEVICE01", evening))
    await rig.settle()
    #expect(rig.radios.wifiActions("TESTDEVICE01") == ["UPDATE_PROFILE", "SYNC_TIME"])
    let (id, body) = try #require(rig.radios.stored.first)
    #expect(id == "TESTDEVICE01")
    let shared = try decodeShared(body, now: rig.clock.now).get()
    #expect(shared.profile == evening)
    #expect(rig.diffuser("TESTDEVICE01")?.schedule == evening)
  }

  @Test func copyingSendsTheWholeTableToEachTarget() async {
    let rig = Rig(file: twoDiffusers(schedule: evening))
    rig.model.send(.copySchedule(from: "TESTDEVICE01", to: ["TESTDEVICE02"]))
    #expect(rig.model.copies["TESTDEVICE02"] == .sending)
    await rig.settle()
    #expect(rig.radios.wifiActions("TESTDEVICE02") == ["UPDATE_PROFILE", "SYNC_TIME"])
    #expect(rig.model.copies["TESTDEVICE02"] == .copied)
    #expect(rig.diffuser("TESTDEVICE02")?.schedule?.entries == evening.entries)
    #expect(rig.radios.stored.map(\.0) == ["TESTDEVICE02"])
  }

  @Test func aNewerSharedScheduleIsAdoptedAndAnOlderOneIgnored() async {
    let rig = Rig(file: twoDiffusers(schedule: evening))
    var morning = evening
    morning.entries[0].start = HhMm(parsing: "06:00")!
    morning.entries[0].end = HhMm(parsing: "08:00")!
    let older = encodeShared(morning, sentAt: 1_798_000_000)!
    rig.radios.report(.mqtt(topic: "scent-control/v1/schedule/TESTDEVICE01", body: older, at: rig.clock.now))
    #expect(rig.diffuser("TESTDEVICE01")?.schedule == evening)
    let newer = encodeShared(morning, sentAt: 1_799_500_000)!
    rig.radios.report(.mqtt(topic: "scent-control/v1/schedule/TESTDEVICE01", body: newer, at: rig.clock.now))
    #expect(rig.diffuser("TESTDEVICE01")?.schedule == morning)
  }

  @Test func theListenersRecordsAreNotLoggedAsSchedules() async {
    let rig = Rig(file: twoDiffusers())
    let before = rig.model.log.count
    let state = Data(#"{"v":1,"device_id":"TESTDEVICE01"}"#.utf8)
    rig.radios.report(.mqtt(topic: "scent-control/v1/state/TESTDEVICE01", body: state, at: rig.clock.now))
    rig.radios.report(.mqtt(topic: "scent-control/v1/listener", body: Data(#"{"v":1}"#.utf8), at: rig.clock.now))
    #expect(rig.model.log.count == before)
  }

  @Test func aScanLinksAKnownDiffuserByItsNameAndListsTheRest() async {
    // An advertised name ends with the device ID, which is 12 hex digits (a made-up one here).
    var file = twoDiffusers()
    file.devices[1].id = "0000000000A2"
    let rig = Rig(file: file)
    let hallway = UUID()
    rig.radios.hits = [
      ScanHit(id: hallway, name: "Scentiment-H B 0000000000a2", rssi: -60),
      ScanHit(id: newOne, name: "Scentiment-D W", rssi: -70),
    ]
    rig.model.send(.scan)
    await rig.settle()
    #expect(rig.diffuser("0000000000A2")?.bluetoothLinked == true)
    #expect(rig.model.found.map(\.id) == [newOne])
    #expect(rig.model.found.first?.kind == .air2)
  }

  @Test func addingRunsTheMacsStepsAndNamesTheDiffuser() async throws {
    let rig = Rig(file: twoDiffusers())
    rig.radios.hits = [ScanHit(id: newOne, name: "Scentiment-D B", rssi: -60)]
    rig.model.send(.scan)
    await rig.settle()
    rig.model.send(.add(found: newOne, name: "Kitchen Counter", room: "Kitchen", wifiName: nil, wifiPassword: nil))
    await rig.settle(2000)
    // Read, pointed at the broker, restarted: now it waits for the diffuser on the broker.
    let commands = rig.radios.bluetoothSent.map { $0.1.action(via: .bluetooth) }
    #expect(commands == ["SAVE_MQTT", "SECRET_COMMAND"])
    guard case .saveMqtt(let broker, let username, let password) = rig.radios.bluetoothSent[0].1 else {
      Issue.record("SAVE_MQTT first")
      return
    }
    #expect(broker == "wss://broker.example.test/mqtt" && username == "example-user")
    #expect(password.expose() == "example-password")
    let connected = #"{"action":"device_connected","payload":{"deviceId":"TESTDEVICE04","fw":"2.30","model":"Diffuser"}}"#
    rig.radios.report(
      .mqtt(topic: "scentiment-mqtt-prod/user/example-route", body: Data(connected.utf8), at: rig.clock.now + 1))
    await rig.settle(2000)
    #expect(rig.radios.wifiSent.contains { $0.0 == "TESTDEVICE04" && $0.1 == .ping(userid: "example-route") })
    let added = try #require(rig.diffuser("TESTDEVICE04"))
    #expect(added.name == "Kitchen Counter" && added.room == "Kitchen" && added.bluetoothLinked)
    #expect(rig.model.adds[newOne]?.finished == true)
    // No password reached the log.
    #expect(!rig.model.log.contains { ($0.detail ?? "").contains("example-password") })
  }

  @Test func addingADiffuserOffWifiAsksForANetwork() async {
    let rig = Rig(file: twoDiffusers())
    rig.radios.infoText = "01|Diffuser|Black|TESTDEVICE04|2.30|false|0"
    rig.model.send(.add(found: newOne, name: nil, room: nil, wifiName: nil, wifiPassword: nil))
    await rig.settle()
    #expect(rig.model.adds[newOne]?.needsWifi == true)
    #expect(rig.radios.bluetoothSent.isEmpty)
  }

  @Test func addingWithoutABrokerPasswordStopsBeforeWritingAnything() async {
    let rig = Rig(file: twoDiffusers(), password: nil)
    rig.model.send(.add(found: newOne, name: nil, room: nil, wifiName: nil, wifiPassword: nil))
    await rig.settle()
    #expect(rig.radios.bluetoothSent.isEmpty)
    #expect(
      rig.model.adds[newOne]?.steps.first?.state == .failed("Save the broker password in Settings first."))
  }

  @Test func theRoundPingsDiffusersOnTheBrokerWithTheRouteID() async {
    let rig = Rig(file: twoDiffusers())
    await rig.settle()
    let pinged = Set(rig.radios.wifiSent.filter { $0.1 == .ping(userid: "example-route") }.map(\.0))
    #expect(pinged == ["TESTDEVICE01", "TESTDEVICE02"])
  }

  @Test func namesRoomsAndSchedulesSurviveARelaunch() async throws {
    let first = Rig(file: twoDiffusers(schedule: evening))
    first.model.send(.rename(id: "TESTDEVICE02", name: "  Hall   Closet ", room: "bedroom"))
    await first.settle()
    let saved = try #require(try Store(folder: first.folder).load())
    let again = Rig(file: saved)
    #expect(again.diffuser("TESTDEVICE02")?.name == "Hall Closet")
    // A room in other capitals is the room in use.
    #expect(again.diffuser("TESTDEVICE02")?.room == "Bedroom")
    #expect(again.diffuser("TESTDEVICE01")?.schedule == evening)
    #expect(again.diffuser("TESTDEVICE01")?.bluetoothLinked == true)
  }

  @Test func goingToTheBackgroundLetsGoOfBluetooth() async {
    let rig = Rig(file: twoDiffusers())
    rig.model.send(.appActive(false))
    #expect(rig.radios.idled == 1)
  }
}

@Suite struct Pure {
  @Test func autoIsBluetoothFirstAndForcedHasNoFallback() {
    #expect(route(.auto, bluetooth: true, wifi: true) == [.bluetooth, .wifi])
    #expect(route(.auto, bluetooth: false, wifi: true) == [.wifi])
    #expect(route(.wifi, bluetooth: true, wifi: true) == [.wifi])
    #expect(route(.bluetooth, bluetooth: true, wifi: false) == [.bluetooth])
    #expect(route(.auto, bluetooth: false, wifi: false).isEmpty)
  }

  @Test func theCoolDownEndsAfterTwoMinutesOrWhenHeardAgain() {
    #expect(bluetoothWorthTrying(failedAt: nil, heardBluetooth: nil, now: 100))
    #expect(!bluetoothWorthTrying(failedAt: 100, heardBluetooth: nil, now: 219))
    #expect(bluetoothWorthTrying(failedAt: 100, heardBluetooth: nil, now: 220))
    #expect(bluetoothWorthTrying(failedAt: 100, heardBluetooth: 101, now: 150))
  }

  @Test func onWifiReadsTheStatusOnlyAfterFirmwareTwo() {
    func info(_ firmware: String, _ configured: Bool, _ status: UInt8) -> DeviceInfo {
      DeviceInfo(
        manufacturerID: "01", modelName: "Diffuser", variantName: "Black", deviceID: "TESTDEVICE01",
        firmware: firmware, wifiConfigured: configured, wifiStatus: status)
    }
    #expect(onWifi(info("2.30", true, 3)))
    #expect(!onWifi(info("2.30", true, 0)))
    #expect(onWifi(info("2.0", true, 0)))
    #expect(!onWifi(info("2.30", false, 3)))
    #expect(onWifi(info("garbled", true, 0)))
  }

  @Test func aSharedScheduleRoundTripsAndRefusesOtherVersionsAndTheFuture() throws {
    let body = try #require(encodeShared(evening, sentAt: 1_000))
    #expect(try decodeShared(body, now: 1_000).get() == SharedSchedule(profile: evening, sentAt: 1_000))
    #expect(decodeShared(Data(#"{"v":2}"#.utf8), now: 0) == .failure(.version))
    #expect(decodeShared(Data("not json".utf8), now: 0) == .failure(.invalid))
    #expect(decodeShared(body, now: 1_000 - 86_401) == .failure(.invalid))
  }

  @Test func theMacsSharedScheduleIsReadAsTheMacWritesIt() throws {
    // As serde writes it on the Mac: keys in its own order, the offset as a float.
    let mac = #"""
      {"v":1,"profile":{"always_on":{"enable":true,"level":2},"entries":[{"start":"07:30","end":"09:00","weekdays":[1,2,3,4,5],"level":1,"enable":true}],"timezone_hours":-5.0,"flag":"replace"},"sent_at":1800000000}
      """#
    let shared = try decodeShared(Data(mac.utf8), now: 1_800_000_000).get()
    #expect(shared.profile.alwaysOn == AlwaysOn(enable: true, level: .medium))
    #expect(shared.profile.entries.first?.start == HhMm(parsing: "07:30"))
    #expect(shared.profile.timezoneHours == -5)
  }

  @Test func namesAreTidied() {
    #expect(cleanName("  Living \t Room  ") == "Living Room")
    #expect(cleanName(String(repeating: "a", count: 60)).count == nameMaxCharacters)
    #expect(cleanName("\u{7}") == "")
  }

  @Test func failureLinesSayWhatWorkedInstead() {
    let name: (Via) -> String = { _ in "TURN_ON" }
    #expect(failureLine(name, [], nil) == nil)
    let fell = failureLine(name, [(.bluetooth, "No answer.")], .wifi)
    #expect(fell?.0 == "Bluetooth TURN_ON")
    #expect(fell?.1 == "Bluetooth couldn't reach the diffuser, so this went over WiFi. No answer.")
    #expect(failureLine(name, [(.bluetooth, "A."), (.wifi, "B.")], nil)?.1 == "Neither route worked. Bluetooth: A. WiFi: B.")
  }
}
