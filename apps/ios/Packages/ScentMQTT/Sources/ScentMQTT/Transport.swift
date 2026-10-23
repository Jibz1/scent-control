// What the client needs from the network and from time, kept behind small protocols so
// that the tests can stand in for both.
import Foundation

/// An open connection that carries MQTT packets as binary messages, both ways.
protocol MQTTTransport: Sendable {
  /// Sends one message. Messages go out in the order of the calls that sent them.
  func send(_ data: Data) async throws
  /// The next message from the broker. Throws once the connection has closed.
  func receive() async throws -> Data
  /// Closes the connection. Pending and later sends and receives throw. Safe to call
  /// more than once.
  func close()
}

/// Opens connections to the broker.
protocol MQTTConnector: Sendable {
  /// Opens a connection, or throws why it couldn't. Stops and throws when the task is
  /// cancelled.
  func connect(to url: URL) async throws -> any MQTTTransport
}

/// Waits. Throws when the task is cancelled.
protocol MQTTSleeper: Sendable {
  func sleep(for duration: Duration) async throws
}

/// Waits on the system's clock.
struct TaskSleeper: MQTTSleeper {
  func sleep(for duration: Duration) async throws {
    try await Task.sleep(for: duration)
  }
}

/// A transport failure with words for the person reading it.
struct TransportError: Error, Sendable, LocalizedError {
  let message: String

  var errorDescription: String? { message }
}
