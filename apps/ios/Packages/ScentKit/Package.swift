// swift-tools-version: 6.2
// The engine behind the screens: what the app knows about its diffusers, and how a
// person's action becomes a message to one, over Bluetooth or the broker. Ported from the
// Mac app's hub (`apps/desktop/src/hub`). Builds for the Mac too, so `swift test` runs its
// tests without a simulator.
import PackageDescription

let package = Package(
  name: "ScentKit",
  platforms: [.iOS(.v26), .macOS(.v26)],
  products: [
    .library(name: "ScentEngine", targets: ["ScentEngine"])
  ],
  dependencies: [
    .package(path: "../ScentProtocol"),
    .package(path: "../ScentMQTT"),
    .package(path: "../ScentBluetooth"),
  ],
  targets: [
    .target(
      name: "ScentEngine",
      dependencies: [
        .product(name: "ScentProtocol", package: "ScentProtocol"),
        .product(name: "ScentMQTT", package: "ScentMQTT"),
        .product(name: "ScentBluetooth", package: "ScentBluetooth"),
      ]),
    .testTarget(name: "ScentEngineTests", dependencies: ["ScentEngine"]),
  ]
)
