// MQTT over a WebSocket with URLSession: the `mqtt` subprotocol, one binary message per
// packet, and the system's TLS for `wss://`.
import Foundation
import Synchronization

/// Opens WebSocket connections that speak MQTT.
struct WebSocketConnector: MQTTConnector {
  func connect(to url: URL) async throws -> any MQTTTransport {
    let transport = WebSocketTransport(url: url)
    do {
      try await transport.open()
    } catch {
      transport.close()
      throw error
    }
    return transport
  }
}

/// One WebSocket connection, on its own URLSession so that closing it lets everything go.
final class WebSocketTransport: MQTTTransport {
  private let session: URLSession
  private let task: URLSessionWebSocketTask
  private let delegate: WebSocketDelegate

  init(url: URL) {
    let delegate = WebSocketDelegate()
    // Ephemeral: no cookies or cache to keep.
    let configuration = URLSessionConfiguration.ephemeral
    configuration.waitsForConnectivity = false
    session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
    task = session.webSocketTask(with: url, protocols: ["mqtt"])
    self.delegate = delegate
  }

  /// Opens the WebSocket and returns once the broker has agreed to speak MQTT on it.
  func open() async throws {
    let task = task
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        delegate.waitForOpen(continuation)
        task.resume()
      }
    } onCancel: {
      task.cancel()
    }
  }

  func send(_ data: Data) async throws {
    do {
      try await task.send(.data(data))
    } catch {
      throw closedError(error)
    }
  }

  func receive() async throws -> Data {
    let message: URLSessionWebSocketTask.Message
    do {
      message = try await task.receive()
    } catch {
      throw closedError(error)
    }
    switch message {
    case .data(let data):
      return data
    case .string:
      throw TransportError(message: "The broker sent text where MQTT needs binary.")
    @unknown default:
      throw TransportError(message: "The broker sent a WebSocket message of an unknown kind.")
    }
  }

  func close() {
    task.cancel(with: .normalClosure, reason: nil)
    session.finishTasksAndInvalidate()
  }

  /// The words for a send or receive that failed: the broker closing the WebSocket says
  /// so, rather than the socket error that follows it.
  private func closedError(_ error: any Error) -> any Error {
    task.closeCode == .invalid
      ? error : TransportError(message: "The broker closed the connection.")
  }
}

/// Hears the WebSocket open, or fail before it does.
private final class WebSocketDelegate: NSObject, URLSessionWebSocketDelegate, Sendable {
  private enum Opening {
    case waiting(CheckedContinuation<Void, any Error>?)
    case settled(Result<Void, any Error>)
  }

  private let opening = Mutex(Opening.waiting(nil))

  /// Resumes `continuation` when the WebSocket opens or fails, or at once if it has.
  func waitForOpen(_ continuation: CheckedContinuation<Void, any Error>) {
    let settled: Result<Void, any Error>? = opening.withLock { opening in
      switch opening {
      case .waiting:
        opening = .waiting(continuation)
        return nil
      case .settled(let result):
        return result
      }
    }
    if let settled {
      continuation.resume(with: settled)
    }
  }

  private func settle(_ result: Result<Void, any Error>) {
    let continuation: CheckedContinuation<Void, any Error>? = opening.withLock { opening in
      guard case .waiting(let continuation) = opening else { return nil }
      opening = .settled(result)
      return continuation
    }
    continuation?.resume(with: result)
  }

  func urlSession(
    _ session: URLSession, webSocketTask: URLSessionWebSocketTask,
    didOpenWithProtocol protocol: String?
  ) {
    // As rumqttc checks: the broker must answer with the `mqtt` subprotocol.
    guard `protocol` == "mqtt" else {
      settle(
        .failure(
          TransportError(message: "The broker didn't agree to speak MQTT over the WebSocket.")))
      webSocketTask.cancel(with: .protocolError, reason: nil)
      return
    }
    settle(.success(()))
  }

  func urlSession(
    _ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?
  ) {
    if let response = task.response as? HTTPURLResponse, response.statusCode != 101 {
      settle(
        .failure(
          TransportError(
            message: "The broker's address answered with HTTP \(response.statusCode), "
              + "not a WebSocket.")))
    } else {
      settle(
        .failure(
          error ?? TransportError(message: "The WebSocket closed before it opened.")))
    }
  }
}
