// The library's 59 functions: upstream's lib/functions, declared with
// defineFunction in upstream's order (index.ts). A function with an
// implementation gives what upstream's gives in Bun, quirks and error
// messages included; one without returns null until its ticket lands.
// conformance/golden/intelligent-ui-functions.json records upstream's own
// results, and FunctionReplayTests replays them through the runtime.
//
// The runtime checks a call before the implementation runs, as upstream's
// does: an explicit null for an optional argument counts as missing, a
// missing required argument returns null without calling it, arguments of
// the wrong type are reported, and invalid array items are pruned. So an
// implementation reads its arguments knowing their types.
import OpenUINative

extension IntelligentUI {
    /// The 59 functions, in upstream's order (`functions` in index.ts).
    public static let functions: [DefinedFunction] = listFunctions + numberFunctions + textFunctions
        + colorFunctions + dateFunctions + dataFunctions

    /// One declaration, as upstream's `defineFunction` writes it.
    static func function(
        _ name: String, _ description: String, _ params: [FieldDefinition], returns: PropType,
        implementation: (@Sendable (JSONValue) throws -> JSONValue)? = nil
    ) -> DefinedFunction {
        defineFunction(name: name, description: description, params: params, returns: returns,
                       implementation: implementation)
    }
}

// The param types upstream's lib/functions/helpers.ts shares.
extension PropType {
    /// `list()`: any list.
    static let list = PropType.array(.any)
    /// `numbers()`: a list of numbers.
    static let numbers = PropType.array(.number)
    /// `date()`: a date string or a timestamp.
    static let date = PropType.union([.string, .number])
    /// `agg()`: how rows are aggregated.
    static let agg = PropType.words(["sum", "avg", "count", "min", "max"])
}

/// An error an implementation throws, as upstream's `throw new Error(message)`:
/// the runtime reports `@Name threw: <message>` and the call gives null.
struct FunctionError: Error, CustomStringConvertible {
    let description: String

    init(_ message: String) {
        description = message
    }
}

extension JSONValue {
    /// A string argument the runtime has already checked.
    func stringArgument(_ name: String) throws(FunctionError) -> String {
        guard let value = self[name]?.string else { throw FunctionError("\(name) is not a string") }
        return value
    }

    /// A number argument the runtime has already checked.
    func numberArgument(_ name: String) throws(FunctionError) -> Double {
        guard let value = self[name]?.number else { throw FunctionError("\(name) is not a number") }
        return value
    }
}

/// JavaScript's own behaviour where an implementation depends on it.
enum JavaScript {
    /// String(number) uses shortest round-trip digits and fixed notation
    /// from 1e-6 up to (but excluding) 1e21.
    static func numberString(_ n: Double) -> String {
        if n == 0 { return "0" }
        if n.isNaN { return "NaN" }
        if n.isInfinite { return n < 0 ? "-Infinity" : "Infinity" }
        let shortest = String(abs(n))
        let mantissa = shortest.prefix { $0 != "e" }
        let digits = String(mantissa.hasSuffix(".0") ? mantissa.dropLast(2) : mantissa)
        let sign = n < 0 ? "-" : ""
        let power = shortest[mantissa.endIndex...].dropFirst()
        guard let exponent = Int(power) else { return sign + digits }
        if !(-6..<21).contains(exponent) {
            return sign + digits + "e" + (exponent >= 0 ? "+" : "") + String(exponent)
        }
        let significant = digits.filter { $0 != "." }
        let point = exponent + 1
        if point <= 0 {
            return sign + "0." + String(repeating: "0", count: -point) + significant
        }
        if point >= significant.count {
            return sign + significant + String(repeating: "0", count: point - significant.count)
        }
        return sign + significant.prefix(point) + "." + significant.dropFirst(point)
    }

    /// `Math.round`: the nearest integer, halves toward +∞ (so -2.5 gives -2,
    /// and 0.49999999999999994 gives 0, which `floor(x + 0.5)` gets wrong).
    static func round(_ x: Double) -> Double {
        guard x.isFinite else { return x }
        let down = x.rounded(.down)
        let up = x - down >= 0.5 ? down + 1 : down
        // -0.4 rounds to -0, as JavaScript's does.
        return up == 0 && x.sign == .minus ? -0.0 : up
    }

    /// `String.prototype.trim`, on UTF-16 code units: JavaScript's whitespace
    /// and line terminators, which leave out U+0085, U+180E and U+200B.
    static func trim(_ text: String) -> Substring.UTF16View {
        let units = text.utf16
        let start = units.firstIndex { !isWhitespace($0) } ?? units.endIndex
        let end = units[start...].lastIndex { !isWhitespace($0) }.map { units.index(after: $0) } ?? start
        return units[start..<end]
    }

    /// WhiteSpace and LineTerminator in ECMA-262: the Zs category (Unicode
    /// 17, as Bun's), tab, vertical tab, form feed, BOM, LF, CR, LS and PS.
    static func isWhitespace(_ unit: UInt16) -> Bool {
        switch unit {
        case 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F,
             0x3000, 0xFEFF:
            true
        default:
            false
        }
    }
}
