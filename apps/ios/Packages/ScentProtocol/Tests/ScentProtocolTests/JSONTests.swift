// The internal JSON reader and writer, which stand in for serde_json. Expected text here
// is what serde_json 1.0.151 gives for the same input.

import Foundation
import Testing

@testable import ScentProtocol

@Suite struct JSONWriterTests {
  @Test func stringsAreEscapedAsSerdeJSONEscapesThem() {
    #expect(JSON.string("a\"b\\c/d").text == #""a\"b\\c/d""#)
    #expect(JSON.string("\u{8}\u{c}\n\r\t").text == #""\b\f\n\r\t""#)
    #expect(JSON.string("\u{0}\u{1}\u{1f}").text == #""\u0000\u0001\u001f""#)
    // DEL, non-ASCII and the JavaScript line separators are written as they are.
    #expect(utf8(JSON.string("\u{7f}é🌿\u{2028}").text) == utf8("\"\u{7f}é🌿\u{2028}\""))
  }

  @Test func objectsKeepTheirKeysInOrder() {
    var object: JSONObject = ["z": .int(1), "a": .int(2)]
    object.set("m", .null)
    object.set("z", .bool(false))  // replaced in place
    #expect(JSON.object(object).text == #"{"z":false,"a":2,"m":null}"#)
    #expect(
      JSON.array([.int(-1), .uint(UInt64.max), .object([:]), .array([])]).text
        == "[-1,18446744073709551615,{},[]]")
  }

  @Test(arguments: [
    (0.0, "0.0"), (-0.0, "-0.0"), (-5.0, "-5.0"), (5.5, "5.5"), (-3.5, "-3.5"), (100.0, "100.0"),
    (0.1, "0.1"), (1.0 / 3.0, "0.3333333333333333"), (1e15, "1000000000000000.0"), (1e16, "1e+16"),
    (123456.789, "123456.789"), (1e-5, "0.00001"), (1e-6, "1e-6"), (1.25e-5, "0.0000125"),
    (-4.5e-9, "-4.5e-9"), (1e23, "1e+23"), (5e-324, "5e-324"),
    (Double.greatestFiniteMagnitude, "1.7976931348623157e+308"), (Double.nan, "null"),
    (-Double.infinity, "null"),
  ])
  func floatsAreWrittenAsSerdeJSONWritesThem(value: Double, expected: String) {
    #expect(JSON.double(value).text == expected)
  }

  @Test func floatsReadBackAsTheSameValue() {
    var generator = SystemRandomNumberGenerator()
    for _ in 0..<20_000 {
      let value = Double(bitPattern: generator.next())
      guard value.isFinite else { continue }
      let written = JSON.double(value).text
      #expect(Double(written)?.bitPattern == value.bitPattern, "\(written)")
    }
  }
}

@Suite struct JSONReaderTests {
  @Test func readsWhatSerdeJSONReads() {
    for text in [
      "null", "true", "false", "0", "-1", "1.5", "1e2", "-0", " \t\r\n{} \n", "[]", "{}", #""""#,
      #"{"a":[1,{"b":null}],"c":"é🌿\/"}"#, "18446744073709551616", "1e-400",
      "0e999999999999999999",
    ] {
      #expect(JSON.parse(text) != nil, "\(text.debugDescription)")
    }
  }

  @Test func rejectsWhatSerdeJSONRejects() {
    for text in [
      "", " ", "nul", "nulls", "True", "01", "-", "1.", ".5", "1e", "1e+", "+1", "[1,]",
      #"{"a":1,}"#,
      "[1 2]", #"{"a" 1}"#, "{a:1}", "'a'", #""a"#, #""\x""#, #""\u12""#, "\"a\u{1}b\"", "\"\t\"",
      #""\udf3f""#, #""\ud83c""#, #""\ud83cx""#, #""\ud83cA""#, "1e400", "-1e400", "{}{}",
      "[] x", "\u{c}{}", "{}\u{b}", "NaN", "Infinity",
    ] {
      #expect(JSON.parse(text) == nil, "\(text.debugDescription)")
    }
  }

