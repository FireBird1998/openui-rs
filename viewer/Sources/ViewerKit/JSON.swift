import Foundation

/// A JSON value that keeps object keys in the order they were written, which
/// Foundation's decoders don't. Props are drawn in the core's order.
public enum JSONValue: Sendable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([(key: String, value: JSONValue)])

    public static func == (a: JSONValue, b: JSONValue) -> Bool {
        switch (a, b) {
        case (.null, .null): true
        case let (.bool(x), .bool(y)): x == y
        case let (.number(x), .number(y)): x == y
        case let (.string(x), .string(y)): x == y
        case let (.array(x), .array(y)): x == y
        case let (.object(x), .object(y)):
            x.count == y.count && zip(x, y).allSatisfy { $0.key == $1.key && $0.value == $1.value }
        default: false
        }
    }

    /// The value at `key`, if this is an object that has it.
    public subscript(key: String) -> JSONValue? {
        guard case let .object(entries) = self else { return nil }
        return entries.first { $0.key == key }?.value
    }

    public var string: String? {
        if case let .string(s) = self { s } else { nil }
    }

    public var bool: Bool? {
        if case let .bool(b) = self { b } else { nil }
    }

    public var number: Double? {
        if case let .number(n) = self { n } else { nil }
    }

    public var array: [JSONValue]? {
        if case let .array(a) = self { a } else { nil }
    }

    /// Compact JSON text, for showing values that have no better form.
    public var compact: String {
        switch self {
        case .null: "null"
        case let .bool(b): b ? "true" : "false"
        case let .number(n): JSONValue.format(n)
        case let .string(s): JSONValue.quote(s)
        case let .array(a): "[" + a.map(\.compact).joined(separator: ",") + "]"
        case let .object(o): "{" + o.map { JSONValue.quote($0.key) + ":" + $0.value.compact }.joined(separator: ",") + "}"
        }
    }

    /// A number the way JavaScript prints it: no ".0" on whole numbers.
    public static func format(_ n: Double) -> String {
        if n == n.rounded(), abs(n) < 1e15 { String(Int64(n)) } else { String(n) }
    }

    static func quote(_ s: String) -> String {
        var out = "\""
        for scalar in s.unicodeScalars {
            switch scalar {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            case _ where scalar.value < 0x20: out += String(format: "\\u%04x", scalar.value)
            default: out.unicodeScalars.append(scalar)
            }
        }
        return out + "\""
    }
}

public struct JSONError: Error, CustomStringConvertible {
    public let offset: Int
    public let reason: String
    public var description: String { "JSON error at byte \(offset): \(reason)" }
}

extension JSONValue {
    /// Parses JSON text. Escaped lone surrogates, which JavaScript strings may
    /// hold but Swift strings can't, become U+FFFD.
    public static func parse(_ text: String) throws(JSONError) -> JSONValue {
        var reader = Reader(bytes: Array(text.utf8))
        reader.skipSpace()
        let value = try reader.value(depth: 0)
        reader.skipSpace()
        guard reader.at == reader.bytes.count else { throw reader.fail("trailing text") }
        return value
    }
}

private struct Reader {
    /// Far deeper than any parse result; stops runaway input before the stack does.
    static let maxDepth = 4000

    let bytes: [UInt8]
    var at = 0

    func fail(_ reason: String) -> JSONError { JSONError(offset: at, reason: reason) }

    mutating func skipSpace() {
        while at < bytes.count, [0x20, 0x09, 0x0A, 0x0D].contains(bytes[at]) { at += 1 }
    }

    mutating func expect(_ word: String) throws(JSONError) {
        for b in word.utf8 {
            guard at < bytes.count, bytes[at] == b else { throw fail("expected \(word)") }
            at += 1
        }
    }

