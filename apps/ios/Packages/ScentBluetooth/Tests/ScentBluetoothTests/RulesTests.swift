import CoreBluetooth
import Foundation
import Testing

@testable import ScentBluetooth

// The decisions that need no Bluetooth: the GATT layout, chunking, the scan filter, the
// link timer and the wording. Ported from the tests in `apps/desktop/src/ble.rs`.

@Suite struct GattTests {
  @Test func theGattIdsAreTheScentimentOnes() {
    #expect(Gatt.service == "00000180-0000-1000-8000-00805f9b34fb")
    #expect(GattCharacteristic.write.uuid == "0000dead-0000-1000-8000-00805f9b34fb")
    #expect(GattCharacteristic.status.uuid == "0000fef3-0000-1000-8000-00805f9b34fb")
    #expect(GattCharacteristic.info.uuid == "0000fef4-0000-1000-8000-00805f9b34fb")
    #expect(Gatt.short(Gatt.service) == "0180")
    #expect(GattCharacteristic.allCases.map(\.label) == ["0xDEAD", "0xFEF3", "0xFEF4"])
  }

  @Test func framesAreWrittenInOrderIn177ByteChunks() {
    let frame = Data((0..<400).map { UInt8($0 % 251) })
    let parts = Gatt.chunks(frame)
    #expect(parts.map(\.count) == [177, 177, 46])
    #expect(parts.reduce(Data(), +) == frame)

    // A short frame goes out in one write, and one of exactly 177 bytes too.
    #expect(Gatt.chunks(Data("{\"action\":\"PING\"}-|".utf8)).count == 1)
    #expect(Gatt.chunks(Data(count: 177)).map(\.count) == [177])
    #expect(Gatt.chunks(Data(count: 178)).map(\.count) == [177, 1])
    #expect(Gatt.chunks(Data()).isEmpty)
  }

  @Test func chunksOfASliceStartAtItsOwnStart() {
    let whole = Data((0..<300).map { UInt8($0 % 256) })
    let slice = whole[50..<300]
    let parts = Gatt.chunks(slice)
    #expect(parts.map(\.count) == [177, 73])
    #expect(parts.reduce(Data(), +) == Data(slice))
    #expect(parts.allSatisfy { $0.startIndex == 0 })
  }

  @Test func theFirstMissingCharacteristicIsNamedInTheMacsOrder() {
    #expect(Gatt.firstMissing(from: []) == .write)
    #expect(Gatt.firstMissing(from: [.write, .status]) == .info)
    #expect(Gatt.firstMissing(from: [.write, .info]) == .status)
    #expect(Gatt.firstMissing(from: Set(GattCharacteristic.allCases)) == nil)
  }

  @Test func coreBluetoothUuidsAreRecognizedInEitherForm() {
    #expect(CoreBluetoothRadio.gatt(CBUUID(string: "FEF3")) == .status)
    #expect(
      CoreBluetoothRadio.gatt(CBUUID(string: "0000fef4-0000-1000-8000-00805f9b34fb")) == .info)
    #expect(CoreBluetoothRadio.gatt(CBUUID(string: "dead")) == .write)
    #expect(CoreBluetoothRadio.gatt(CBUUID(string: "2A00")) == nil)
  }
}

@Suite struct ScanFilterTests {
  let id = UUID(uuid: (1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))