  @Test func numbersAreIntegersOnlyWithoutAFractionOrExponent() {
    #expect(JSON.parse("5")?.int64 == 5)
    #expect(JSON.parse("-5")?.int64 == -5)
    #expect(JSON.parse("9223372036854775807")?.int64 == Int64.max)
    #expect(JSON.parse("-9223372036854775808")?.int64 == Int64.min)
    for text in [
      "5.0", "5e0", "-0", "9223372036854775808", "-9223372036854775809", "true", #""5""#,
    ] {
      #expect(JSON.parse(text)?.int64 == nil, "\(text)")
    }
    // `-0` is the double -0.0, as in serde_json.
    if case .double(let zero)? = JSON.parse("-0") {
      #expect(zero == 0 && zero.sign == .minus)
    } else {
      Issue.record("-0 should read as a double")
    }
  }

  @Test func aRepeatedKeyKeepsItsPlaceAndTakesTheLastValue() {
    #expect(JSON.parse(#"{"a":1,"b":2,"a":3}"#)?.text == #"{"a":3,"b":2}"#)
    // Keys are compared byte for byte, not as canonically equivalent text.
    #expect(JSON.parse("{\"\u{e9}\":1,\"e\u{301}\":2}")?.object?.members.count == 2)
  }

  @Test func nestingStopsAt127Levels() {
    let deep = String(repeating: "[", count: 127) + String(repeating: "]", count: 127)
    #expect(JSON.parse(deep) != nil)
    let deeper = String(repeating: "[", count: 128) + String(repeating: "]", count: 128)
    #expect(JSON.parse(deeper) == nil)
    let objects =
      String(repeating: #"{"a":"#, count: 128) + "1" + String(repeating: "}", count: 128)
    #expect(JSON.parse(objects) == nil)
  }

  @Test func whatIsReadIsWrittenBackCompactly() {
    #expect(
      JSON.parse(#" { "data" : [ 85 , -1 , 2.50 , "é\n" , true , null ] } "#)?.text
        == "{\"data\":[85,-1,2.5,\"\u{e9}\\n\",true,null]}")
  }
}

@Suite struct TextTests {
  @Test func rustIntegerParsing() {
    #expect(parseInteger("42", as: Int64.self) == 42)
    #expect(parseInteger("+42", as: Int64.self) == 42)
    #expect(parseInteger("-42", as: Int64.self) == -42)
    #expect(parseInteger("-9223372036854775808", as: Int64.self) == Int64.min)
    #expect(parseInteger("255", as: UInt8.self) == 255)
    #expect(parseInteger("+0", as: UInt8.self) == 0)
    for bad in ["", "+", "-", " 1", "1 ", "1_000", "0x1", "٣", "9223372036854775808"] {
      #expect(parseInteger(bad, as: Int64.self) == nil, "\(bad.debugDescription)")
    }
    for bad in ["256", "-0", "-1"] {
      #expect(parseInteger(bad, as: UInt8.self) == nil, "\(bad)")
    }
  }

  @Test func trimmingAndSplittingFollowRust() {
    #expect(trimmed("\u{3000}\u{a0} a b \n\u{2029}") == "a b")
    #expect(trimmed(" \u{301}x") == "\u{301}x")
    #expect(split("a||b|", on: "|") == ["a", "", "b", ""])
    #expect(split("", on: "|") == [""])
    #expect(words("  a\u{2003}b\t c  ") == ["a", "b", "c"])
  }

  @Test func rustDebugQuotesAndEscapes() {
    #expect(rustDebug("plain") == #""plain""#)
    #expect(rustDebug("a\"b\\c\n\r\t\0'") == #""a\"b\\c\n\r\t\0'""#)
    #expect(
      rustDebug("\u{1}\u{7f}\u{a0}\u{200b}\u{301}é🌿") == #""\u{1}\u{7f}\u{a0}\u{200b}\u{301}é🌿""#)
  }
}
