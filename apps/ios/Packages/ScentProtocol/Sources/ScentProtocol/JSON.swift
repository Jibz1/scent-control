// A small JSON reader and writer that behave as serde_json does in the Rust crate (with
// `preserve_order`). Objects keep their keys in the order they were written or read, the
// writer gives the same compact text byte for byte, and the reader accepts and rejects
// what serde_json does. Foundation's JSON coders differ on all three, so frames are never
// built with them.

/// A JSON value.
enum JSON: Sendable {
  case null
  case bool(Bool)
  /// A whole number written without a fraction or an exponent. The reader gives `int` for
  /// negative numbers and `uint` for the rest, as serde_json does.
  case int(Int64)
  case uint(UInt64)
  /// Any other number. One that isn't finite is written as `null`, as serde_json does.
  /// The reader rounds correctly; serde_json (without `float_roundtrip`) can be off in the
  /// last digit for long numbers. Nothing in the protocol reads a double's value.
  case double(Double)
  case string(String)
  case array([JSON])
  case object(JSONObject)
}

/// A JSON object that keeps its keys in order. Keys are compared byte for byte. Setting a
/// key that is already there replaces its value and keeps its place, as serde_json does
/// with a repeated key.
struct JSONObject: Sendable, ExpressibleByDictionaryLiteral {
  private(set) var members: [(key: String, value: JSON)] = []
  private var positions: [[UInt8]: Int] = [:]

  init() {}

  init(dictionaryLiteral elements: (String, JSON)...) {
    for (key, value) in elements { set(key, value) }
  }

  subscript(key: String) -> JSON? {
    positions[Array(key.utf8)].map { members[$0].value }
  }

  mutating func set(_ key: String, _ value: JSON) {
    let bytes = Array(key.utf8)
    if let at = positions[bytes] {
      members[at].value = value
    } else {
      positions[bytes] = members.count
      members.append((key, value))
    }
  }
}

extension JSON {
  /// The value of `key` when this is an object that has it.
  subscript(key: String) -> JSON? {
    if case .object(let object) = self { object[key] } else { nil }
  }

  var string: String? {
    if case .string(let text) = self { text } else { nil }
  }

  var array: [JSON]? {
    if case .array(let items) = self { items } else { nil }
  }

  var object: JSONObject? {
    if case .object(let object) = self { object } else { nil }
  }

  /// serde_json's `as_i64`: a whole number that fits, never a number with a fraction or
  /// an exponent (`2.0` is not one).
  var int64: Int64? {
    switch self {
    case .int(let n): n
    case .uint(let n): n <= UInt64(Int64.max) ? Int64(n) : nil
    default: nil
    }
  }
}

// MARK: Writing

extension JSON {
  /// Compact JSON text, exactly as serde_json's `to_string` writes it.
  var text: String { String(decoding: bytes, as: UTF8.self) }

  var bytes: [UInt8] {
    var out: [UInt8] = []
    write(into: &out)
    return out
  }

  func write(into out: inout [UInt8]) {
    switch self {
    case .null: out += "null".utf8
    case .bool(let value): out += (value ? "true" : "false").utf8
    case .int(let n): out += String(n).utf8
    case .uint(let n): out += String(n).utf8
    case .double(let value): out += jsonNumber(value).utf8
    case .string(let text): writeString(text, into: &out)
    case .array(let items):
      out.append(UInt8(ascii: "["))
      for (n, item) in items.enumerated() {
        if n > 0 { out.append(UInt8(ascii: ",")) }
        item.write(into: &out)
      }
      out.append(UInt8(ascii: "]"))
    case .object(let object):
      out.append(UInt8(ascii: "{"))
      for (n, member) in object.members.enumerated() {
        if n > 0 { out.append(UInt8(ascii: ",")) }
        writeString(member.key, into: &out)
        out.append(UInt8(ascii: ":"))
        member.value.write(into: &out)
      }
      out.append(UInt8(ascii: "}"))
    }
  }
}

private let hexDigits = Array("0123456789abcdef".utf8)

/// A JSON string as serde_json writes it: `"` and `\` escaped, control characters as
/// `\b \t \n \f \r` or `\u00XX`, and everything else (`/`, DEL, non-ASCII) as it is.
private func writeString(_ text: String, into out: inout [UInt8]) {
  out.append(UInt8(ascii: "\""))
  for byte in text.utf8 {
    switch byte {
    case UInt8(ascii: "\""): out += #"\""#.utf8
    case UInt8(ascii: "\\"): out += #"\\"#.utf8
    case 0x08: out += #"\b"#.utf8
    case 0x09: out += #"\t"#.utf8
    case 0x0A: out += #"\n"#.utf8
    case 0x0C: out += #"\f"#.utf8
    case 0x0D: out += #"\r"#.utf8
    case ..<0x20:
      out += #"\u00"#.utf8
      out.append(hexDigits[Int(byte >> 4)])
      out.append(hexDigits[Int(byte & 0xF)])
    default: out.append(byte)
    }
  }
  out.append(UInt8(ascii: "\""))
}

