import Foundation
import Testing

@testable import ScentBluetooth

// The central's whole flow against a fake Core Bluetooth and a clock the test moves:
// scanning, opening links, reads and writes, timeouts, and letting go. Every suite has a
// time limit, so a request that never ends fails its test instead of hanging the run.

private let hint = Wording.connectHint
private let info = "01|Diffuser|Black|TESTDEVICE01|2.30|true|3"

@MainActor @Suite(.timeLimit(.minutes(1))) struct SetUpTests {
  @Test func theRealCentralTouchesNoBluetoothUntilARequestNeedsIt() {
    // Creating Core Bluetooth's manager outside an app that declares Bluetooth use ends
    // the process on macOS, so this test passing also shows nothing created it.
    let central = BluetoothCentral(events: { _ in })
    #expect(central.availability == .unknown)
    central.disconnectAll()
    #expect(!central.isRadioStarted)
  }

  @Test func bluetoothIsSetUpByTheFirstRequest() async throws {
    let rig = Rig()
    #expect(rig.central.availability == .unknown)
    #expect(rig.radio.calls.isEmpty)
    _ = try await rig.central.scan(seconds: 0)
    #expect(rig.radio.calls.first == .start)
    #expect(rig.radio.count(.start) == 1)
    #expect(rig.central.availability == .poweredOn)
    #expect(rig.availabilities == [.poweredOn])
  }

  @Test func bluetoothThatIsNotReadyYetIsWaitedFor() async {
    let rig = Rig()
    rig.radio.stateOnStart = nil
    let id = rig.diffuser()
    let request = Pending { try await rig.central.readStatus(id) }
    await rig.until("set up") { rig.radio.isStarted }
    await rig.pass(.milliseconds(2_500))
    #expect(!rig.radio.calls.contains(.connect(id)))
    rig.radio.emit(.stateChanged(.poweredOn))
    #expect(await rig.error(request) == nil)
    #expect(rig.radio.count(.connect(id)) == 1)
  }

  @Test func bluetoothStillNotReadyAfter3SecondsIsGivenUpOn() async {
    let rig = Rig()
    rig.radio.stateOnStart = nil
    let scan = Pending { try await rig.central.scan(seconds: 1) }
    await rig.until("set up") { rig.radio.isStarted }
    await rig.pass(.milliseconds(2_900))
    #expect(!scan.isDone)
    await rig.pass(.milliseconds(100))
    #expect(await rig.error(scan) == BluetoothError(message: Wording.notReady(.unknown)!))
    #expect(!rig.radio.calls.contains(.startScan))
  }

  @Test func whileIOSAsksForPermissionTheWaitIsAMinute() async {
    let rig = Rig()
    rig.radio.stateOnStart = nil
    rig.radio.authorizationPending = true
    let id = rig.diffuser()
    let answered = Pending { try await rig.central.readInfo(id) }
    await rig.until("set up") { rig.radio.isStarted }
    await rig.pass(.seconds(20))
    #expect(!answered.isDone)
    // Unauthorized before the person has answered isn't their answer.
    rig.radio.emit(.stateChanged(.unauthorized))
    await rig.pass(.seconds(20))
    #expect(!answered.isDone)
    rig.radio.authorizationPending = false
    rig.radio.emit(.stateChanged(.unauthorized))
    #expect(await rig.error(answered) == BluetoothError(message: Wording.notReady(.unauthorized)!))

    let other = Rig()
    other.radio.stateOnStart = nil
    other.radio.authorizationPending = true
    let unanswered = Pending { try await other.central.scan(seconds: 1) }
    await other.until("set up") { other.radio.isStarted }
    await other.pass(.milliseconds(59_900))
    #expect(!unanswered.isDone)
    await other.pass(.milliseconds(100))
    #expect(await other.error(unanswered) == BluetoothError(message: Wording.notReady(.unknown)!))
  }

  @Test func bluetoothTurnedOffSaysSoAndTouchesNoDiffuser() async {
    let rig = Rig()
    rig.radio.stateOnStart = .poweredOff
    let id = rig.diffuser()
    let send = Pending { try await rig.central.send(id, frame: Data("x-|".utf8)) }
    #expect(await rig.error(send) == BluetoothError(message: Wording.notReady(.poweredOff)!))
    #expect(rig.radio.calls == [.start])
    #expect(rig.availabilities == [.poweredOff])
  }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct ScanTests {
  @Test func aScanReportsTheScentimentDiffusersItHeard() async throws {
    let rig = Rig()
    let air = UUID(uuid: (1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let hvac = UUID(uuid: (2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let far = UUID(uuid: (3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let speaker = UUID(uuid: (4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    rig.radio.advertising = [
      hvac: Sighting(name: "Scentiment-H B n 0A1B2C3D4E02", rssi: -60),
      air: Sighting(name: "Scentiment-D B n 0A1B2C3D4E01", rssi: -71),
      far: Sighting(name: "Scentiment-D B n 0A1B2C3D4E03", rssi: -95),
      speaker: Sighting(name: "JBL Flip 6", rssi: -40),
    ]
    let scan = Pending { try await rig.central.scan(seconds: 5) }
    await rig.until("scanning") { rig.radio.calls.contains(.startScan) }
    await rig.pass(.milliseconds(4_900))
    #expect(!scan.isDone)
    #expect(!rig.radio.calls.contains(.stopScan), "still scanning")
    await rig.pass(.milliseconds(100))
    #expect(
      try await rig.value(scan) == [
        ScanHit(id: air, name: "Scentiment-D B n 0A1B2C3D4E01", rssi: -71),
        ScanHit(id: hvac, name: "Scentiment-H B n 0A1B2C3D4E02", rssi: -60),
      ])
    #expect(rig.radio.calls == [.start, .startScan, .stopScan])
  }

  @Test func aDiffuserHeardEarlierInThisLaunchIsStillListed() async throws {
    let rig = Rig()
    let air = UUID(uuid: (1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    rig.radio.advertising = [air: Sighting(name: "Scentiment-D B n 0A1B2C3D4E01", rssi: -71)]
    let first = Pending { try await rig.central.scan(seconds: 1) }
    await rig.until("scanning") { rig.radio.calls.contains(.startScan) }
    await rig.pass(.seconds(1))
    #expect(try await rig.value(first).map(\.id) == [air])

    rig.radio.advertising = [:]
    let second = Pending { try await rig.central.scan(seconds: 1) }
    await rig.until("scanning again") { rig.radio.count(.startScan) == 2 }
    await rig.pass(.seconds(1))
    #expect(try await rig.value(second).map(\.id) == [air])
  }

  @Test func overlappingScansShareOneScan() async throws {
    let rig = Rig()
    let short = Pending { try await rig.central.scan(seconds: 2) }
    await rig.until("scanning") { rig.radio.calls.contains(.startScan) }
    let long = Pending { try await rig.central.scan(seconds: 4) }
    await rig.settle()
    await rig.pass(.seconds(2))
    _ = try await rig.value(short)
    #expect(rig.radio.count(.startScan) == 1)
    #expect(rig.radio.count(.stopScan) == 0, "the longer scan still runs")
    await rig.pass(.seconds(2))
    _ = try await rig.value(long)
    #expect(rig.radio.count(.stopScan) == 1)
  }

  @Test func aScanEndsWhenBluetoothTurnsOff() async {
    let rig = Rig()
    let scan = Pending { try await rig.central.scan(seconds: 10) }
    await rig.until("scanning") { rig.radio.calls.contains(.startScan) }
    rig.radio.emit(.stateChanged(.poweredOff))
    #expect(await rig.error(scan) == BluetoothError(message: Wording.notReady(.poweredOff)!))
    #expect(rig.availabilities == [.poweredOn, .poweredOff])
  }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct LinkTests {
  @Test func readInfoOpensALinkAndReadsFef4() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.values[.info] = Data(info.utf8)
    #expect(try await rig.central.readInfo(id) == info)
    // Remembered by Bluetooth, so reached with no scan.
    #expect(
      rig.radio.calls == [
        .start, .retrieve(id), .connect(id), .discoverServices(id), .discoverCharacteristics(id),
        .subscribe(id), .read(id, .info),
      ])
  }

  @Test func theInfoIsReadAsUtf8WithBadBytesReplaced() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.values[.info] = Data([0x30, 0x31, 0xFF, 0x7C])
    #expect(try await rig.central.readInfo(id) == "01\u{FFFD}|")
  }

  @Test func laterRequestsUseTheOpenLink() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    _ = try await rig.central.readInfo(id)
    try await rig.central.readStatus(id)
    try await rig.central.send(id, frame: Data("{\"action\":\"PING\"}-|".utf8))
    #expect(rig.radio.count(.connect(id)) == 1)
  }

  @Test func aDiffuserNotRememberedIsLookedForWithAScan() async throws {
    let rig = Rig()
    let id = UUID(uuid: (9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    rig.radio.advertising = [id: Sighting(name: "Scentiment-D B n 0A1B2C3D4E01", rssi: -70)]
    try await rig.central.readStatus(id)
    #expect(
      Array(rig.radio.calls.prefix(5)) == [
        .start, .retrieve(id), .startScan, .stopScan, .connect(id),
      ])
  }

  @Test func aDiffuserAScanHeardIsReachedWithoutAnotherScan() async throws {
    let rig = Rig()
    let id = UUID(uuid: (9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    rig.radio.advertising = [id: Sighting(name: "Scentiment-D B n 0A1B2C3D4E01", rssi: -70)]
    _ = try await rig.central.scan(seconds: 0)
    await rig.settle()
    try await rig.central.readStatus(id)
    #expect(rig.radio.count(.startScan) == 1)
    #expect(rig.radio.count(.connect(id)) == 1)
  }

  @Test func aDiffuserThatNeverAdvertisesIsGivenUpOnAfter10Seconds() async {
    let rig = Rig()
    let id = UUID(uuid: (9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let request = Pending { try await rig.central.readInfo(id) }
    await rig.until("scanning") { rig.radio.calls.contains(.startScan) }
    await rig.pass(.milliseconds(9_900))
    #expect(!rig.radio.calls.contains(.stopScan))
    await rig.pass(.milliseconds(100))
    #expect(
      await rig.error(request)
        == BluetoothError(message: "Timed out while looking for the diffuser. " + hint))
    #expect(rig.radio.calls.last == .stopScan)
    #expect(!rig.radio.calls.contains(.connect(id)))
  }

  @Test func aConnectionThatNeverCompletesIsCancelledAfter10Seconds() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.connect]
    let request = Pending { try await rig.central.send(id, frame: Data("x-|".utf8)) }
    await rig.until("connecting") { rig.radio.calls.contains(.connect(id)) }
    await rig.pass(.seconds(10))
    #expect(
      await rig.error(request) == BluetoothError(message: "Timed out while connecting. " + hint))
    #expect(rig.radio.calls.last == .cancel(id))
    #expect(rig.radio.writes(to: id).isEmpty)
  }

  @Test func aConnectionThatComesThroughAfterItsAttemptTimedOutIsLetGo() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.connect]
    let request = Pending { try await rig.central.readStatus(id) }
    await rig.until("connecting") { rig.radio.calls.contains(.connect(id)) }
    await rig.pass(.seconds(10))
    #expect(await rig.error(request) != nil)
    #expect(rig.radio.count(.cancel(id)) == 1)
    rig.radio.connected(id)
    #expect(rig.radio.count(.cancel(id)) == 2, "a diffuser takes one connection at a time")
  }

  @Test func requestsQueuedBehindALinkThatCouldNotOpenFailWithIt() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.connect]
    let first = Pending { try await rig.central.send(id, frame: Data("a-|".utf8)) }
    await rig.until("connecting") { rig.radio.calls.contains(.connect(id)) }
    let second = Pending { try await rig.central.readStatus(id) }
    let third = Pending { try await rig.central.readInfo(id) }
    await rig.settle()
    await rig.pass(.seconds(10))
    let expected = BluetoothError(message: "Timed out while connecting. " + hint)
    #expect(await rig.error(first) == expected)
    #expect(await rig.error(second) == expected)
    #expect(await rig.error(third) == expected)
    #expect(rig.radio.count(.connect(id)) == 1)
  }

  @Test func aConnectFailureSaysWhatBluetoothSaid() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.failures[.connect] = RadioFailure("Peer removed pairing information.")
    let request = Pending { try await rig.central.readInfo(id) }
    #expect(
      await rig.error(request)
        == BluetoothError(
          message: "Bluetooth failed while connecting: Peer removed pairing information. " + hint))
  }

  @Test func aDeviceWithoutOurCharacteristicsIsNotADiffuser() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.gatt = [.write, .status]
    let request = Pending { try await rig.central.readInfo(id) }
    #expect(
      await rig.error(request)
        == BluetoothError(
          message:
            "The device has no Bluetooth characteristic 0xFEF4, so it is not a diffuser we can "
            + "talk to."))
    #expect(rig.radio.calls.contains(.cancel(id)))
    #expect(!rig.radio.calls.contains(.subscribe(id)))
  }

  @Test func aDiffuserThatDropsTheLinkIsReconnectedOnTheNextRequest() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    try await rig.central.readStatus(id)
    rig.radio.drop(id)
    try await rig.central.readStatus(id)
    #expect(rig.radio.count(.connect(id)) == 2)
    #expect(rig.radio.count(.cancel(id)) == 0, "there was nothing left to disconnect")
  }

  @Test func aLinkThatWentDownWithoutAWordIsOpenedAfresh() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    try await rig.central.readStatus(id)
    rig.radio.connections[id] = .disconnected
    try await rig.central.readStatus(id)
    #expect(rig.radio.count(.connect(id)) == 2)
  }

  @Test func aDisconnectLeftOverFromAnEarlierLinkDoesNotEndTheNextOne() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.connect]
    let request = Pending { try await rig.central.readStatus(id) }
    await rig.until("connecting") { rig.radio.calls.contains(.connect(id)) }
    rig.radio.emit(.disconnected(id, nil))
    rig.radio.holding = []
    rig.radio.connected(id)
    #expect(await rig.error(request) == nil)
  }

  @Test func theNextLinkWaitsUpTo3SecondsForADisconnectToBeConfirmed() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.confirmsDisconnect = false
    rig.radio.failures[.read] = RadioFailure("Reading is not permitted.")
    let failed = Pending { try await rig.central.readStatus(id) }
    #expect(
      await rig.error(failed)
        == BluetoothError(message: "Couldn't read the diffuser's status: Reading is not permitted.")
    )
    #expect(rig.radio.calls.last == .cancel(id))

    rig.radio.failures = [:]
    let next = Pending { try await rig.central.readStatus(id) }
    await rig.settle()
    await rig.pass(.milliseconds(2_900))
    #expect(rig.radio.count(.connect(id)) == 1, "still waiting for the disconnect")
    await rig.pass(.milliseconds(100))
    #expect(await rig.error(next) == nil)
    #expect(rig.radio.count(.connect(id)) == 2)
  }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct SendTests {
  let frame = Data((0..<400).map { UInt8(32 + $0 % 90) })

  @Test func aFrameIsWrittenInPartsEachAfterTheLastIsAcknowledged() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let request = Pending { try await rig.central.send(id, frame: frame) }
    for n in 1...3 {
      await rig.until("write \(n)") { rig.radio.writes(to: id).count == n }
      await rig.settle()
      #expect(rig.radio.writes(to: id).count == n, "one part at a time")
      #expect(!request.isDone)
      rig.radio.emit(.wrote(id, nil))
    }
    #expect(await rig.error(request) == nil)
    #expect(rig.radio.writes(to: id).map(\.count) == [177, 177, 46])
    #expect(rig.radio.writes(to: id).reduce(Data(), +) == frame)
  }

  @Test func theLinkStaysOpenAfterASendThenClosesWhenIdle() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    try await rig.central.send(id, frame: Data("{\"action\":\"PING\"}-|".utf8))
    await rig.pass(.milliseconds(9_900))
    #expect(!rig.radio.calls.contains(.cancel(id)))
    await rig.pass(.milliseconds(100))
    #expect(rig.radio.count(.cancel(id)) == 1)
  }

  @Test func usingTheLinkKeepsItOpen() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    try await rig.central.send(id, frame: Data("a-|".utf8))
    await rig.pass(.seconds(8))
    try await rig.central.readStatus(id)
    await rig.pass(.milliseconds(9_900))
    #expect(!rig.radio.calls.contains(.cancel(id)))
    await rig.pass(.milliseconds(100))
    #expect(rig.radio.count(.cancel(id)) == 1)
    #expect(rig.radio.count(.connect(id)) == 1)
  }

  @Test func aLastPartThatGoesUnansweredLeavesTheCommandUnknown() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let request = Pending { try await rig.central.send(id, frame: frame) }
    for n in 1...2 {
      await rig.until("write \(n)") { rig.radio.writes(to: id).count == n }
      rig.radio.emit(.wrote(id, nil))
    }
    await rig.until("the last write") { rig.radio.writes(to: id).count == 3 }
    await rig.pass(.seconds(10))
    #expect(
      await rig.error(request)
        == BluetoothError(
          message:
            "The diffuser didn't answer within 10 s. The command may have gone through anyway.",
          unanswered: true))
    #expect(rig.radio.calls.last == .cancel(id), "the link is closed")
  }

  @Test func anEarlierPartThatGoesUnansweredIsAPlainFailure() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let request = Pending { try await rig.central.send(id, frame: frame) }
    await rig.until("the first write") { rig.radio.writes(to: id).count == 1 }
    await rig.pass(.seconds(10))
    #expect(
      await rig.error(request) == BluetoothError(message: "The diffuser didn't answer within 10 s.")
    )
  }

  @Test func aLinkThatDropsDuringTheLastWriteLeavesTheCommandUnknown() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let restart = Data(
      "{\"action\":\"SECRET_COMMAND\",\"payload\":{\"command\":\"restart\"}}-|".utf8)
    let request = Pending { try await rig.central.send(id, frame: restart) }
    await rig.until("the write") { rig.radio.writes(to: id).count == 1 }
    rig.radio.drop(id)
    #expect(
      await rig.error(request)
        == BluetoothError(
          message: "Couldn't write to the diffuser: The specified device has disconnected from us. "
            + "The command may have gone through anyway.", unanswered: true))
  }

  @Test func aRefusedWriteIsAPlainFailure() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.failures[.write] = RadioFailure("Writing is not permitted.")
    let request = Pending { try await rig.central.send(id, frame: Data("x-|".utf8)) }
    #expect(
      await rig.error(request)
        == BluetoothError(message: "Couldn't write to the diffuser: Writing is not permitted."))
  }

  @Test func aFailureNeverCarriesTheFrame() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let secret =
      "{\"action\":\"WIFI_CONNECT\",\"payload\":{\"ssid\":\"ExampleWiFi\",\"password\":\"hunter2!\"}}-|"
    let request = Pending { try await rig.central.send(id, frame: Data(secret.utf8)) }
    await rig.until("the write") { rig.radio.writes(to: id).count == 1 }
    await rig.pass(.seconds(10))
    let error = await rig.error(request)
    #expect(error != nil)
    #expect(error?.message.contains("hunter2!") == false)
    #expect(error?.message.contains("ExampleWiFi") == false)
  }

  @Test func bluetoothTurningOffDuringTheLastWriteLeavesTheCommandUnknown() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let request = Pending { try await rig.central.send(id, frame: Data("x-|".utf8)) }
    await rig.until("the write") { rig.radio.writes(to: id).count == 1 }
    rig.radio.emit(.stateChanged(.poweredOff))
    #expect(
      await rig.error(request)
        == BluetoothError(
          message: Wording.notReady(.poweredOff)! + " The command may have gone through anyway.",
          unanswered: true))
    #expect(rig.availabilities == [.poweredOn, .poweredOff])
  }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct NotificationTests {
  @Test func notificationsAreForwardedWhileTheLinkIsOpen() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    try await rig.central.send(
      id, frame: Data("{\"action\":\"TURN_ON\",\"payload\":{\"on\":1}}-|".utf8))
    rig.radio.emit(.valueUpdated(id, .status, Data("enable:1".utf8), nil))
    await rig.pass(.seconds(1))
    rig.radio.emit(.valueUpdated(id, .status, Data("level:2-|".utf8), nil))
    // Only 0xFEF3 is forwarded.
    rig.radio.emit(.valueUpdated(id, .info, Data(info.utf8), nil))
    #expect(rig.notifications.map(\.0) == [id, id])
    #expect(rig.notifications.map(\.1) == [Data("enable:1".utf8), Data("level:2-|".utf8)])
    guard case .notification(_, _, let at) = rig.events.last else {
      Issue.record("no notification")
      return
    }
    #expect(at == rig.timing.date)

    // Once the link has closed, nothing more is forwarded.
    await rig.pass(.seconds(10))
    #expect(rig.radio.calls.contains(.cancel(id)))
    rig.radio.emit(.valueUpdated(id, .status, Data("enable:0".utf8), nil))
    #expect(rig.notifications.count == 2)
  }

  @Test func readStatusReadsFef3AndReportsItOnceAsANotification() async throws {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.values[.status] = Data("S:0,L:0,C:2,Bat:200".utf8)
    try await rig.central.readStatus(id)
    #expect(rig.radio.calls.last == .read(id, .status))
    #expect(rig.notifications.map(\.0) == [id])
    #expect(rig.notifications.map(\.1) == [Data("S:0,L:0,C:2,Bat:200".utf8)])
  }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct OrderTests {
  @Test func oneRequestAtATimePerDiffuser() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let first = Pending { try await rig.central.send(id, frame: Data("a-|".utf8)) }
    await rig.until("the first write") { rig.radio.writes(to: id).count == 1 }
    let second = Pending { try await rig.central.send(id, frame: Data("b-|".utf8)) }
    await rig.settle()
    #expect(rig.radio.writes(to: id) == [Data("a-|".utf8)], "the second waits its turn")
    rig.radio.emit(.wrote(id, nil))
    #expect(await rig.error(first) == nil)
    await rig.until("the second write") { rig.radio.writes(to: id).count == 2 }
    rig.radio.emit(.wrote(id, nil))
    #expect(await rig.error(second) == nil)
    #expect(rig.radio.writes(to: id) == [Data("a-|".utf8), Data("b-|".utf8)])
  }

  @Test func differentDiffusersGoSideBySide() async {
    let rig = Rig()
    let air = rig.diffuser(1)
    let hvac = rig.diffuser(2)
    rig.radio.holding = [.connect]
    let one = Pending { try await rig.central.readStatus(air) }
    let two = Pending { try await rig.central.readStatus(hvac) }
    await rig.until("both connecting") {
      rig.radio.calls.contains(.connect(air)) && rig.radio.calls.contains(.connect(hvac))
    }
    rig.radio.holding = []
    rig.radio.connected(hvac)
    rig.radio.connected(air)
    #expect(await rig.error(one) == nil)
    #expect(await rig.error(two) == nil)
  }

  @Test func moreThan16WaitingRequestsAreRefused() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let first = Pending { try await rig.central.send(id, frame: Data("a-|".utf8)) }
    await rig.until("the first write") { rig.radio.writes(to: id).count == 1 }
    let queued = (0..<16).map { _ in Pending { try await rig.central.readStatus(id) } }
    await rig.settle()
    let refused = Pending { try await rig.central.readStatus(id) }
    #expect(await rig.error(refused) == BluetoothError(message: Wording.busy))
    rig.radio.holding = []
    rig.radio.emit(.wrote(id, nil))
    #expect(await rig.error(first) == nil)
    for request in queued { #expect(await rig.error(request) == nil) }
  }

  @Test func aCancelledRequestLeavesTheQueueAndIsNeverSent() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.write]
    let first = Pending { try await rig.central.send(id, frame: Data("a-|".utf8)) }
    await rig.until("the first write") { rig.radio.writes(to: id).count == 1 }
    let cancelled = Pending { try await rig.central.send(id, frame: Data("b-|".utf8)) }
    await rig.settle()
    let last = Pending { try await rig.central.send(id, frame: Data("c-|".utf8)) }
    await rig.settle()
    cancelled.cancel()
    #expect(await rig.error(cancelled) == BluetoothError(message: Wording.cancelled))
    #expect(!first.isDone, "it ended at once, while the request ahead was still going")
    rig.radio.holding = []
    rig.radio.emit(.wrote(id, nil))
    #expect(await rig.error(first) == nil)
    #expect(await rig.error(last) == nil)
    #expect(rig.radio.writes(to: id) == [Data("a-|".utf8), Data("c-|".utf8)])
  }

  @Test func aRequestCancelledWhileItsLinkOpensIsNeverSent() async {
    let rig = Rig()
    let id = rig.diffuser()
    rig.radio.holding = [.subscribe]
    let request = Pending { try await rig.central.send(id, frame: Data("a-|".utf8)) }
    await rig.until("subscribing") { rig.radio.calls.contains(.subscribe(id)) }
    // The link opens, but the caller has given up by the time the request goes on.
    rig.radio.emit(.subscribed(id, nil))
    request.cancel()
    #expect(await rig.error(request) == BluetoothError(message: Wording.cancelled))
    #expect(rig.radio.writes(to: id).isEmpty)

    // The link it opened is still there for the next request.
    rig.radio.holding = []
    let next = Pending { try await rig.central.readStatus(id) }
    #expect(await rig.error(next) == nil)
    #expect(rig.radio.count(.connect(id)) == 1)
  }

  @Test func disconnectAllLetsGoOfEveryDiffuser() async throws {
    let rig = Rig()
    let air = rig.diffuser(1)
    let hvac = rig.diffuser(2)
    try await rig.central.readStatus(air)
    try await rig.central.readStatus(hvac)
    rig.radio.holding = [.write]
    let inFlight = Pending { try await rig.central.send(air, frame: Data("a-|".utf8)) }
    await rig.until("the write") { rig.radio.writes(to: air).count == 1 }
    let queued = Pending { try await rig.central.send(air, frame: Data("b-|".utf8)) }
    let scan = Pending { try await rig.central.scan(seconds: 10) }
    await rig.settle()

    rig.central.disconnectAll()
    #expect(
      await rig.error(inFlight)
        == BluetoothError(
          message: Wording.stopped + " The command may have gone through anyway.", unanswered: true)
    )
    #expect(await rig.error(queued) == BluetoothError(message: Wording.stopped))
    #expect(await rig.error(scan) == BluetoothError(message: Wording.stopped))
    #expect(rig.radio.calls.contains(.cancel(air)))
    #expect(rig.radio.calls.contains(.cancel(hvac)))
    #expect(rig.radio.writes(to: air) == [Data("a-|".utf8)])

    // The next request opens a fresh link.
    rig.radio.holding = []
    await rig.settle()
    try await rig.central.readStatus(air)
    #expect(rig.radio.count(.connect(air)) == 2)
  }
}
