// Text handling as Rust's `str` does it, so the parsers read text exactly as the Rust
// crate does. Swift's `String` compares and splits by grapheme cluster and treats
// canonically equivalent text as equal; Rust works on bytes and characters. Splitting
// here is on ASCII bytes, comparing is byte for byte, and trimming is per character.

/// Text compared byte for byte, for `switch` statements over known keys and codes.
struct Bytes: Equatable, ExpressibleByStringLiteral {
  let utf8: [UInt8]

  init(_ text: some StringProtocol) { utf8 = Array(text.utf8) }
  init(stringLiteral text: String) { utf8 = Array(text.utf8) }
}

/// Whether two texts have the same bytes.
func same(_ a: some StringProtocol, _ b: some StringProtocol) -> Bool {
  a.utf8.elementsEqual(b.utf8)
}

/// Rust's `str::trim`: without the leading and trailing characters that have the Unicode
/// White_Space property.
func trimmed(_ text: some StringProtocol) -> String {
  let scalars = text.unicodeScalars
  guard let first = scalars.firstIndex(where: { !$0.properties.isWhitespace }),
    let last = scalars.lastIndex(where: { !$0.properties.isWhitespace })
  else { return "" }
  var out = String.UnicodeScalarView()
  out.append(contentsOf: scalars[first...last])
  return String(out)
}

/// Rust's `str::split` on an ASCII character: every piece, empty ones included.
func split(_ text: some StringProtocol, on separator: Unicode.Scalar) -> [String] {
  text.utf8.split(separator: UInt8(ascii: separator), omittingEmptySubsequences: false)
    .map { String(decoding: $0, as: UTF8.self) }
}

/// Rust's `str::split_once` on an ASCII character.
func splitOnce(_ text: some StringProtocol, on separator: Unicode.Scalar) -> (String, String)? {
  let utf8 = text.utf8
  guard let at = utf8.firstIndex(of: UInt8(ascii: separator)) else { return nil }
  return (
    String(decoding: utf8[..<at], as: UTF8.self),
    String(decoding: utf8[utf8.index(after: at)...], as: UTF8.self)
  )
}

/// Rust's `str::split_whitespace`: the pieces between runs of Unicode White_Space.
func words(_ text: some StringProtocol) -> [String] {
  text.unicodeScalars.split(whereSeparator: { $0.properties.isWhitespace }).map { piece in
    var out = String.UnicodeScalarView()
    out.append(contentsOf: piece)
    return String(out)
  }
}

/// `text` without `prefix`, compared byte for byte, or `nil` when it doesn't start with it.
func stripPrefix(_ text: String, _ prefix: String) -> String? {
  guard text.utf8.starts(with: prefix.utf8) else { return nil }
  return String(decoding: text.utf8.dropFirst(prefix.utf8.count), as: UTF8.self)
}

/// Rust's `to_ascii_lowercase`: only A to Z change.
func asciiLowercased(_ text: some StringProtocol) -> String {
  String(decoding: text.utf8.map(asciiLower), as: UTF8.self)
}

/// Rust's `to_ascii_uppercase`: only a to z change.
func asciiUppercased(_ text: some StringProtocol) -> String {
  String(
    decoding: text.utf8.map { (0x61...0x7A).contains($0) ? $0 - 0x20 : $0 }, as: UTF8.self)
}

func asciiLower(_ byte: UInt8) -> UInt8 {
  (0x41...0x5A).contains(byte) ? byte + 0x20 : byte
}

/// Rust's `str::parse` for an integer type: an optional `+` (or `-` for a signed type),
/// then one or more ASCII digits, and a value that fits. Nothing else, not even spaces.
func parseInteger<T: FixedWidthInteger>(_ text: some StringProtocol, as _: T.Type = T.self) -> T? {
  var digits = Substring(text).utf8[...]
  var negative = false
  if let sign = digits.first, sign == UInt8(ascii: "+") || (sign == UInt8(ascii: "-") && T.isSigned)
  {
    negative = sign == UInt8(ascii: "-")
    digits = digits.dropFirst()
  }
  guard !digits.isEmpty else { return nil }
  var value: T = 0
  for byte in digits {
    guard (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte) else { return nil }
    let digit = T(byte - UInt8(ascii: "0"))
    let (shifted, overflowA) = value.multipliedReportingOverflow(by: 10)
    let (next, overflowB) =
      negative
      ? shifted.subtractingReportingOverflow(digit) : shifted.addingReportingOverflow(digit)
    guard !overflowA, !overflowB else { return nil }
    value = next
  }
  return value
}

/// `text` as Rust's `{:?}` shows a string: in double quotes, with `\"`, `\\`, `\0`, `\t`,
/// `\r` and `\n` escaped, and `\u{..}` for characters that don't print or that extend
/// the one before.
func rustDebug(_ text: String) -> String {
  var out = "\""
  for scalar in text.unicodeScalars {
    switch scalar {
    case "\0": out += "\\0"
    case "\t": out += "\\t"
    case "\r": out += "\\r"
    case "\n": out += "\\n"
    case "\\": out += "\\\\"
    case "\"": out += "\\\""
    default:
      if scalar.properties.isGraphemeExtend || !isPrintable(scalar) {
        out += "\\u{" + String(scalar.value, radix: 16) + "}"
      } else {
        out.unicodeScalars.append(scalar)
      }
    }
  }
  return out + "\""
}

/// Rust's `is_printable`: everything but separators other than the space, control and
/// format characters, surrogates, private use and unassigned code points.
private func isPrintable(_ scalar: Unicode.Scalar) -> Bool {
  if scalar.value < 0x20 { return false }
  if scalar.value < 0x7F { return true }
  switch scalar.properties.generalCategory {
  case .spaceSeparator, .lineSeparator, .paragraphSeparator, .control, .format, .surrogate,
    .privateUse, .unassigned:
    return false
  default:
    return true
  }
}
