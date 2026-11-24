// Text: formatting and string edits. Upstream's lib/functions/text.ts, declared
// in its order.
import Foundation
import OpenUINative

extension IntelligentUI {
    static let textFunctions: [DefinedFunction] = [
        function("Format", #"Number as text: "number" 12,345.6, "currency" $12,345.68 (currency is a code like "EUR"), "percent" 45 -> 45%, "compact" 1.2M ($1.2M with a currency)"#, [
            .init("value", .number),
            .init("kind", .words(["number", "currency", "percent", "compact"]), optional: true),
            .init("decimals", .number, optional: true),
            .init("currency", .string, optional: true),
        ], returns: .string, implementation: TextFunctions.format),
        function("Join", #"Joins the items (or their field) into text with sep (default ", "), skipping nulls"#, [
            .init("list", .list),
            .init("sep", .string, optional: true),
            .init("field", .string, optional: true),
        ], returns: .string, implementation: TextFunctions.join),
        function("Plural", #"Count and word: @Plural(1, "guest") is "1 guest", 3 gives "3 guests"; plural defaults to word + "s""#, [
            .init("n", .number),
            .init("word", .string),
            .init("plural", .string, optional: true),
        ], returns: .string, implementation: TextFunctions.plural),
        function("Pad", #"Pads the start of value to length with char (default "0"), e.g. 7 -> "07""#, [
            .init("value", .date),
            .init("length", .number),
            .init("char", .string, optional: true),
        ], returns: .string, implementation: TextFunctions.pad),
        function("Upper", "Text in upper case", [
            .init("text", .string),
        ], returns: .string, implementation: TextFunctions.upper),
        function("Lower", "Text in lower case", [
            .init("text", .string),
        ], returns: .string, implementation: TextFunctions.lower),
        function("Trim", "Text without spaces at the start and end", [
            .init("text", .string),
        ], returns: .string, implementation: TextFunctions.trim),
        function("Capitalize", #"Each word with a capital first letter, e.g. "new york" is "New York""#, [
            .init("text", .string),
        ], returns: .string, implementation: TextFunctions.capitalize),
        function("Split", #"List of the parts of text between sep (default ","), each trimmed, empty parts dropped, e.g. @Split("a, b,,c") is ["a", "b", "c"]"#, [
            .init("text", .string),
            .init("sep", .string, optional: true),
        ], returns: .array(.string), implementation: TextFunctions.split),
        function("Replace", "Text with every find replaced by replacement", [
            .init("text", .string),
            .init("find", .string),
            .init("replacement", .string),
        ], returns: .string, implementation: TextFunctions.replace),
        function("Duration", #"Seconds as text: "short" 1h 20m, 4m 05s, 45s (default); "clock" 1:20:00, 4:05"#, [
            .init("seconds", .number),
            .init("style", .words(["short", "clock"]), optional: true),
        ], returns: .string, implementation: TextFunctions.duration),
        function("Default", #"value, or fallback when value is null, "" or not a number, e.g. an empty number input: @Default($weight, 0)"#, [
            .init("value", .any, optional: true),
            .init("fallback", .any),
        ], returns: .any, implementation: TextFunctions.defaultValue),
    ]
}

// KMP on UTF-16 code units. A mismatch reuses the longest matching prefix
// before the failed unit. Each comparison either consumes a unit or shortens
// the matched prefix, both when building the table and when scanning the text.
func forEachLiteralPart(of text: String, separatedBy separator: String,
                        equal: (UInt16, UInt16) -> Bool = (==),
                        visit: (String) throws -> Void) rethrows {
    var pattern: [(unit: UInt16, fallback: Int)] = []
    func advance(_ matched: Int, with unit: UInt16) -> Int {
        var length = matched
        while let next = pattern.dropFirst(length).first {
            if equal(next.unit, unit) { return length + 1 }
            guard length > 0 else { break }
            length = next.fallback
        }
        return 0
    }

    var border = 0
    for unit in separator.utf16 {
        // On a mismatch at this unit, fall back using only the prefix before it.
        let fallback = border
        border = advance(border, with: unit)
        pattern.append((unit, fallback))
    }
    guard !pattern.isEmpty else { try visit(text); return }

    let source = Array(text.utf16)
    var start = 0
    var matched = 0
    for (offset, unit) in source.enumerated() {
        matched = advance(matched, with: unit)
        if matched == pattern.count {
            let end = offset + 1
            let part = source.dropFirst(start).prefix(end - pattern.count - start)
            try visit(String(decoding: part, as: UTF16.self))
            start = end
            matched = 0 // Split and Replace consume non-overlapping matches.
        }
    }
    try visit(String(decoding: source.dropFirst(start), as: UTF16.self))
}

