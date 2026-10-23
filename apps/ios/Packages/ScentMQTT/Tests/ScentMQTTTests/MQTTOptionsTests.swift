// The options' defaults, the client IDs, and the password staying out of sight.
import Foundation
import Testing

@testable import ScentMQTT

@Suite("Options")
struct MQTTOptionsTests {
  @Test("default to a 30 s keep-alive, a 5 s wait between tries and a random client ID")
  func defaults() {
    let options = MQTTOptions.example
    #expect(options.keepAlive == .seconds(30))
    #expect(options.retryAfter == .seconds(5))
    #expect(options.clientID.hasPrefix("scent-ios-"))
    #expect(options.subscriptions == ["devices/#", "app/v1/#"])
  }

  @Test("client IDs are scent-ios- and 8 lowercase hex digits")
  func clientIDs() {
    #expect(MQTTOptions.clientID(0xAB12_CD34) == "scent-ios-ab12cd34")
    #expect(MQTTOptions.clientID(0x1) == "scent-ios-00000001")
    #expect(MQTTOptions.clientID(.max) == "scent-ios-ffffffff")
    let ids = (0..<20).map { _ in MQTTOptions.randomClientID() }
    for id in ids {
      let hex = id.dropFirst("scent-ios-".count)
      #expect(id.hasPrefix("scent-ios-"))
      #expect(hex.count == 8)
      #expect(hex.allSatisfy { "0123456789abcdef".contains($0) }, "\(id)")
    }
    #expect(Set(ids).count > 1)
  }

  @Test("never show the password")
  func hidePassword() {
    var options = MQTTOptions.example
    options.password = "not-a-real-secret"
    var dumped = ""
    dump(options, to: &dumped)
    for text in [
      options.description, options.debugDescription, String(describing: options),
      String(reflecting: options), "\(options)", dumped,
    ] {
      #expect(!text.contains("not-a-real-secret"), "\(text)")
      #expect(text.contains("<hidden>"), "\(text)")
      #expect(text.contains("example-user"), "\(text)")
    }
  }

  @Test("errors say their message")
  func errors() {
    #expect(MQTTError.notConnected.message == "Not connected to the broker.")
    #expect(MQTTError.notConnected.localizedDescription == "Not connected to the broker.")
    #expect(
      MQTTError.tooBusy.localizedDescription
        == "The broker connection is too busy to take this message.")
    #expect(MQTTError(message: "x") == MQTTError(message: "x"))
  }

  @Test("topic filters follow MQTT's wildcard rules (4.7)")
  func filters() {
    for good in ["a", "a/b", "#", "a/#", "+", "+/b", "a/+/c", "/", "a//b", "+/+/#"] {
      #expect(MQTTClient.isFilter(good), "\(good)")
    }
    for bad in ["", "a#", "#/a", "a/#/b", "a+", "+a/b", "a/b+", "a\u{0}b"] {
      #expect(!MQTTClient.isFilter(bad), "\(bad)")
    }
    for good in ["a", "a/b", "/", "devices/TESTDEVICE01"] {
      #expect(MQTTClient.isPublishable(good), "\(good)")
    }
    for bad in ["", "a/+", "#", "a/#", "a\u{0}"] {
      #expect(!MQTTClient.isPublishable(bad), "\(bad)")
    }
  }
}
