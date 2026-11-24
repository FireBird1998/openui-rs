import Foundation
import Observation
import SwiftUI

/// Structured rules follow react-ui 0.18.1 and lang-core 1.1.0. Regexes use
/// ICU with explicit translations for the named flagless JavaScript gaps; URL
/// checks cover special-scheme authorities, including numeric hosts, but are
/// not a full WHATWG parser.
struct ValidationRule: Equatable {
    var type: String
    var arg: JSONValue?
}

package enum Validation {
    /// react-ui registers structured objects only; list-form rules are ignored.
    static func rules(_ value: PropValue?) -> [ValidationRule] {
        switch value {
        case let .record(fields)?:
            return fields.compactMap { field in
                switch field.value {
                case .scalar(.bool(false)), .scalar(.null): return nil
                case .scalar(.bool(true)): return ValidationRule(type: field.name)
                default: return ValidationRule(type: field.name, arg: field.value.json)
                }
            }
        default:
            return []
        }
    }

    /// The first rule `value` breaks, in upstream's words; nil when it keeps
    /// them all. A nil value is undefined. Rule types upstream doesn't know
    /// are skipped, as upstream does.
    static func check(_ value: JSONValue?, _ rules: [ValidationRule]) -> String? {
        for rule in rules {
            if let error = check(value, rule) { return error }
        }
        return nil
    }

    private static func check(_ value: JSONValue?, _ rule: ValidationRule) -> String? {
        let empty = isEmpty(value)
        switch rule.type {
        case "required":
            if empty { return "This field is required" }
            if case let .object(entries)? = value, !entries.isEmpty,
               entries.allSatisfy({ $0.value.bool != nil }), !entries.contains(where: { $0.value.bool == true }) {
                return "At least one option is required"
            }
            return nil
        case "email":
            if empty { return nil }
            guard case let .string(s)? = value else { return "Please enter a valid email" }
            return matches(#"^[^\s@]+@[^\s@]+\.[^\s@]+$"#, s) == true ? nil : "Please enter a valid email"
        case "url":
            if empty { return nil }
            guard case let .string(s)? = value, isURL(s) else { return "Please enter a valid URL" }
            return nil
        case "numeric":
            if empty { return nil }
            switch value {
            case .number?: return nil
            case let .string(s)? where !JS.parseFloat(s).isNaN && !s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty: return nil
            default: return "Must be a number"
            }
        case "min", "max":
            if empty { return nil }
            let n: Double = if case let .number(n)? = value { n } else { JS.parseFloat(JS.string(value)) }
            if n.isNaN { return nil }
            let bound = JS.number(rule.arg)
            if rule.type == "min" { return n >= bound ? nil : "Must be at least \(JS.format(bound))" }
            return n <= bound ? nil : "Must be no more than \(JS.format(bound))"
        case "minLength", "maxLength":
            if empty { return nil }
            guard case let .string(s)? = value else { return nil }
            let length = Double(s.utf16.count)
            let bound = JS.number(rule.arg)
            if rule.type == "minLength" { return length >= bound ? nil : "Must be at least \(JS.format(bound)) characters" }
            return length <= bound ? nil : "Must be no more than \(JS.format(bound)) characters"
        case "pattern":
            if empty { return nil }
            guard case let .string(s)? = value, case let .string(pattern)? = rule.arg else { return nil }
            return matches(pattern, s) == false ? "Invalid format" : nil
        default:
            return nil
        }
    }

    /// Upstream's `isEmpty`: undefined, null, "", [], {}, or a `{value}`
    /// whose value is empty.
    private static func isEmpty(_ value: JSONValue?) -> Bool {
        switch value {
        case nil, .null?, .string("")?: return true
        case let .array(items)?: return items.isEmpty
        case let .object(entries)?:
            if let inner = entries.first(where: { $0.key == "value" }) { return isEmpty(inner.value) }
            return entries.isEmpty
        default: return false
        }
    }

    /// Whether `pattern` finds a match in `text`; nil when it doesn't compile.
    private static func matches(_ pattern: String, _ text: String) -> Bool? {
        // ECMAScript WhiteSpace + LineTerminator includes BOM but excludes
        // ICU's NEL (U+0085); it is Unicode even in a flagless JS pattern.
        let jsSpaces = #"\x{0009}-\x{000D}\x{0020}\x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}"#
        let chars = Array(pattern.unicodeScalars)
        var translated = ""
        var inClass = false
        var i = 0
        while i < chars.count {
            let c = chars[i]
            i += 1
            if c == "\\", i < chars.count {
                let escaped = chars[i]
                i += 1
                switch escaped {
                case "d": translated += "[0-9]"
                case "D": translated += "[^0-9]"
                case "s": translated += "[" + jsSpaces + "]"
                case "S": translated += "[^" + jsSpaces + "]"
                case "b" where !inClass, "B" where !inClass:
                    let word = "[A-Za-z0-9_]"
                    translated += escaped == "b"
                        ? "(?:(?<=\(word))(?!\(word))|(?<!\(word))(?=\(word)))"
                        : "(?:(?<=\(word))(?=\(word))|(?<!\(word))(?!\(word)))"
                case "b": translated += "\\x{08}"
                case "B" where inClass: translated += "B"
                case "w": translated += "[A-Za-z0-9_]"
                case "W": translated += "[^A-Za-z0-9_]"
                // With no u/v flag JavaScript treats property escapes as
                // identity escapes: \p{L} matches the literal p{L}.
                case "p", "P":
                    translated += String(escaped)
                    if i < chars.count, chars[i] == "{",
                       let end = chars[i...].firstIndex(of: "}") {
                        translated += NSRegularExpression.escapedPattern(for: String(String.UnicodeScalarView(chars[i...end])))
                        i = end + 1
                    }
                default: translated += "\\" + String(escaped)
                }
            } else if c == "[" {
                if inClass { translated += "\\["; continue }
                // ICU permits nested sets and literal initial ]; JavaScript's
                // flagless classes close at the first unescaped ].
                if i < chars.count, chars[i] == "]" {
                    translated += "(?!)"
                    i += 1
                } else if i + 1 < chars.count, chars[i] == "^", chars[i + 1] == "]" {
                    translated += "[\\s\\S]"
                    i += 2
                } else { translated += "["; inClass = true }
            } else if c == "]" {
                translated += inClass ? "]" : "\\]"
                inClass = false
            } else {
                // Bare inline modifiers are invalid in JavaScript RegExp.
                // Upstream ignores patterns that do not compile.
                if !inClass, c == "(", i + 1 < chars.count, chars[i] == "?",
                   "imswx-".unicodeScalars.contains(chars[i + 1]) {
                    var end = i + 1
                    while end < chars.count, "imswx-".unicodeScalars.contains(chars[end]) { end += 1 }
                    // Bare modifiers such as (?i) are invalid in JS. Scoped
                    // modifiers such as (?i:a) remain with the platform engine.
                    if end < chars.count, chars[end] == ")" { return nil }
                }
                translated += c == "$" && !inClass ? "\\z" : String(c)
            }
        }
        guard let regex = try? NSRegularExpression(pattern: translated) else { return nil }
        return regex.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
    }

    package static func isURL(_ s: String) -> Bool {
        // WHATWG strips ASCII tab/LF/CR throughout, and only C0/space at ends.
        let text = String(String.UnicodeScalarView(s.unicodeScalars.filter { $0 != "\t" && $0 != "\n" && $0 != "\r" }))
        let trimmed = text.trimmingCharacters(in: CharacterSet(charactersIn: "\u{0}"..." "))
        guard let colon = trimmed.firstIndex(of: ":"),
              trimmed[..<colon].first?.isLetter == true,
              trimmed[..<colon].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "+-.".contains($0)) })
        else { return false }
        let scheme = trimmed[..<colon].lowercased()
        guard ["http", "https", "ws", "wss", "ftp"].contains(scheme) else { return true }
        let rest = trimmed[trimmed.index(after: colon)...].drop { $0 == "/" || $0 == "\\" }
        let authority = rest.prefix { !"/\\?#".contains($0) }
        let hostPort = authority.split(separator: "@", omittingEmptySubsequences: false).last ?? ""
        guard !hostPort.isEmpty else { return false }
        var host = hostPort
        var port: Substring?
        if hostPort.hasPrefix("[") {
            guard let end = hostPort.firstIndex(of: "]") else { return false }
            host = hostPort[...end]
            let tail = hostPort[hostPort.index(after: end)...]
            if !tail.isEmpty {
                guard tail.first == ":" else { return false }
                port = tail.dropFirst()
            }
        } else if let colon = hostPort.lastIndex(of: ":") {
            host = hostPort[..<colon]
            port = hostPort[hostPort.index(after: colon)...]
        }
        guard !host.isEmpty, !host.contains(where: { $0.isWhitespace || $0 == "\u{0}" }) else { return false }
        if let port, !port.isEmpty {
            guard port.allSatisfy({ $0.isASCII && $0.isNumber }), let n = UInt32(port), n <= 65535 else { return false }
        }
        if host.hasPrefix("[") {
            return URLComponents(string: "\(scheme)://\(host)")?.host != nil
        }
        // Special hosts are percent-decoded before forbidden-code-point and
        // IPv4 checks. Foundation rejects legal quotes/backticks/braces and
        // accepts malformed numeric hosts, so it cannot decide these cases.
        guard let decoded = String(host).removingPercentEncoding, !decoded.isEmpty,
              !decoded.unicodeScalars.contains(where: {
                  $0.value <= 0x20 || $0.value == 0x7f || "#%/:<>?@[\\]^|".unicodeScalars.contains($0)
              }) else { return false }
        var parts = decoded.split(separator: ".", omittingEmptySubsequences: false)
        if parts.last == "" { parts.removeLast() }
        guard let last = parts.last else { return false }
        let hexNumber = last.lowercased().hasPrefix("0x") && last.dropFirst(2).allSatisfy { $0.isASCII && $0.isHexDigit }
        let endsInNumber = hexNumber || ipv4Number(last) != nil || (!last.isEmpty && last.allSatisfy { $0.isASCII && $0.isNumber })
        if endsInNumber {
            guard parts.count <= 4 else { return false }
            let numbers = parts.compactMap(ipv4Number)
            guard numbers.count == parts.count, numbers.dropLast().allSatisfy({ $0 <= 255 }),
                  let final = numbers.last else { return false }
            return final < (UInt64(1) << (8 * (5 - numbers.count)))
        }
        // Foundation performs IDNA validation for ACE labels. Restrict this
        // check to punycode: it rejects legal quote/backtick/brace hosts.
        for part in parts where part.lowercased().hasPrefix("xn--") {
            guard URLComponents(string: "\(scheme)://\(part)")?.host != nil else { return false }
        }
        return true
    }

    /// WHATWG IPv4 numbers: decimal, leading-zero octal or 0x hex. Overflow
    /// refuses the URL instead of wrapping, including arbitrarily long input.
    private static func ipv4Number(_ text: Substring) -> UInt64? {
        guard !text.isEmpty else { return nil }
        var digits = text
        var radix = 10
        if digits.lowercased().hasPrefix("0x") { digits = digits.dropFirst(2); radix = 16 }
        else if digits.count >= 2, digits.first == "0" { digits = digits.dropFirst(); radix = 8 }
        if digits.isEmpty { return 0 }
        guard digits.utf8.allSatisfy({ byte in
            if byte >= 48 && byte <= 57 { return Int(byte - 48) < radix }
            if radix == 16 { return (65...70).contains(byte) || (97...102).contains(byte) }
            return false
        }) else { return nil }
        return UInt64(digits, radix: radix)
    }

}

