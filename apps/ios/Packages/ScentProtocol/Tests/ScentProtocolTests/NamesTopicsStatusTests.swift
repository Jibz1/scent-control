// Ported from the tests in `packages/scent-core/src/names.rs`, `topics.rs` and
// `status.rs`.

import Testing

@testable import ScentProtocol

@Suite struct NameTests {
  @Test func recognisesScentimentNames() {
    #expect(isScentimentName("Scentiment-D B 1A2B"))
    #expect(isScentimentName("SCENTIMENT-H W"))
    #expect(!isScentimentName("Scent K.A5.WIFI"))
    #expect(!isScentimentName(""))
  }

  @Test func readsModelAndVariantCodes() {
    #expect(
      parseName("Scentiment-D B 1A2B")
        == NameInfo(modelCode: "D", model: "Diffuser Air 2", variantCode: "B", variant: "Black"))
    #expect(
      parseName("Scentiment-H W")
        == NameInfo(modelCode: "H", model: "Hvac", variantCode: "W", variant: "White"))
    #expect(parseName("Scentiment-Z Q") == NameInfo(modelCode: "Z", variantCode: "Q"))
    #expect(parseName("Scentiment") == NameInfo())
    #expect(parseName("Scentiment-D") == NameInfo(modelCode: "D", model: "Diffuser Air 2"))
  }

  @Test func knowsEveryModelAndVariantCode() {
    let models = [
      ("D", "Diffuser Air 2"), ("H", "Hvac"), ("P", "Pro"), ("M", "Mini"), ("T", "Mini 2 LE"),
      ("W", "The Plug-In"), ("R", "Diffuser_4"),
    ]
    for (code, name) in models {
      #expect(parseName("Scentiment-\(code) B").model == name)
    }
    let variants = [
      ("B", "Black"), ("C", "Graphite Gold"), ("G", "Grey"), ("W", "White"), ("S", "Silver"),
      ("R", "Red"),
    ]
    for (code, name) in variants {
      #expect(parseName("Scentiment-D \(code)").variant == name)
    }
  }

  @Test func deviceIDIsTheLastTokenWhenItIsTwelveHexDigits() {
    #expect(deviceIDFromName("Scentiment-D B n A1B2C3D4E5F6") == "A1B2C3D4E5F6")
    #expect(deviceIDFromName("Scentiment-H B n 0a1b2c3d4e5f") == "0A1B2C3D4E5F")
    for name in [
      "Scentiment-D B", "", "   ", "Scentiment-D B n A1B2C3D4E5F", "Scentiment-D B n A1B2C3D4E5F67",
      "Scentiment-D B n G1B2C3D4E5F6",
    ] {
      #expect(deviceIDFromName(name) == nil, "\(name.debugDescription)")
    }
  }

  /// Swift only: tokens split on Unicode spaces, as Rust's `split_whitespace` does.
  @Test func tokensSplitOnAnyUnicodeSpace() {
    #expect(
      parseName("Scentiment-D\u{A0}B\u{2003}x")
        == NameInfo(modelCode: "D", model: "Diffuser Air 2", variantCode: "B", variant: "Black"))
    #expect(deviceIDFromName("Scentiment-D B\u{3000}0a1b2c3d4e5f\u{2028}") == "0A1B2C3D4E5F")
  }
}

@Suite struct TopicTests {
  @Test func topicsMatchTheAppsConstants() {
    #expect(Topics.devicePrefix == "scentiment-mqtt-prod/device/")
    #expect(Topics.subscribeAll == "scentiment-mqtt-prod/#")
    #expect(Topics.device("TESTDEVICE01") == "scentiment-mqtt-prod/device/TESTDEVICE01")
  }

  @Test func ourOwnTopicsAreUnderOnePrefixApartFromScentiments() {
    #expect(Topics.appPrefix == "scent-control/v1/")
    #expect(Topics.subscribeApp == "scent-control/v1/#")
    #expect(Topics.schedulePrefix == "scent-control/v1/schedule/")
    for ours in [Topics.subscribeApp, Topics.schedulePrefix] {
      #expect(ours.hasPrefix(Topics.appPrefix), "\(ours)")
    }
    // The diffusers subscribe under Scentiment's prefix: nothing of ours is there.
    #expect(!Topics.appPrefix.hasPrefix("scentiment-mqtt-prod/"))
    #expect(!"scentiment-mqtt-prod/".hasPrefix(Topics.appPrefix))
  }

  @Test func aScheduleTopicIsBuiltOnlyForAValidDeviceID() {
    #expect(Topics.schedule("TESTDEVICE01") == "scent-control/v1/schedule/TESTDEVICE01")
    for bad in [
      "", "#", "+", "TESTDEVICE01/../x", "TESTDEVICE01/#", "testdevice01",
      "../../scentiment-mqtt-prod/device/TESTDEVICE01",
    ] {
      #expect(Topics.schedule(bad) == nil, "\(bad.debugDescription)")
    }
    // Whatever it is given, it never builds a topic a diffuser listens on.
    for id in ["TESTDEVICE01", "TESTDEVICE02", "scentiment-mqtt-prod/device/X"] {
      if let topic = Topics.schedule(id) {
        #expect(topic.hasPrefix(Topics.schedulePrefix), "\(topic)")
        #expect(!topic.hasPrefix("scentiment-mqtt-prod/"), "\(topic)")
        #expect(!topic.contains("+") && !topic.contains("#"), "\(topic)")
      }
    }
  }