    mutating func value(depth: Int) throws(JSONError) -> JSONValue {
        guard depth < Reader.maxDepth else { throw fail("nested too deeply") }
        guard at < bytes.count else { throw fail("unexpected end") }
        switch bytes[at] {
        case UInt8(ascii: "{"):
            at += 1
            var entries: [(key: String, value: JSONValue)] = []
            skipSpace()
            if at < bytes.count, bytes[at] == UInt8(ascii: "}") { at += 1; return .object(entries) }
            while true {
                skipSpace()
                guard at < bytes.count, bytes[at] == UInt8(ascii: "\"") else { throw fail("expected a key") }
                let key = try string()
                skipSpace()
                try expect(":")
                skipSpace()
                entries.append((key, try value(depth: depth + 1)))
                skipSpace()
                guard at < bytes.count else { throw fail("unexpected end") }
                if bytes[at] == UInt8(ascii: ",") { at += 1; continue }
                if bytes[at] == UInt8(ascii: "}") { at += 1; return .object(entries) }
                throw fail("expected , or }")
            }
        case UInt8(ascii: "["):
            at += 1
            var items: [JSONValue] = []
            skipSpace()
            if at < bytes.count, bytes[at] == UInt8(ascii: "]") { at += 1; return .array(items) }
            while true {
                skipSpace()
                items.append(try value(depth: depth + 1))
                skipSpace()
                guard at < bytes.count else { throw fail("unexpected end") }
                if bytes[at] == UInt8(ascii: ",") { at += 1; continue }
                if bytes[at] == UInt8(ascii: "]") { at += 1; return .array(items) }
                throw fail("expected , or ]")
            }
        case UInt8(ascii: "\""):
            return .string(try string())
        case UInt8(ascii: "t"):
            try expect("true")
            return .bool(true)
        case UInt8(ascii: "f"):
            try expect("false")
            return .bool(false)
        case UInt8(ascii: "n"):
            try expect("null")
            return .null
        default:
            return .number(try number())
        }
    }

    mutating func number() throws(JSONError) -> Double {
        let start = at
        while at < bytes.count, "+-0123456789.eE".utf8.contains(bytes[at]) { at += 1 }
        guard at > start, let n = Double(String(decoding: bytes[start..<at], as: UTF8.self)) else {
            throw fail("expected a value")
        }
        return n
    }

    mutating func hex4() throws(JSONError) -> UInt32 {
        guard at + 4 <= bytes.count,
              let v = UInt32(String(decoding: bytes[at..<at + 4], as: UTF8.self), radix: 16)
        else { throw fail("bad \\u escape") }
        at += 4
        return v
    }

    mutating func string() throws(JSONError) -> String {
        at += 1 // the opening quote
        var out: [UInt8] = []
        func append(_ scalar: Unicode.Scalar) { out.append(contentsOf: Array(String(scalar).utf8)) }
        while true {
            guard at < bytes.count else { throw fail("unterminated string") }
            let b = bytes[at]
            at += 1
            if b == UInt8(ascii: "\"") { return String(decoding: out, as: UTF8.self) }
            guard b == UInt8(ascii: "\\") else { out.append(b); continue }
            guard at < bytes.count else { throw fail("unterminated string") }
            let e = bytes[at]
            at += 1
            switch e {
            case UInt8(ascii: "\""): out.append(0x22)
            case UInt8(ascii: "\\"): out.append(0x5C)
            case UInt8(ascii: "/"): out.append(0x2F)
            case UInt8(ascii: "b"): out.append(0x08)
            case UInt8(ascii: "f"): out.append(0x0C)
            case UInt8(ascii: "n"): out.append(0x0A)
            case UInt8(ascii: "r"): out.append(0x0D)
            case UInt8(ascii: "t"): out.append(0x09)
            case UInt8(ascii: "u"):
                let unit = try hex4()
                if (0xD800..<0xDC00).contains(unit),
                   at + 6 <= bytes.count, bytes[at] == UInt8(ascii: "\\"), bytes[at + 1] == UInt8(ascii: "u") {
                    let save = at
                    at += 2
                    let low = try hex4()
                    if (0xDC00..<0xE000).contains(low) {
                        append(Unicode.Scalar(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))!)
                        continue
                    }
                    at = save
                }
                append(Unicode.Scalar(unit) ?? "\u{FFFD}")
            default:
                throw fail("bad escape")
            }
        }
    }
}
