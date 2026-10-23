import Foundation

// How many times a diffuser's schedule may hold. Ported from `max_times` in
// `apps/desktop/src/ui/schedule_editor.rs`.

/// The table sizes the firmware keeps, its always-on factory entry included: 7 up to
/// firmware 2.21, and 100 after (`docs/findings.md`, "Schedules: stored on the device").
private let smallTable = 7
private let largeTable = 100
private let lastSmallTable: [UInt32] = [2, 21]

/// How many times a diffuser with this firmware keeps, besides its always-on entry, which
/// every `UPDATE_PROFILE` sends first. A firmware that can't be read gets the smaller
/// number.
public func maxTimes(firmware: String?) -> Int {
  let parts = firmware?.split(separator: ".", omittingEmptySubsequences: false)
    .map { UInt32($0.trimmingCharacters(in: .whitespaces)) }
  let version = parts.flatMap { $0.contains(nil) ? nil : $0.compactMap { $0 } }
  let large = version.map { lastSmallTable.lexicographicallyPrecedes($0) } ?? false
  return (large ? largeTable : smallTable) - 1
}