/// A float as serde_json writes it (with the `zmij` formatter): the shortest digits that
/// read back as the same value, as a plain decimal with at least one digit after the point
/// when the decimal exponent is from -5 to 15 (`-5.0`, `0.00001`, `1000000000000000.0`),
/// otherwise as `1e+16`, `-4.5e-9`. Not finite gives `null`.
func jsonNumber(_ value: Double) -> String {
  guard value.isFinite else { return "null" }
  let sign = value.sign == .minus ? "-" : ""
  if value == 0 { return sign + "0.0" }
  let (digits, exponent) = shortestDigits(value.magnitude)
  let count = digits.count
  var out = sign
  if (-5...15).contains(exponent) {
    if count - 1 <= exponent {
      out += digits
      out += String(repeating: "0", count: exponent - (count - 1))
      out += ".0"
    } else if exponent >= 0 {
      let point = digits.index(digits.startIndex, offsetBy: exponent + 1)
      out += digits[..<point]
      out += "."
      out += digits[point...]
    } else {
      out += "0."
      out += String(repeating: "0", count: -exponent - 1)
      out += digits
    }
    return out
  }
  out += digits.prefix(1)
  if count > 1 {
    out += "."
    out += digits.dropFirst()
  }
  out += exponent >= 0 ? "e+" : "e-"
  out += String(exponent.magnitude)
  return out
}

/// The shortest decimal digits that read back as `magnitude` (finite, above zero), with
/// no leading or trailing zeros, and the power of ten of the first digit. Swift's own
/// `description` already finds the shortest, closest digits; only its layout differs.
private func shortestDigits(_ magnitude: Double) -> (digits: String, exponent: Int) {
  let text = magnitude.description
  var mantissa = Substring(text)
  var power = 0
  if let e = text.firstIndex(where: { $0 == "e" || $0 == "E" }) {
    mantissa = text[..<e]
    power = Int(text[text.index(after: e)...]) ?? 0
  }
  var digits = ""
  var whole: Int?
  for character in mantissa {
    if character == "." { whole = digits.count } else { digits.append(character) }
  }
  var exponent = (whole ?? digits.count) - 1 + power
  while digits.first == "0" {
    digits.removeFirst()
    exponent -= 1
  }
  while digits.last == "0" { digits.removeLast() }
  return (digits, exponent)
}

// MARK: Reading

extension JSON {
  /// Reads one JSON document as serde_json's `from_str` does, or `nil` where it fails.
  static func parse(_ text: String) -> JSON? { parse(Array(text.utf8)) }

  static func parse(_ bytes: [UInt8]) -> JSON? {
    var reader = JSONReader(bytes: bytes)
    return reader.document()
  }
}

/// serde_json's grammar: strict RFC 8259, whitespace only as space, tab, CR and LF, no
/// trailing commas, at most 127 levels of nesting, strings without raw control
/// characters or lone surrogates, numbers that fit in a double.
private struct JSONReader {
  let bytes: [UInt8]
  var at = 0
  /// serde_json's recursion limit: entering the 128th level fails.
  var depthLeft = 128

  init(bytes: [UInt8]) { self.bytes = bytes }

  mutating func document() -> JSON? {
    guard let value = value() else { return nil }
    skipWhitespace()
    return at == bytes.count ? value : nil
  }

  private var current: UInt8? { at < bytes.count ? bytes[at] : nil }

  private mutating func skipWhitespace() {
    while let byte = current, byte == 0x20 || byte == 0x09 || byte == 0x0A || byte == 0x0D {
      at += 1
    }
  }

  private mutating func value() -> JSON? {
    skipWhitespace()
    guard let byte = current else { return nil }
    switch byte {
    case UInt8(ascii: "n"): return literal("null", .null)
    case UInt8(ascii: "t"): return literal("true", .bool(true))
    case UInt8(ascii: "f"): return literal("false", .bool(false))
    case UInt8(ascii: "\""):
      at += 1
      return string().map(JSON.string)
    case UInt8(ascii: "-"), UInt8(ascii: "0")...UInt8(ascii: "9"): return number()
    case UInt8(ascii: "["): return nested { $0.array() }
    case UInt8(ascii: "{"): return nested { $0.object() }
    default: return nil
    }
  }

  private mutating func literal(_ word: String, _ value: JSON) -> JSON? {
    for byte in word.utf8 {
      guard current == byte else { return nil }
      at += 1
    }
    return value
  }

  private mutating func nested(_ body: (inout JSONReader) -> JSON?) -> JSON? {
    depthLeft -= 1
    guard depthLeft > 0 else { return nil }
    at += 1
    let value = body(&self)
    depthLeft += 1
    return value
  }

  private mutating func array() -> JSON? {
    var items: [JSON] = []
    skipWhitespace()
    if current == UInt8(ascii: "]") {
      at += 1
      return .array(items)
    }
    while true {
      guard let item = value() else { return nil }
      items.append(item)
      skipWhitespace()
      switch current {
      case UInt8(ascii: ","): at += 1
      case UInt8(ascii: "]"):
        at += 1
        return .array(items)
      default: return nil
      }
    }
  }