/// The bits of JavaScript's conversions the rules lean on.
enum JS {
    /// `Number(x)` for a rule's argument.
    static func number(_ value: JSONValue?) -> Double {
        switch value {
        case nil: .nan
        case .null?: 0
        case let .bool(b)?: b ? 1 : 0
        case let .number(n)?: n
        case let .string(s)?: number(s) ?? .nan
        case let .array(items)?: items.isEmpty ? 0 : items.count == 1 ? number(string(items[0])) ?? .nan : .nan
        case .object?: .nan
        }
    }

    /// `Number(s)`: the whole string, trimmed, as a number; "" is 0.
    static func number(_ s: String) -> Double? {
        let trimmed = s.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return 0 }
        switch trimmed {
        case "Infinity", "+Infinity": return .infinity
        case "-Infinity": return -.infinity
        default: break
        }
        guard trimmed.allSatisfy({ $0.isASCII && ($0.isHexDigit || "+-.eExXoObB".contains($0)) }) else { return .nan }
        if let n = Double(trimmed) { return n }
        for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] where trimmed.hasPrefix(prefix) {
            return UInt64(trimmed.dropFirst(2), radix: radix).map(Double.init) ?? .nan
        }
        return .nan
    }

    /// `parseFloat(s)`: the longest decimal number at the start, after
    /// whitespace; NaN when there is none.
    static func parseFloat(_ s: String) -> Double {
        let text = Array(s.drop { $0.isWhitespace }.utf8)
        var i = 0
        if i < text.count, text[i] == UInt8(ascii: "+") || text[i] == UInt8(ascii: "-") { i += 1 }
        if text[i...].starts(with: Array("Infinity".utf8)) {
            return text.first == UInt8(ascii: "-") ? -.infinity : .infinity
        }
        func digits() -> Int {
            let start = i
            while i < text.count, (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(text[i]) { i += 1 }
            return i - start
        }
        var mantissa = digits()
        if i < text.count, text[i] == UInt8(ascii: ".") {
            i += 1
            mantissa += digits()
        }
        guard mantissa > 0 else { return .nan }
        var end = i
        if i < text.count, text[i] == UInt8(ascii: "e") || text[i] == UInt8(ascii: "E") {
            i += 1
            if i < text.count, text[i] == UInt8(ascii: "+") || text[i] == UInt8(ascii: "-") { i += 1 }
            if digits() > 0 { end = i }
        }
        return Double(String(decoding: text[..<end], as: UTF8.self)) ?? .nan
    }

    /// `String(x)`.
    static func string(_ value: JSONValue?) -> String {
        switch value {
        case nil: "undefined"
        case .null?: "null"
        case let .bool(b)?: b ? "true" : "false"
        case let .number(n)?: format(n)
        case let .string(s)?: s
        case let .array(items)?: items.map { $0 == .null ? "" : string($0) }.joined(separator: ",")
        case .object?: "[object Object]"
        }
    }

    /// A number as JavaScript prints it.
    static func format(_ n: Double) -> String {
        if n.isNaN { return "NaN" }
        if n.isInfinite { return n > 0 ? "Infinity" : "-Infinity" }
        return JSONValue.format(n)
    }
}

