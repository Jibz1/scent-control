// swift-tools-version: 6.2
// A small MQTT 3.1.1 client over a secure WebSocket (`wss://`), for the app's broker. The
// packet codec is pure and tested; the connection uses URLSession's WebSocket and the
// system's TLS.
import PackageDescription

let package = Package(
  name: "ScentMQTT",
  platforms: [.iOS(.v26), .macOS(.v26)],
  products: [
    .library(name: "ScentMQTT", targets: ["ScentMQTT"])
  ],
  targets: [
    .target(name: "ScentMQTT"),
    .testTarget(name: "ScentMQTTTests", dependencies: ["ScentMQTT"]),
  ]
)