  @Test func onlyScentimentNamesBecomeScanHits() {
    let hit = ScanFilter.hit(id, Sighting(name: "Scentiment-D B n 0A1B2C3D4E5F", rssi: -61))
    #expect(hit == ScanHit(id: id, name: "Scentiment-D B n 0A1B2C3D4E5F", rssi: -61))
    // Any case will do.
    #expect(
      ScanFilter.hit(id, Sighting(name: "scentiment-h w", rssi: nil))?.name == "scentiment-h w")
    #expect(ScanFilter.hit(id, Sighting(name: "SCENTIMENT", rssi: nil)) != nil)
    #expect(ScanFilter.hit(id, Sighting(name: "JBL Flip 6", rssi: -40)) == nil)
    #expect(ScanFilter.hit(id, Sighting(name: nil, rssi: -40)) == nil)
  }

  @Test func aDiffuserBelowMinus90DbmIsLeftOut() {
    let name = "Scentiment-D B n 0A1B2C3D4E5F"
    #expect(ScanFilter.hit(id, Sighting(name: name, rssi: -90)) != nil)
    #expect(ScanFilter.hit(id, Sighting(name: name, rssi: -91)) == nil)
    // With no signal strength it is kept.
    #expect(ScanFilter.hit(id, Sighting(name: name, rssi: nil)) != nil)
  }

  @Test func bluetoothSays127WhenItHasNoSignalStrength() {
    #expect(ScanFilter.rssi(reported: 127) == nil)
    #expect(ScanFilter.rssi(reported: -71) == -71)
  }

  @Test func hitsAreSortedByNameThenId() {
    let a = UUID(uuid: (2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let b = UUID(uuid: (1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let c = UUID(uuid: (3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let other = UUID(uuid: (4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    let hits = ScanFilter.hits([
      a: Sighting(name: "Scentiment-H B n 1", rssi: -60),
      b: Sighting(name: "Scentiment-D B n 2", rssi: -70),
      c: Sighting(name: "Scentiment-D B n 2", rssi: -50),
      other: Sighting(name: "Speaker", rssi: -30),
    ])
    #expect(hits.map(\.id) == [b, c, a])
  }

  @Test func aLaterSightingKeepsWhatItLacks() {
    var sighting = Sighting(name: nil, rssi: -80)
    sighting.update(name: "Scentiment-D B n 1", rssi: nil)
    #expect(sighting == Sighting(name: "Scentiment-D B n 1", rssi: -80))
    sighting.update(name: nil, rssi: -60)
    #expect(sighting == Sighting(name: "Scentiment-D B n 1", rssi: -60))
  }
}

@Suite struct LinkTimerTests {
  let t0 = ContinuousClock.now

  @Test func anIdleLinkIsDroppedAfter10Seconds() {
    var timer = LinkTimer(openedAt: t0)
    timer.touch(t0)
    #expect(!timer.shouldDisconnect(t0 + .seconds(9)))
    #expect(timer.shouldDisconnect(t0 + .seconds(10)))

    // Touching again at 9 s defers the disconnect by another 10 s.
    timer.touch(t0 + .seconds(9))
    #expect(!timer.shouldDisconnect(t0 + .seconds(10)))
    #expect(!timer.shouldDisconnect(t0 + .seconds(18)))
    #expect(timer.shouldDisconnect(t0 + .seconds(19)))
  }

  @Test func aSendKeepsTheLinkOpenForTheListenWindow() {
    var timer = LinkTimer(openedAt: t0)
    timer.sent(t0)
    #expect(!timer.shouldDisconnect(t0 + LinkTimer.listenAfterSend))
    #expect(timer.deadline == t0 + max(LinkTimer.idleAfter, LinkTimer.listenAfterSend))

    // Neither an older touch nor an older send can bring the deadline forward.
    let later = t0 + .seconds(9)
    timer.touch(later)
    timer.sent(t0)
    #expect(timer.deadline == later + LinkTimer.idleAfter)
  }

  @Test func theLimitsAreTheMacs() {
    #expect(Limits.connect == .seconds(10))
    #expect(Limits.io == .seconds(10))
    #expect(Limits.disconnect == .seconds(3))
    #expect(Limits.settle == .seconds(3))
    #expect(Limits.queue == 16)
    #expect(LinkTimer.idleAfter == .seconds(10))
    #expect(LinkTimer.listenAfterSend == .seconds(6))
    #expect(Gatt.chunkSize == 177)
  }
}

@Suite struct WordingTests {
  @Test func onlyErrorsABusyDiffuserCanCauseCarryTheHint() {
    let hinted = Wording.opening(.timedOut, activity: "connecting")
    #expect(hinted?.message.hasPrefix("Timed out while connecting. ") == true)
    #expect(hinted?.message.contains("one Bluetooth connection at a time") == true)
    #expect(hinted?.message.contains("Scentiment app") == true)
    #expect(hinted?.unanswered == false)

    // "Close the Scentiment app" is no help when the device is not a diffuser.
    let plain = Wording.notADiffuser(.info)
    #expect(
      plain.message
        == "The device has no Bluetooth characteristic 0xFEF4, so it is not a diffuser we can talk to."
    )
  }

  @Test func stepsThatFailOrTimeOutSayWhatTheyWereDoing() {
    #expect(Wording.opening(.value(.done(nil)), activity: "connecting") == nil)
    let failed = Wording.opening(
      .value(.failed("Peer removed pairing information.")), activity: "connecting")
    #expect(
      failed?.message.hasPrefix(
        "Bluetooth failed while connecting: Peer removed pairing information. ")
        == true)
    let dropped = Wording.opening(.value(.dropped(nil)), activity: "reading the services")
    #expect(
      dropped?.message.hasPrefix(
        "Bluetooth failed while reading the services: the Bluetooth link was lost. ")
        == true)
    // Bluetooth being off says so, with no hint about other apps.
    let off = Wording.notReady(.poweredOff)!
    #expect(
      Wording.opening(.value(.stopped(off)), activity: "connecting") == BluetoothError(message: off)
    )
    #expect(
      Wording.opening(.cancelled, activity: "connecting")
        == BluetoothError(message: Wording.cancelled))
  }

  @Test func aCommandsLastWriteThatGoesUnansweredIsUnanswered() {
    let timedOut = Wording.exchange(
      .timedOut, failure: Wording.writeFailed, mayHaveGoneThrough: true)
    #expect(
      timedOut
        == BluetoothError(
          message:
            "The diffuser didn't answer within 10 s. The command may have gone through anyway.",
          unanswered: true))
    let dropped = Wording.exchange(
      .value(.dropped("The specified device has disconnected from us.")),
      failure: Wording.writeFailed,
      mayHaveGoneThrough: true)
    #expect(dropped?.unanswered == true)
    #expect(
      dropped?.message
        == "Couldn't write to the diffuser: The specified device has disconnected from us. "
        + "The command may have gone through anyway.")
    #expect(
      Wording.exchange(.cancelled, failure: Wording.writeFailed, mayHaveGoneThrough: true)?
        .unanswered
        == true)
    #expect(
      Wording.exchange(
        .value(.stopped(Wording.stopped)), failure: Wording.writeFailed, mayHaveGoneThrough: true)?
        .unanswered == true)
  }

  @Test func aRefusedWriteOrAnEarlierPartIsNotUnanswered() {
    // Bluetooth said the write failed, so it didn't go through.
    let refused = Wording.exchange(
      .value(.failed("Writing is not permitted.")), failure: Wording.writeFailed,
      mayHaveGoneThrough: true)
    #expect(
      refused
        == BluetoothError(message: "Couldn't write to the diffuser: Writing is not permitted."))
    // An earlier part carries no terminator, so the diffuser can't have acted on it.
    let early = Wording.exchange(.timedOut, failure: Wording.writeFailed, mayHaveGoneThrough: false)
    #expect(early == BluetoothError(message: "The diffuser didn't answer within 10 s."))
    #expect(
      Wording.exchange(.value(.done(nil)), failure: Wording.writeFailed, mayHaveGoneThrough: true)
        == nil)
  }

  @Test func readsSayWhatTheyRead() {
    let info = Wording.exchange(
      .value(.failed("Reading is not permitted.")), failure: Wording.readInfoFailed,
      mayHaveGoneThrough: false)
    #expect(info?.message == "Couldn't read the diffuser's info: Reading is not permitted.")
    let status = Wording.exchange(
      .timedOut, failure: Wording.readStatusFailed, mayHaveGoneThrough: false)
    #expect(status == BluetoothError(message: "The diffuser didn't answer within 10 s."))
  }

  @Test func whatEachStateMeans() {
    #expect(Wording.notReady(.poweredOn) == nil)
    #expect(
      Wording.notReady(.poweredOff)
        == "Bluetooth is turned off on this iPhone. Turn it on in Control Center or Settings, then try again."
    )
    let denied = Wording.notReady(.unauthorized)!
    #expect(denied.contains("Settings, Privacy & Security, Bluetooth"))
    #expect(Wording.notReady(.unsupported) == "Bluetooth isn't available on this device.")
    // Unknown after the wait: the permission prompt still open, resetting, or not known.
    for state in [BluetoothAvailability.unknown, .resetting] {
      #expect(
        Wording.notReady(state)
          == "Bluetooth isn't ready on this iPhone. If iOS asked, allow Scent Control to use Bluetooth "
          + "(Settings, Privacy & Security, Bluetooth).")
    }
  }

  @Test func theErrorIsTheMessage() {
    let error = BluetoothError(message: "Timed out while connecting.")
    #expect(error.localizedDescription == "Timed out while connecting.")
    #expect(error.unanswered == false)
  }

  @Test func coreBluetoothStatesMapOneToOne() {
    #expect(CoreBluetoothRadio.availability(.unknown) == .unknown)
    #expect(CoreBluetoothRadio.availability(.resetting) == .resetting)
    #expect(CoreBluetoothRadio.availability(.unsupported) == .unsupported)
    #expect(CoreBluetoothRadio.availability(.unauthorized) == .unauthorized)
    #expect(CoreBluetoothRadio.availability(.poweredOff) == .poweredOff)
    #expect(CoreBluetoothRadio.availability(.poweredOn) == .poweredOn)
  }

  @Test func errorsThatMeanTheLinkWentDownAreMarkedSo() {
    let lost = NSError(domain: CBErrorDomain, code: CBError.Code.peripheralDisconnected.rawValue)
    #expect(CoreBluetoothRadio.failure(lost)?.linkLost == true)
    let timeout = NSError(domain: CBErrorDomain, code: CBError.Code.connectionTimeout.rawValue)
    #expect(CoreBluetoothRadio.failure(timeout)?.linkLost == true)
    let refused = NSError(
      domain: CBATTErrorDomain, code: CBATTError.Code.writeNotPermitted.rawValue)
    #expect(CoreBluetoothRadio.failure(refused)?.linkLost == false)
    #expect(CoreBluetoothRadio.failure(nil) == nil)
  }
}
