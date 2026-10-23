// Helpers shared by the tests.

import Foundation

@testable import ScentProtocol

func level(_ n: Int) -> Level { Level(n)! }

func hhmm(_ text: String) -> HhMm { HhMm(parsing: text)! }

func data(_ text: String) -> Data { Data(text.utf8) }

func text(_ data: Data) -> String { String(decoding: data, as: UTF8.self) }

/// The bytes of `text`, for comparing byte for byte: `String` equality treats canonically
/// equivalent text as equal.
func utf8(_ text: String) -> [UInt8] { Array(text.utf8) }

/// The repository root: the first folder above this file that has `fixtures/scentiment`.
let repoRoot: URL = {
  var folder = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
  while !FileManager.default.fileExists(
    atPath: folder.appendingPathComponent("fixtures/scentiment").path)
  {
    let parent = folder.deletingLastPathComponent()
    precondition(parent != folder, "no fixtures folder above \(#filePath)")
    folder = parent
  }
  return folder
}()