  private mutating func object() -> JSON? {
    var object = JSONObject()
    skipWhitespace()
    if current == UInt8(ascii: "}") {
      at += 1
      return .object(object)
    }
    while true {
      skipWhitespace()
      guard current == UInt8(ascii: "\"") else { return nil }
      at += 1
      guard let key = string() else { return nil }
      skipWhitespace()
      guard current == UInt8(ascii: ":") else { return nil }
      at += 1
      guard let member = value() else { return nil }
      object.set(key, member)
      skipWhitespace()
      switch current {
      case UInt8(ascii: ","): at += 1
      case UInt8(ascii: "}"):
        at += 1
        return .object(object)
      default: return nil
      }
    }
  }

  /// The rest of a string whose opening quote has been read.
  private mutating func string() -> String? {
    var out: [UInt8] = []
    while let byte = current {
      at += 1
      switch byte {
      case UInt8(ascii: "\""): return String(decoding: out, as: UTF8.self)
      case UInt8(ascii: "\\"):
        guard let escaped = current else { return nil }
        at += 1
        switch escaped {
        case UInt8(ascii: "\""), UInt8(ascii: "\\"), UInt8(ascii: "/"): out.append(escaped)
        case UInt8(ascii: "b"): out.append(0x08)
        case UInt8(ascii: "f"): out.append(0x0C)
        case UInt8(ascii: "n"): out.append(0x0A)
        case UInt8(ascii: "r"): out.append(0x0D)
        case UInt8(ascii: "t"): out.append(0x09)
        case UInt8(ascii: "u"):
          guard let scalar = unicodeEscape() else { return nil }
          UTF8.encode(scalar) { out.append($0) }
        default: return nil
        }
      case ..<0x20: return nil
      default: out.append(byte)
      }
    }
    return nil
  }

  /// The character of a `\u` escape whose `\u` has been read. A leading surrogate must be
  /// followed by a `\u` escape of a trailing one.
  private mutating func unicodeEscape() -> Unicode.Scalar? {
    guard let first = hex4() else { return nil }
    switch first {
    case 0xDC00...0xDFFF: return nil
    case 0xD800...0xDBFF:
      guard current == UInt8(ascii: "\\") else { return nil }
      at += 1
      guard current == UInt8(ascii: "u") else { return nil }
      at += 1
      guard let second = hex4(), (0xDC00...0xDFFF).contains(second) else { return nil }
      return Unicode.Scalar(((first - 0xD800) << 10 | (second - 0xDC00)) + 0x1_0000)
    default: return Unicode.Scalar(first)
    }
  }

  private mutating func hex4() -> UInt32? {
    var value: UInt32 = 0
    for _ in 0..<4 {
      guard let byte = current else { return nil }
      let digit: UInt8
      switch byte {
      case UInt8(ascii: "0")...UInt8(ascii: "9"): digit = byte - UInt8(ascii: "0")
      case UInt8(ascii: "a")...UInt8(ascii: "f"): digit = byte - UInt8(ascii: "a") + 10
      case UInt8(ascii: "A")...UInt8(ascii: "F"): digit = byte - UInt8(ascii: "A") + 10
      default: return nil
      }
      value = value << 4 | UInt32(digit)
      at += 1
    }
    return value
  }

  /// A number: `int` or `uint` when whole and in range, else `double`. `-0` is the
  /// double -0.0 and a number too large for a double fails, as in serde_json.
  private mutating func number() -> JSON? {
    let start = at
    let negative = current == UInt8(ascii: "-")
    if negative { at += 1 }
    guard let lead = current, isDigit(lead) else { return nil }
    at += 1
    if lead == UInt8(ascii: "0") {
      if let next = current, isDigit(next) { return nil }
    } else {
      skipDigits()
    }
    let wholeEnd = at
    var isFloat = false
    if current == UInt8(ascii: ".") {
      at += 1
      guard let digit = current, isDigit(digit) else { return nil }
      skipDigits()
      isFloat = true
    }
    if current == UInt8(ascii: "e") || current == UInt8(ascii: "E") {
      at += 1
      if current == UInt8(ascii: "+") || current == UInt8(ascii: "-") { at += 1 }
      guard let digit = current, isDigit(digit) else { return nil }
      skipDigits()
      isFloat = true
    }
    let text = String(decoding: bytes[start..<at], as: UTF8.self)
    if !isFloat {
      let digits = String(decoding: bytes[(negative ? start + 1 : start)..<wholeEnd], as: UTF8.self)
      if let magnitude = UInt64(digits) {
        if !negative { return .uint(magnitude) }
        if magnitude != 0 && magnitude <= UInt64(Int64.max) + 1 {
          return .int(Int64(truncatingIfNeeded: 0 &- magnitude))
        }
      }
    }
    guard let value = Double(text), value.isFinite else { return nil }
    return .double(value)
  }

  private mutating func skipDigits() {
    while let byte = current, isDigit(byte) { at += 1 }
  }

  private func isDigit(_ byte: UInt8) -> Bool {
    (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte)
  }
}
