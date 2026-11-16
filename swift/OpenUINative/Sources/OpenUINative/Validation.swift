import Foundation
import Observation
import SwiftUI

/// Structured rules follow react-ui 0.18.1 and lang-core 1.1.0. Regexes use
/// ICU with explicit ASCII digit and absolute end-anchor translations; URL
/// checks cover special-scheme authorities but are not a full WHATWG parser.
struct ValidationRule: Equatable {
    var type: String
    var arg: JSONValue?
}

enum Validation {
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
        var translated = ""
        var escaped = false
        var inClass = false
        for c in pattern {
            if escaped {
                switch c {
                case "d": translated += "[0-9]"
                case "D": translated += "[^0-9]"
                default: translated += "\\" + String(c)
                }
                escaped = false
            } else if c == "\\" {
                escaped = true
            } else {
                if c == "[" { inClass = true }
                if c == "]" { inClass = false }
                translated += c == "$" && !inClass ? "\\z" : String(c)
            }
        }
        if escaped { translated += "\\" }
        guard let regex = try? NSRegularExpression(pattern: translated) else { return nil }
        return regex.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
    }

    private static func isURL(_ s: String) -> Bool {
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
        return URLComponents(string: "\(scheme)://\(hostPort)")?.host != nil
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
                value = Self.selectionValue(element, live: live, form: formName, name: name)
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

    private static func selectionValue(_ element: Element, live: LiveProgram, form: String?, name: String) -> JSONValue? {
        let props = PropReader(element: element)
        let single = props.keyword("type", element.typeName == "Chips" ? "multiple" : "single") == "single"
        let defaultValue = props.value("defaultValue")?.json
        let normalized: JSONValue?
        if single {
            let text = defaultValue?.string ?? defaultValue?.array?.first?.string
            normalized = text.flatMap { $0.isEmpty ? nil : .string($0) }
        } else if let array = defaultValue?.array {
            normalized = .array(array.filter { $0.string != nil })
        } else if let text = defaultValue?.string {
            normalized = .array(text.isEmpty ? [] : [.string(text)])
        } else { normalized = nil }
        let stored = live.field(form: form, name: name)
        let value = stored == .null ? normalized : stored ?? normalized
        if single { return value?.string.flatMap { $0.isEmpty ? nil : .string($0) } }
        return value?.array.flatMap { $0.isEmpty ? nil : .array($0) }
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
