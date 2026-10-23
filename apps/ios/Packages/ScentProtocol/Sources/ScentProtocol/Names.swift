// What a diffuser's Bluetooth name says about it. Ported from
// `packages/scent-core/src/names.rs`.

/// Model letters in a Bluetooth name, from `tools/scentiment_ble.py`.
private let modelCodes: [(code: String, name: String)] = [
  ("D", "Diffuser Air 2"),
  ("H", "Hvac"),
  ("P", "Pro"),
  ("M", "Mini"),
  ("T", "Mini 2 LE"),
  ("W", "The Plug-In"),
  ("R", "Diffuser_4"),
]

/// Variant letters in a Bluetooth name, from `tools/scentiment_ble.py`.
private let variantCodes: [(code: String, name: String)] = [
  ("B", "Black"),
  ("C", "Graphite Gold"),
  ("G", "Grey"),
  ("W", "White"),
  ("S", "Silver"),
  ("R", "Red"),
]

/// What a Bluetooth name says about the model. A code we don't know keeps its letter but
/// has no name.
public struct NameInfo: Sendable, Hashable {
  public var modelCode: String?
  public var model: String?
  public var variantCode: String?
  public var variant: String?

  public init(
    modelCode: String? = nil, model: String? = nil, variantCode: String? = nil,
    variant: String? = nil
  ) {
    self.modelCode = modelCode
    self.model = model
    self.variantCode = variantCode
    self.variant = variant
  }
}

/// True when a Bluetooth name belongs to a Scentiment diffuser.
public func isScentimentName(_ name: String) -> Bool {
  name.lowercased().utf8.firstRange(of: "scentiment".utf8) != nil
}

/// Reads the model and variant codes from `<prefix>-<ModelCode> <VariantCode> [suffix]`.
public func parseName(_ name: String) -> NameInfo {
  let tokens = words(splitOnce(name, on: "-")?.1 ?? "")
  let modelCode = tokens.first
  let variantCode = tokens.count > 1 ? tokens[1] : nil
  return NameInfo(
    modelCode: modelCode, model: modelCode.flatMap { lookup(modelCodes, $0) },
    variantCode: variantCode, variant: variantCode.flatMap { lookup(variantCodes, $0) })
}

/// The device ID is the last token of the advertised name, when it is 12 hex digits.
/// Returned in upper case.
public func deviceIDFromName(_ name: String) -> String? {
  guard let token = words(name).last, token.utf8.count == 12,
    token.utf8.allSatisfy(isASCIIHexDigit)
  else { return nil }
  return asciiUppercased(token)
}

private func isASCIIHexDigit(_ byte: UInt8) -> Bool {
  (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte)
    || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains(byte)
    || (UInt8(ascii: "A")...UInt8(ascii: "F")).contains(byte)
}

private func lookup(_ table: [(code: String, name: String)], _ code: String) -> String? {
  table.first { same($0.code, code) }?.name
}
