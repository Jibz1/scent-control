// swift-tools-version: 6.2
// The diffusers' protocol, ported from `packages/scent-core`: pure, with no I/O (bytes or
// JSON in, typed data out). Tested on the Mac with `swift test` against the same captured
// messages as the Rust crate, in `fixtures/`.
import PackageDescription

let package = Package(
  name: "ScentProtocol",
  platforms: [.iOS(.v26), .macOS(.v26)],
  products: [
    .library(name: "ScentProtocol", targets: ["ScentProtocol"])
  ],
  targets: [
    .target(name: "ScentProtocol"),
    .testTarget(name: "ScentProtocolTests", dependencies: ["ScentProtocol"]),
  ]
)
