// swift-tools-version: 6.2
// The iPhone app's screens, in SwiftUI, drawn from ScentEngine's model. iPhone only.
import PackageDescription

let package = Package(
  name: "ScentUI",
  platforms: [.iOS(.v26)],
  products: [
    .library(name: "ScentUI", targets: ["ScentUI"])
  ],
  dependencies: [
    .package(path: "../ScentKit"),
    .package(path: "../ScentProtocol"),
  ],
  targets: [
    .target(
      name: "ScentUI",
      dependencies: [
        .product(name: "ScentEngine", package: "ScentKit"),
        .product(name: "ScentProtocol", package: "ScentProtocol"),
      ])
  ]
)