private enum TextFunctions {
    static func format(_ args: JSONValue) throws -> JSONValue {
        let value = try args.numberArgument("value")
        guard value.isFinite else { throw FunctionError("value is not a finite number") }
        let kind = args["kind"]?.string
        let compact = kind == "compact"
        let currency = args["currency"]?.string
        let usesCurrency = kind == "currency" || (compact && currency?.isEmpty == false)
        let fallback = compact || kind == "percent" ? 1 : 2
        let places = (args["decimals"]?.number).map { Int(min(10, max(0, $0))) } ?? fallback
        let amount = FormatAmount(value, places: places, padded: kind == "currency", compact: compact)
        let prefix = usesCurrency ? amount.currencyPrefix(currency ?? "USD") : ""
        return .string(amount.sign + prefix + amount.text + (kind == "percent" ? "%" : ""))
    }

    static func plural(_ args: JSONValue) throws -> JSONValue {
        let count = try args.numberArgument("n")
        guard count.isFinite else { throw FunctionError("n is not a finite number") }
        let word = try args.stringArgument("word")
        let noun = abs(count) == 1 ? word : (args["plural"]?.string ?? word + "s")
        let amount = FormatAmount(count, places: 2, padded: false, compact: false)
        return .string(amount.sign + amount.text + " " + noun)
    }

    static func pad(_ args: JSONValue) throws -> JSONValue {
        let text = try ListFunctions.text(args.itemArgument("value"))
        let length = try args.numberArgument("length")
        guard length.isFinite else { throw FunctionError("length is not a finite number") }
        let width = Int(max(0, min(1000, length.rounded(.towardZero))))
        let missing = width - text.utf16.count
        guard missing > 0 else { return .string(text) }
        let fill = args["char"]?.string ?? ""
        let units = Array((fill.isEmpty ? "0" : fill).utf16.prefix(missing))
        var prefix: [UInt16] = []
        while prefix.count < missing { prefix.append(contentsOf: units.prefix(missing - prefix.count)) }
        return .string(String(decoding: prefix, as: UTF16.self) + text)
    }

    static func upper(_ args: JSONValue) throws -> JSONValue {
        try .string(args.stringArgument("text").uppercased())
    }

    static func lower(_ args: JSONValue) throws -> JSONValue {
        // Unicode Final_Sigma ignores case-ignorable scalars on either side.
        // Lookahead stops at the next significant scalar (another sigma stops it too).
        let text = try args.stringArgument("text")
        var rest = text.unicodeScalars[...]
        var precededByCased = false
        var result = ""
        while let scalar = rest.popFirst() {
            if scalar == "Σ", precededByCased,
               rest.first(where: { !$0.properties.isCaseIgnorable })?.properties.isCased != true {
                result += "ς"
            } else {
                result += scalar.properties.lowercaseMapping
            }
            if !scalar.properties.isCaseIgnorable { precededByCased = scalar.properties.isCased }
        }
        return .string(result)
    }

    static func trim(_ args: JSONValue) throws -> JSONValue {
        try .string(String(decoding: JavaScript.trim(args.stringArgument("text")), as: UTF16.self))
    }

    static func capitalize(_ args: JSONValue) throws -> JSONValue {
        var result = ""
        var atStart = true
        for scalar in try args.stringArgument("text").unicodeScalars {
            switch scalar.properties.generalCategory {
            case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter:
                result += atStart ? String(scalar).uppercased() : String(scalar)
            default: result.unicodeScalars.append(scalar)
            }
            atStart = scalar == "-" || scalar == "(" || scalar == "/" || scalar == "\""
                || UInt16(exactly: scalar.value).map(JavaScript.isWhitespace) == true
        }
        return .string(result)
    }

    static func split(_ args: JSONValue) throws -> JSONValue {
        let text = try args.stringArgument("text")
        let separator = args["sep"]?.string ?? ","
        var result: [JSONValue] = []
        func append(_ part: String) throws {
            guard !part.isEmpty else { return }
            guard result.count < 10_000 else { throw FunctionError("more than 10000 items") }
            result.append(.string(part))
        }
        if separator.isEmpty {
            for unit in text.utf16 where !JavaScript.isWhitespace(unit) {
                try append(String(decoding: [unit], as: UTF16.self))
            }
        } else {
            try forEachLiteralPart(of: text, separatedBy: separator) { part in
                try append(String(decoding: JavaScript.trim(part), as: UTF16.self))
            }
        }
        return .array(result)
    }

    // Join separators and Replace replacements can multiply two admitted inputs.
    // Stop at the existing host result limit while constructing the text.
    static func append(_ piece: String, to result: inout String) throws {
        guard piece.utf8.count <= defaultHostByteLimit - result.utf8.count else {
            throw OpenUIError(.limitExceeded, "host function result byte limit")
        }
        result += piece
    }

