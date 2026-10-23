// What the client is given and what it says back: the options, the events and the errors.
import Foundation

/// Where the broker is, who we are on it, and what to listen to.
public struct MQTTOptions: Sendable {
  /// `wss://host[:port]/path`. `ws://` is taken too, for a broker on this machine in tests.
  public var url: URL
  public var username: String
  /// Never printed: it is left out of `description`, `debugDescription`, `dump` and every
  /// error and event.
  public var password: String
  /// Who this connection is on the broker. A second connection with the same ID takes
  /// the first one's place, so each copy of the app makes its own.
  public var clientID: String
  /// Topic filters, subscribed at QoS 1 on every CONNACK.
  public var subscriptions: [String]
  /// How often the client pings the broker. A ping that has had no answer by the next one
  /// means the connection is dead. Zero turns pings off.
  public var keepAlive: Duration
  /// How long the client waits after a failure before it tries the broker again.
  public var retryAfter: Duration

  /// Options with a random client ID, a 30 s keep-alive and a 5 s wait between tries.
  public init(url: URL, username: String, password: String, subscriptions: [String]) {
    self.url = url
    self.username = username
    self.password = password
    self.subscriptions = subscriptions
    clientID = MQTTOptions.randomClientID()
    keepAlive = .seconds(30)
    retryAfter = .seconds(5)
  }

  /// `scent-ios-` and 8 lowercase hex digits from the system's random source.
  public static func randomClientID() -> String {
    clientID(UInt32.random(in: .min ... .max))
  }

  static func clientID(_ number: UInt32) -> String {
    let hex = String(number, radix: 16)
    return "scent-ios-" + String(repeating: "0", count: 8 - hex.count) + hex
  }
}

extension MQTTOptions: CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable {
  /// The options in words, with the password hidden.
  public var description: String {
    "MQTTOptions(url: \(url.absoluteString), username: \"\(username)\", password: <hidden>, "
      + "clientID: \"\(clientID)\", subscriptions: \(subscriptions), keepAlive: \(keepAlive), "
      + "retryAfter: \(retryAfter))"
  }

  public var debugDescription: String { description }

  /// What `dump` and the debugger show: every field, with the password hidden.
  public var customMirror: Mirror {
    Mirror(
      self,
      children: [
        "url": url, "username": username, "password": "<hidden>", "clientID": clientID,
        "subscriptions": subscriptions, "keepAlive": keepAlive, "retryAfter": retryAfter,
      ],
      displayStyle: .struct)
  }
}

/// What the client reports, in the order it happens.
public enum MQTTEvent: Sendable, Equatable {
  /// The broker accepted the login. Sent again after every reconnect.
  case connected
  /// The broker refused the username or password, or says the user is not authorized.
  /// The client stops trying until `start()` is called again.
  case loginRefused(String)
  /// The connection failed or dropped, and why, in words. The client tries again after
  /// `retryAfter`. One exception: options that can't work (an address that isn't
  /// `wss://`, a topic filter MQTT can't take) are reported this way by `start()`, and
  /// nothing is tried.
  case disconnected(String)
  /// A message from the broker. `retained` is set when the broker kept it from before and
  /// is handing it over because of a new subscription.
  case message(topic: String, payload: Data, retained: Bool)
}

/// Why a message wasn't queued.
public struct MQTTError: Error, Sendable, Equatable, LocalizedError {
  public let message: String

  public init(message: String) {
    self.message = message
  }

  public var errorDescription: String? { message }

  /// The broker isn't connected, so nothing is queued to go out later, when it would no
  /// longer be expected.
  public static let notConnected = MQTTError(message: "Not connected to the broker.")
  /// Sixteen messages are already waiting to go out.
  public static let tooBusy = MQTTError(
    message: "The broker connection is too busy to take this message.")
  /// The packet would be over 256 KiB, the most the connection writes or reads.
  public static let tooLarge = MQTTError(
    message: "The message is too large to send: the limit is 256 KiB.")
}
