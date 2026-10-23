// swift-tools-version: 6.2
// Talking to the diffusers over Bluetooth with Core Bluetooth: scanning, reading their
// info and status, and writing command frames. Knows the GATT layout, not the protocol.
import PackageDescription

let package = Package(
  name: "ScentBluetooth",
  platforms: [.iOS(.v26), .macOS(.v26)],
  products: [
    .library(name: "ScentBluetooth", targets: ["ScentBluetooth"])
  ],
  targets: [
    .target(name: "ScentBluetooth"),
    .testTarget(name: "ScentBluetoothTests", dependencies: ["ScentBluetooth"]),
  ]
)
