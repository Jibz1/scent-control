// swift-tools-version: 6.1
// Pins command-line tools for the iOS scripts. Run them with
// `swift run --package-path Tools <tool>` so every machine uses the same version.
import PackageDescription

let package = Package(
  name: "Tools",
  platforms: [.macOS(.v14)],
  dependencies: [
    .package(url: "https://github.com/yonaskolb/XcodeGen.git", exact: "2.46.0")
  ],
  targets: [
    .target(name: "Tools", path: "Sources")
  ]
)