    static func replace(_ args: JSONValue) throws -> JSONValue {
        let text = try args.stringArgument("text")
        let find = try args.stringArgument("find")
        let replacement = try args.stringArgument("replacement")
        guard !find.isEmpty else { return .string(text) }
        var result = ""
        var first = true
        try forEachLiteralPart(of: text, separatedBy: find) { part in
            if !first { try append(replacement, to: &result) }
            try append(part, to: &result)
            first = false
        }
        return .string(result)
    }

    static func duration(_ args: JSONValue) throws -> JSONValue {
        let seconds = try args.numberArgument("seconds")
        guard seconds.isFinite else { throw FunctionError("seconds is not a finite number") }
        let sign = seconds < 0 ? "-" : ""
        let total = JavaScript.round(abs(seconds))
        let h = (total / 3600).rounded(.down)
        let m = Int(total.truncatingRemainder(dividingBy: 3600) / 60)
        let s = Int(total.truncatingRemainder(dividingBy: 60))
        func two(_ n: Int) -> String { n < 10 ? "0" + String(n) : String(n) }
        if args["style"]?.string == "clock" {
            let hm = h == 0 ? String(m) : JavaScript.numberString(h) + ":" + two(m)
            return .string(sign + hm + ":" + two(s))
        }
        if h != 0 { return .string(sign + JavaScript.numberString(h) + "h" + (m == 0 ? "" : " \(m)m")) }
        if m != 0 { return .string("\(sign)\(m)m" + (s == 0 ? "" : " \(two(s))s")) }
        return .string("\(sign)\(s)s")
    }

    static func defaultValue(_ args: JSONValue) throws -> JSONValue {
        let fallback = try args.itemArgument("fallback")
        guard let value = args["value"], value != .null, value.string?.isEmpty != true else { return fallback }
        return value
    }
}

private extension TextFunctions {
    static func join(_ args: JSONValue) throws -> JSONValue {
        let list = try args.listArgument()
        let read = ListFunctions.fieldReader(args["field"]?.string ?? "", method: methodText)
        let separator = args["sep"]?.string ?? ", "
        var result = ""
        var first = true
        for item in list {
            guard let value = read(item), value != .null else { continue }
            if !first { try append(separator, to: &result) }
            first = false
            switch value {
            case .array, .object: try stringify(value, into: &result)
            default: try append(ListFunctions.text(value), to: &result)
            }
        }
        return .string(result)
    }

    // Bun's String(function) is observable when helpers.read finds an inherited method.
    static let objectMethods: Set<String> = [
        "toString", "toLocaleString", "valueOf", "hasOwnProperty", "propertyIsEnumerable", "isPrototypeOf",
        "__defineGetter__", "__defineSetter__", "__lookupGetter__", "__lookupSetter__",
    ]
    static let arrayMethods: Set<String> = [
        "at", "concat", "copyWithin", "entries", "every", "fill", "filter", "find", "findIndex", "findLast",
        "findLastIndex", "flat", "flatMap", "forEach", "includes", "indexOf", "join", "keys", "lastIndexOf",
        "map", "pop", "push", "reduce", "reduceRight", "reverse", "shift", "slice", "some", "sort", "splice",
        "toReversed", "toSorted", "toSpliced", "unshift", "values", "with",
    ]

    static func methodText(_ name: String, array: Bool) -> JSONValue? {
        if name == "constructor" { return .string("function \(array ? "Array" : "Object")() { [native code] }") }
        guard objectMethods.contains(name) || (array && arrayMethods.contains(name)) else { return nil }
        return .string("function \(name)() {\n    [native code]\n}")
    }

    private enum Step {
        case value(JSONValue), array(ArraySlice<JSONValue>), object(ArraySlice<(key: String, value: JSONValue)>)
    }

    // JSON.stringify for containers, whose numeric spelling and control
    // escapes differ from JSONValue.compact. The runtime supplies JS key order.
    static func stringify(_ value: JSONValue, into out: inout String) throws {
        var work = [Step.value(value)]
        while let step = work.popLast() {
            switch step {
            case .value(.object(let entries)):
                try append("{", to: &out)
                work.append(.object(entries[...]))
            case .value(.array(let items)):
                try append("[", to: &out)
                work.append(.array(items[...]))
            case .value(.string(let s)): try quote(s, into: &out)
            case .value(.number(let n)): try append(JavaScript.numberString(n), to: &out)
            case .value(let v): try append(v.compact, to: &out)
            case .array(let items):
                guard let first = items.first else { try append("]", to: &out); continue }
                if items.startIndex != 0 { try append(",", to: &out) }
                work.append(.array(items.dropFirst()))
                work.append(.value(first))
            case .object(let entries):
                guard let first = entries.first else { try append("}", to: &out); continue }
                if entries.startIndex != 0 { try append(",", to: &out) }
                try quote(first.key, into: &out)
                try append(":", to: &out)
                work.append(.object(entries.dropFirst()))
                work.append(.value(first.value))
            }
        }
    }

