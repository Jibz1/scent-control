// Ported from the `mask_passwords` tests in `apps/desktop/src/state.rs`.

import Testing

@testable import ScentProtocol

@Suite struct MaskingTests {
  @Test func maskPasswordsReplacesEveryPasswordValue() {
    #expect(
      maskPasswords(
        #"{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"hunter2!"}}"#)
        == #"{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"***"}}"#)
    // Spaces, any case, escapes inside the value, a value that is not a string.
    #expect(maskPasswords(#"{"Password" : "a\"b\\"}"#) == #"{"Password" : "***"}"#)
    #expect(maskPasswords(#"{"password":12345,"x":1}"#) == #"{"password":"***","x":1}"#)
    #expect(maskPasswords(#"{"password":null}"#) == #"{"password":"***"}"#)
    // A longer key name that ends or starts with "password".
    #expect(
      maskPasswords(#"{"wifi_password":"x","mqttPassword":"y","password_hint":"z"}"#)
        == #"{"wifi_password":"***","mqttPassword":"***","password_hint":"***"}"#)
    // Every occurrence, nested ones included.
    #expect(
      maskPasswords(#"{"a":{"password":"1"},"b":[{"password":"2"}],"password":"3"}"#)
        == #"{"a":{"password":"***"},"b":[{"password":"***"}],"password":"***"}"#)
  }

  @Test func maskPasswordsWorksOnTextThatIsNotValidJSON() {
    #expect(
      maskPasswords(#"{"data":enable:0,"password":"p@ss"}"#)
        == #"{"data":enable:0,"password":"***"}"#)
    #expect(maskPasswords(#"{"password":hunter2,"x":1}"#) == #"{"password":"***","x":1}"#)
    // Cut off in the middle of the value: what is there is hidden.
    #expect(maskPasswords(#"{"password":"abc"#) == #"{"password":"***""#)
    // No quoted key, no match.
    #expect(maskPasswords(#"password:"abc""#) == #"password:"abc""#)
    // JSON inside a JSON string, as the quotes then arrive escaped.
    #expect(
      maskPasswords(#"{"msg":"{\"password\":\"hunter2\",\"x\":1}"}"#)
        == #"{"msg":"{\"password\":\"***\",\"x\":1}"}"#)
    #expect(
      maskPasswords(#"{"msg":"{\"password\":\"a\\\"b\",\"x\":1}"}"#)
        == #"{"msg":"{\"password\":\"***\",\"x\":1}"}"#)
    // Non-ASCII text around the value is kept whole.
    #expect(
      utf8(maskPasswords(#"{"note":"héllo","password":"pässwörd","z":"ü"}"#))
        == utf8(#"{"note":"héllo","password":"***","z":"ü"}"#))
  }

  @Test func maskPasswordsLeavesOtherTextAlone() {
    for text in [
      "",
      "S:1,L:3,C:2,Bat:200",
      #"{"action":"PING"}"#,
      #"{"message":"password reset"}"#,
      #"{"message":"password"}"#,
      #"{"hint":"your password: unknown"}"#,
      "password",
    ] {
      #expect(maskPasswords(text) == text)
    }
    // Masking twice is the same as masking once (summaries arrive already redacted).
    let once = maskPasswords(#"{"ssid":"x","password":"hunter2!"}"#)
    #expect(maskPasswords(once) == once)
  }

  /// From `log_masks_passwords_in_echoed_commands`: the app hears its own `SAVE_MQTT` and
  /// `WIFI_CONNECT` come back from the broker, passwords and all.
  @Test func echoedCommandsAreMasked() {
    let wifi = Command.wifiConnect(ssid: "ExampleWiFi", password: Secret("hunter2!"))
    let save = Command.saveMqtt(
      broker: "wss://broker.example.test/mqtt", username: "example-user",
      password: Secret("hunter2!"))
    for command in [wifi, save] {
      let body = text(command.mqttBody())
      #expect(body.contains("hunter2!"), "the device does need it")
      let masked = maskPasswords(body)
      #expect(!masked.contains("hunter2!"))
      #expect(masked == command.redacted(via: .wifi))
      #expect(maskPasswords(text(command.bleFrame())) == command.redacted(via: .bluetooth))
    }
    // Not valid JSON, but the password must still not get through.
    let broken = #"{"action":"WIFI_CONNECT","payload":{"data":oops,"password":"hunter2!"}}"#
    #expect(!maskPasswords(broken).contains("hunter2!"))
    #expect(maskPasswords(broken).contains(#""password":"***""#))
  }
}
