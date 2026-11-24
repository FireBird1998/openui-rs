import Foundation
import OpenUINative

public struct UserTurn: Identifiable {
    public let id = UUID()
    /// The content sent to the Gateway, including the markers for an action.
    public let rawText: String
    /// The text in the bubble.
    public let text: String
    let formData: FormData?

    init(rawText: String, text: String, parsed: ParsedMessage? = nil) {
        self.rawText = rawText
        self.text = parsed?.content ?? text
        formData = parsed.flatMap { FormData(context: $0.context) }
    }
}

/// The form summary shown by react-ui's GenUIUserMessage.
struct FormData {
    struct Field {
        let label: String
        let value: String
    }

    let fields: [Field]
    var title: String { "Form data (\(fields.count) \(fields.count == 1 ? "field" : "fields"))" }

    init?(context: JSONValue) {
        let state = context.array?.first { if case .object = $0 { true } else { false } } ?? context
        guard case let .object(entries) = state else { return nil }

        // A form-named payload wraps each group of fields in its own object.
        let nested = entries.contains { entry in
            entry.value["value"] == nil && Self.entries(of: entry.value).contains { $0.value["value"] != nil }
        }
        let candidates = nested ? entries.flatMap { Self.entries(of: $0.value) } : entries
        fields = candidates.compactMap { label, field in
            guard let value = field["value"], value != .null, value.string != "" else { return nil }
            return Field(label: label, value: Self.display(value))
        }
        if fields.isEmpty { return nil }
    }

    private static func entries(of value: JSONValue) -> [(key: String, value: JSONValue)] {
        switch value {
        case let .object(entries): entries
        case let .array(items): items.enumerated().map { (String($0.offset), $0.element) }
        default: []
        }
    }

    // Context has already passed the binding's size and depth checks.
    private static func display(_ value: JSONValue) -> String {
        switch value {
        case let .string(text): text
        case let .number(number): displayNumber(number)
        case let .array(items): items.map { $0 == .null ? "" : display($0) }.joined(separator: ",")
        case .object: "[object Object]"
        default: value.compact
        }
    }

    /// Swift and JavaScript choose different ranges for printing exponents.
    private static func displayNumber(_ number: Double) -> String {
        let text = JSONValue.format(number)
        let parts = text.split(separator: "e", maxSplits: 1)
        guard parts.count == 2, let mantissa = parts.first, let exponent = parts.last.flatMap({ Int($0) }) else {
            return text.hasSuffix(".0") ? String(text.dropLast(2)) : text
        }
        guard (-6..<21).contains(exponent) else {
            return "\(mantissa)e\(exponent >= 0 ? "+" : "")\(exponent)"
        }

        let sign = mantissa.hasPrefix("-") ? "-" : ""
        let digits = mantissa.dropFirst(sign.count).filter { $0 != "." }
        let point = exponent + 1
        if point <= 0 {
            return sign + "0." + String(repeating: "0", count: -point) + digits
        }
        if point >= digits.count {
            return sign + digits + String(repeating: "0", count: point - digits.count)
        }
        return sign + digits.prefix(point) + "." + digits.dropFirst(point)
    }
}