    static func quote(_ text: String, into out: inout String) throws {
        try append("\"", to: &out)
        for scalar in text.unicodeScalars {
            let escaped: String
            switch scalar {
            case "\"": escaped = "\\\""
            case "\\": escaped = "\\\\"
            case "\u{08}": escaped = "\\b"
            case "\u{0C}": escaped = "\\f"
            case "\n": escaped = "\\n"
            case "\r": escaped = "\\r"
            case "\t": escaped = "\\t"
            case _ where scalar.value < 32: escaped = String(format: "\\u%04x", scalar.value)
            default: escaped = String(scalar)
            }
            try append(escaped, to: &out)
        }
        try append("\"", to: &out)
    }
}

// Bun's en-US Intl rounds the shortest decimal representation of a finite
// Double. Scaling or rounding the binary value first loses decimal ties.
private struct FormatAmount {
    let sign: String
    let text: String
    let abbreviated: Bool

    init(_ value: Double, places: Int, padded: Bool, compact: Bool) {
        sign = value.sign == .minus ? "-" : ""
        let shortest = String(abs(value)).split(separator: "e")
        let mantissa = shortest.first ?? ""
        let exponent = shortest.dropFirst().first.flatMap { Int($0) } ?? 0
        let point = mantissa.prefix { $0 != "." }.count + exponent
        var scale = compact ? min(4, max(0, (point - 1) / 3)) : 0
        let digits = Self.roundedDigits(mantissa, point: point - 3 * scale, places: places)
        var integer = digits.dropLast(places)
        var fraction = digits.suffix(places)
        if compact, scale < 4, integer.count > 3 {
            // Rounding carried 999… into the next magnitude.
            scale += 1
            integer = [1]
            fraction = []
        }
        if !padded {
            while fraction.last == 0 { fraction = fraction.dropLast() }
        }

        var formatted = ""
        let group = integer.count >= (compact ? 5 : 4)
        for (offset, digit) in integer.enumerated() {
            if group, offset > 0, (integer.count - offset).isMultiple(of: 3) { formatted += "," }
            formatted += String(digit)
        }
        if !fraction.isEmpty { formatted += "." + fraction.map(String.init).joined() }
        let suffix = ["", "K", "M", "B", "T"].dropFirst(scale).first ?? ""
        abbreviated = scale > 0
        text = formatted + suffix
    }

    // Keep exactly 'places' digits after the point. The expanded Double has at
    // most 309 integral digits or 324 leading fractional zeros, and places ≤ 10.
    private static func roundedDigits(_ mantissa: Substring, point: Int, places: Int) -> [Int] {
        let digits = Array(repeating: 0, count: max(0, 1 - point)) + mantissa.compactMap(\.wholeNumberValue)
        let keep = max(1, point) + places
        var result = Array(digits.prefix(keep))
        result.append(contentsOf: repeatElement(0, count: keep - result.count))
        if let next = digits.dropFirst(keep).first, next >= 5 {
            var nines = 0
            while result.last == 9 {
                result.removeLast()
                nines += 1
            }
            let incremented = (result.popLast() ?? 0) + 1
            result.append(incremented)
            result.append(contentsOf: repeatElement(0, count: nines))
        }
        return result
    }

    // All non-code prefixes in the recording's en-US ICU data. Code prefixes
    // (including unknown three-letter codes) have a nonbreaking space.
    private static let currencySymbols = [
        "AUD": "A$", "BRL": "R$", "CAD": "CA$", "CNY": "CN¥", "EUR": "€", "GBP": "£",
        "HKD": "HK$", "ILS": "₪", "INR": "₹", "JPY": "¥", "KRW": "₩", "MXN": "MX$",
        "NZD": "NZ$", "PHP": "₱", "TWD": "NT$", "USD": "$", "VND": "₫", "XCD": "EC$",
        "XAF": "FCFA\u{a0}", "XCG": "Cg.\u{a0}", "XOF": "F\u{202f}CFA\u{a0}",
        "XPF": "CFPF\u{a0}", "XXX": "¤",
    ]

    func currencyPrefix(_ currency: String) -> String {
        // Uppercasing can expand one scalar (ß -> SS), so validate after casing.
        guard currency.unicodeScalars.prefix(4).count <= 3 else { return "$" }
        let code = currency.uppercased()
        guard code.utf8.count == 3, code.utf8.allSatisfy({ (65...90).contains($0) }) else { return "$" }
        if code == "SAR" { return abbreviated ? "⃁" : "\u{200e}⃁\u{a0}" }
        return Self.currencySymbols[code] ?? code + "\u{a0}"
    }
}
