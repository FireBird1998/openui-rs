import Foundation
import Observation
import SwiftUI

/// Form validation as upstream's renderer does it (react-lang 1.1.0's
/// `useCreateFormValidation`, react-ui's Input, Select, FormControl and
/// Button, lang-core's `utils/validation.ts`): a Form keeps one error per
/// field; a primary button in it checks every field's rules before its
/// action and runs nothing while one fails; FormControl shows a field's error
/// in place of its hint.
///
/// What a person can't see the same way natively: upstream's patterns are
/// JavaScript regular expressions and these are ICU's (`NSRegularExpression`),
/// which agree on the common syntax but not every corner; `url` stands in
/// for the WHATWG URL parser with a scheme-and-host check. The fields checked
/// are the Inputs and Selects found in the form's evaluated tree, drawn or not
/// (upstream checks the fields that are mounted, which leaves out a field in
/// a tab that isn't showing); this package draws no TextArea, Slider,
/// DatePicker or option groups, so their rules aren't checked.
struct ValidationRule: Equatable {
    var type: String
    var arg: JSONValue?
}

enum Validation {
    /// A field's rules: upstream's structured object (`{required: true,
    /// minLength: 3}`, react-ui's `rules` prop; false and null skip a rule),
    /// or lang-core's list of rule strings (`["required", "min:3"]`, the
    /// older libraries' form). Anything else has no rules.
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
        case let .list(items)?:
            return items.compactMap { item in
                guard case let .scalar(.string(rule)) = item else { return nil }
                guard let colon = rule.firstIndex(of: ":") else { return ValidationRule(type: rule) }
                let type = String(rule[..<colon])
                let raw = String(rule[rule.index(after: colon)...])
                if ["min", "max", "minLength", "maxLength"].contains(type), let n = JS.number(raw), !n.isNaN {
                    return ValidationRule(type: type, arg: .number(n))
                }
                return ValidationRule(type: type, arg: .string(raw))
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
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        return regex.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
    }

    /// `new URL(s)` succeeds: a scheme, then, for the schemes that need one, a host.
    private static func isURL(_ s: String) -> Bool {
        let trimmed = s.trimmingCharacters(in: .whitespacesAndNewlines.union(.controlCharacters))
        guard let colon = trimmed.firstIndex(of: ":"),
              trimmed[..<colon].first?.isLetter == true,
              trimmed[..<colon].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "+-.".contains($0)) })
        else { return false }
        let scheme = trimmed[..<colon].lowercased()
        guard ["http", "https", "ws", "wss", "ftp"].contains(scheme) else { return true }
        let rest = trimmed[trimmed.index(after: colon)...].drop { $0 == "/" || $0 == "\\" }
        let host = rest.prefix { !"/\\?#".contains($0) }.split(separator: "@").last ?? ""
        return !host.isEmpty && !host.hasPrefix(":") && !host.contains(" ")
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

    /// The fields upstream would check in `form`: every Input and Select
    /// with rules under it, not those of a form nested in it. A later field
    /// with the same name replaces an earlier one, as upstream's registry does.
    static func fields(of form: Element) -> [(name: String, field: FieldValue, rules: [ValidationRule])] {
        var found: [(name: String, field: FieldValue, rules: [ValidationRule])] = []
        func walk(_ value: PropValue) {
            switch value {
            case let .element(element):
                if element.typeName == "Form" { return }
                visit(element)
            case let .list(items): items.forEach(walk)
            case let .record(fields): fields.forEach { walk($0.value) }
            case .expression, .binding, .plan, .scalar: break
            }
        }
        func visit(_ element: Element) {
            if element.typeName == "Input" || element.typeName == "Select" {
                let props = PropReader(element: element)
                let rules = Validation.rules(props.value("rules"))
                if !rules.isEmpty, let name = try? props.text("name").plain {
                    found.removeAll { $0.name == name }
                    found.append((name, FieldValue(props, name: name), rules))
                }
            }
            element.props.forEach { walk($0.value) }
        }
        form.props.forEach { walk($0.value) }
        return found
    }

    /// Checks every field of `form` against its rules, replacing every error;
    /// true when none fails.
    func validate(_ form: Element, in live: LiveProgram, formName: String?) -> Bool {
        var found: [String: String] = [:]
        for (name, field, rules) in Self.fields(of: form) {
            if let error = Validation.check(field.value(in: live, form: formName), rules) { found[name] = error }
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

/// The form a view sits in: its element, for the fields a primary button
/// checks, and its errors.
struct FormContext {
    let element: Element
    let validation: FormValidation
}

extension EnvironmentValues {
    @Entry var formContext: FormContext? = nil
}