  @Test func stateTopicsAreBuiltOnlyForValidIDsAndReadBackExactly() {
    #expect(Topics.statePrefix == "scent-control/v1/state/")
    #expect(Topics.listenerTopic == "scent-control/v1/listener")
    for ours in [Topics.statePrefix, Topics.listenerTopic] {
      #expect(ours.hasPrefix(Topics.appPrefix), "\(ours)")
    }
    #expect(Topics.state("TESTDEVICE01") == "scent-control/v1/state/TESTDEVICE01")
    #expect(Topics.state("TESTDEVIC/#") == nil)
    #expect(Topics.stateDevice("scent-control/v1/state/TESTDEVICE01") == "TESTDEVICE01")
    for other in [
      "scent-control/v1/state/TESTDEVICE01/x", "scent-control/v1/state/",
      "scent-control/v1/schedule/TESTDEVICE01",
    ] {
      #expect(Topics.stateDevice(other) == nil, "\(other.debugDescription)")
    }
  }

  @Test func theDeviceOfAScheduleTopicIsReadBackOnlyWhenValid() {
    #expect(Topics.scheduleDevice("scent-control/v1/schedule/TESTDEVICE01") == "TESTDEVICE01")
    for other in [
      "scent-control/v1/schedule/",
      "scent-control/v1/schedule/testdevice01",
      "scent-control/v1/schedule/TESTDEVICE01/x",
      "scent-control/v1/other/TESTDEVICE01",
      "scent-control/v2/schedule/TESTDEVICE01",
      "scentiment-mqtt-prod/device/TESTDEVICE01",
      "",
    ] {
      #expect(Topics.scheduleDevice(other) == nil, "\(other.debugDescription)")
    }
  }

  @Test func deviceIDsAreTwelveUpperCaseLettersOrDigits() {
    for good in ["TESTDEVICE01", "A1B2C3D4E5F6", "000000000000"] {
      #expect(Topics.validDeviceID(good), "\(good)")
    }
    for bad in [
      "", "#", "abc", "TESTDEVICE01/../x", "TESTDEVICE0", "TESTDEVICE011", "testdevice01",
      "TESTDEVICE01\n", "TESTDEVICE0é", "TESTDEVICE+1",
      // Swift only: canonically equal to "K..." for Swift's `==`, but not the same bytes.
      "\u{212A}ESTDEVICE01",
    ] {
      #expect(!Topics.validDeviceID(bad), "\(bad.debugDescription)")
    }
  }
}

@Suite struct OnlineTests {
  private let now = 1_792_510_200

  // Ported from tools/tests/test_control_server.py (online_status_from_ping_and_ack).
  @Test func onlineStatusFromPingAndAnswer() {
    // never pinged, never heard
    #expect(onlineStatus(pingAt: nil, heardAt: nil, now: now) == .unknown)
    // pinged 10 s ago, answered 1 s later
    #expect(onlineStatus(pingAt: now - 10, heardAt: now - 9, now: now) == .online)
    // pinged 10 s ago; the last thing heard is older than the ping
    #expect(onlineStatus(pingAt: now - 10, heardAt: now - 120, now: now) == .waiting)
    // pinged 30 s ago, still nothing newer
    #expect(onlineStatus(pingAt: now - 30, heardAt: now - 610, now: now) == .offline)
  }

  @Test func pingedButNeverHeardWaitsThenGoesOffline() {
    #expect(onlineStatus(pingAt: now - 1, heardAt: nil, now: now) == .waiting)
    #expect(onlineStatus(pingAt: now - 15, heardAt: nil, now: now) == .waiting)
    #expect(onlineStatus(pingAt: now - 16, heardAt: nil, now: now) == .offline)
  }

  @Test func heardRecentlyCountsAsOnlineEvenWithoutAPing() {
    #expect(onlineStatus(pingAt: nil, heardAt: now - 75, now: now) == .online)
    #expect(onlineStatus(pingAt: nil, heardAt: now - 76, now: now) == .unknown)
  }

  @Test func anOldAnswerAfterTheLastPingIsUnknownNotOffline() {
    // Heard after the ping, but long ago: we have no fresh evidence either way.
    #expect(onlineStatus(pingAt: now - 600, heardAt: now - 590, now: now) == .unknown)
  }

  @Test func anAnswerInTheSameSecondAsThePingCounts() {
    #expect(onlineStatus(pingAt: now - 5, heardAt: now - 5, now: now) == .online)
  }

  @Test func aReadingIsStaleOnlyAfter300Seconds() {
    #expect(!isStale(at: now, now: now))
    #expect(!isStale(at: now - 300, now: now))
    #expect(isStale(at: now - 301, now: now))
    // A timestamp from the future (clock step) is not stale.
    #expect(!isStale(at: now + 60, now: now))
  }

  @Test func extremeTimestampsDoNotOverflow() {
    #expect(isStale(at: Int.min, now: Int.max))
    #expect(!isStale(at: Int.max, now: Int.min))
    #expect(onlineStatus(pingAt: Int.min, heardAt: Int.min, now: Int.max) == .unknown)
    #expect(onlineStatus(pingAt: Int.max, heardAt: nil, now: Int.min) == .waiting)
  }

  @Test func theTimingsAreTheAgreedOnes() {
    #expect(pingEverySeconds == 30)
    #expect(answerWithinSeconds == 15)
    #expect(onlineForSeconds == 75)
    #expect(staleAfterSeconds == 300)
  }
}
