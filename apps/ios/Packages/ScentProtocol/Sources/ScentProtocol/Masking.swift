// Hiding passwords in text that goes to a log. Ported from `mask_passwords` in
// `apps/desktop/src/state.rs`.

private let mask = Array("***".utf8)
private let passwordKey = Array("password".utf8)

/// `text` with the value of every `password` key replaced by `"***"`.
///
/// Works on the text itself, so it also covers what is not valid JSON (the firmware sends
/// some) or is cut short, and JSON inside a JSON string. A key counts when its name
/// contains `password` in any case (`wifi_password`). Whatever the value is, a string, a
/// number or `null`, it is shown as `"***"`. Masking text that is already masked changes
/// nothing.
public func maskPasswords(_ text: String) -> String {
  let bytes = Array(text.utf8)
  let lower = bytes.map(asciiLower)  // same byte positions as `bytes`
  var masked: [UInt8] = []
  masked.reserveCapacity(bytes.count)
  var copied = 0  // `bytes[..<copied]` is already in `masked`
  var search = 0
  while let found = lower[search...].firstRange(of: passwordKey) {
    let key = found.lowerBound
    search = key + passwordKey.count
    guard let value = passwordValue(bytes, key: key) else { continue }
    masked += bytes[copied..<value.start]
    let quote = Array(repeating: UInt8(ascii: "\\"), count: value.escapes) + [UInt8(ascii: "\"")]
    masked += quote + mask + quote
    copied = value.end
    search = max(search, value.end)
  }
  masked += bytes[copied...]
  return String(decoding: masked, as: UTF8.self)
}

/// Where a password's value sits in the text.
private struct PasswordValue {
  var start: Int
  var end: Int
  /// Backslashes in front of each quote of a string value: 0 in plain JSON, more when the
  /// JSON sits inside a string.
  var escapes: Int
}

/// The value of the key that has `password` at byte `key`, if there is one.
private func passwordValue(_ bytes: [UInt8], key: Int) -> PasswordValue? {
  func byte(_ i: Int) -> UInt8? { i < bytes.count ? bytes[i] : nil }
  // The key is a quoted name with "password" somewhere in it.
  var nameStart = key
  while nameStart > 0 && isNameByte(bytes[nameStart - 1]) { nameStart -= 1 }
  guard nameStart > 0, bytes[nameStart - 1] == UInt8(ascii: "\"") else { return nil }
  var i = key + passwordKey.count
  while let b = byte(i), isNameByte(b) { i += 1 }
  while byte(i) == UInt8(ascii: "\\") { i += 1 }
  guard byte(i) == UInt8(ascii: "\"") else { return nil }
  i = skipSpaces(bytes, i + 1)
  guard byte(i) == UInt8(ascii: ":") else { return nil }
  let start = skipSpaces(bytes, i + 1)
  let escapes = bytes[start...].prefix { $0 == UInt8(ascii: "\\") }.count
  let quote = start + escapes
  if byte(quote) == UInt8(ascii: "\"") {
    return PasswordValue(
      start: start, end: endOfString(bytes, from: quote + 1, escapes: escapes), escapes: escapes)
  }
  // Not a string: it runs to the next delimiter.
  let end =
    bytes[start...].firstIndex {
      $0 == UInt8(ascii: ",") || $0 == UInt8(ascii: "}") || $0 == UInt8(ascii: "]")
        || isASCIIWhitespace($0)
    } ?? bytes.count
  return PasswordValue(start: start, end: end, escapes: 0)
}

private func isNameByte(_ b: UInt8) -> Bool {
  (UInt8(ascii: "a")...UInt8(ascii: "z")).contains(b)
    || (UInt8(ascii: "A")...UInt8(ascii: "Z")).contains(b)
    || (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(b)
    || b == UInt8(ascii: "_") || b == UInt8(ascii: "-")
}

/// Rust's `u8::is_ascii_whitespace`: space, tab, LF, form feed and CR (not vertical tab).
private func isASCIIWhitespace(_ b: UInt8) -> Bool {
  b == 0x20 || b == 0x09 || b == 0x0A || b == 0x0C || b == 0x0D
}

private func skipSpaces(_ bytes: [UInt8], _ from: Int) -> Int {
  var i = from
  while i < bytes.count && isASCIIWhitespace(bytes[i]) { i += 1 }
  return i
}

/// The position just after the quote that closes the string whose content starts at
/// `from`; the end of the text if it is never closed.
private func endOfString(_ bytes: [UInt8], from: Int, escapes: Int) -> Int {
  if escapes == 0 {
    var i = from
    while i < bytes.count {
      switch bytes[i] {
      case UInt8(ascii: "\\"): i += 2
      case UInt8(ascii: "\""): return i + 1
      default: i += 1
      }
    }
  } else {
    // In JSON inside a string, the closing quote has as many backslashes in front of it
    // as the opening one had; a quote inside the value has more.
    var run = 0
    for i in from..<max(from, bytes.count) {
      switch bytes[i] {
      case UInt8(ascii: "\\"): run += 1
      case UInt8(ascii: "\"") where run == escapes: return i + 1
      default: run = 0
      }
    }
  }
  return bytes.count
}