/// One form's validation errors, by field name, and the checks that set them.
@MainActor @Observable
final class FormValidation {
    private(set) var errors: [String: String] = [:]

    /// Selection belongs to the mounted native Tabs view, keyed by its items
    /// path. Without a recorded selection, an app's Tabs validates all content.
    var selectedTabs: [String: Int] = [:]

    private static let fieldTypes: Set<String> = [
        "Input", "Select", "TextArea", "RadioGroup", "CheckBoxGroup", "Slider",
        "DatePicker", "Chips", "OptionCards",
    ]

    /// Read the current enclosing form for a current, already checked click.
    func validate(enclosing element: Element, in live: LiveProgram, formName: String?) -> Bool {
        guard let form = live.enclosingForm(of: element) else { return true }
        var fields: [(String, Element, [ValidationRule])] = []
        func walk(_ value: PropValue) {
            switch value {
            case let .element(element):
                if element.typeName == "Form" { return }
                let props = PropReader(element: element)
                if element.typeName == "Tabs",
                   let selected = selectedTabs[ElementPath.child(of: element.path ?? "/", "items")] {
                    let items = (try? props.elements("items", of: "TabItem")) ?? []
                    if items.indices.contains(selected), let content = PropReader(element: items[selected].1).value("content") {
                        walk(content)
                    }
                    return
                }
                if Self.fieldTypes.contains(element.typeName) || live.validationComponents[element.typeName] != nil {
                    let rules = Validation.rules(props.value("rules"))
                    if !rules.isEmpty, let name = try? props.text("name").plain {
                        fields.removeAll { $0.0 == name }
                        fields.append((name, element, rules))
                    }
                }
                element.props.forEach { walk($0.value) }
            case let .list(items): items.forEach(walk)
            case let .record(fields): fields.forEach { walk($0.value) }
            default: break
            }
        }
        form.props.forEach { walk($0.value) }
        var found: [String: String] = [:]
        for (name, element, rules) in fields {
            let value: JSONValue?
            if let reader = live.validationComponents[element.typeName] {
                value = reader(element, live, formName)
            } else if element.typeName == "Chips" || element.typeName == "OptionCards" {
                value = SelectionField.validationValue(PropReader(element: element), live: live, form: formName, name: name)
            } else {
                // Input and Select must use the same default reader as their
                // native views: list/record props leave those controls empty.
                let compositeDefaults = element.typeName != "Input" && element.typeName != "Select"
                value = FieldValue(PropReader(element: element), name: name).value(in: live, form: formName, includingCompositeDefaults: compositeDefaults)
            }
            if let error = Validation.check(value, rules) { found[name] = error }
        }
        errors = found
        return found.isEmpty
    }

    /// Checks one field, as upstream does when an input loses focus or a
    /// select changes.
    func validate(_ name: String, value: JSONValue?, rules: [ValidationRule]) {
        let error = Validation.check(value, rules)
        if errors[name] != error { errors[name] = error }
    }

    func clear(_ name: String) {
        if errors[name] != nil { errors[name] = nil }
    }
}

/// The mounted form's errors and tab selections.
struct FormContext {
    let validation: FormValidation
}

extension EnvironmentValues {
    @Entry var formContext: FormContext? = nil
}
