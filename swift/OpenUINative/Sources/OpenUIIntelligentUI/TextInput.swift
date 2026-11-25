// The value and validation rules shared by upstream's Input and TextArea.
import OpenUINative
import SwiftUI

@MainActor
struct TextInput {
    let element: Element
    let name: String
    let label: String?
    let placeholder: String
    let rows: Int?
    let numeric: Bool
    private let rules: [ValidationRule]

    init(_ element: Element, path: String) {
        self.element = element
        let label = element.prop("label")?.ruleScalar?.string
        self.label = label.flatMap { $0.isEmpty ? nil : $0 }
        name = element.statementId ?? self.label ?? path
        placeholder = element.prop("placeholder")?.ruleScalar?.string ?? ""
        numeric = element.typeName == "Input" && element.prop("type")?.ruleScalar?.string == "number"
        if element.typeName == "TextArea" {
            let count = element.prop("rows")?.ruleScalar?.number ?? 3
            rows = count.isFinite ? Int(min(20, max(2, JavaScript.round(count)))) : 3
        } else {
            rows = nil
        }
        rules = Validation.rules(element.prop("rules"))
    }

    func isEnabled(in live: LiveProgram?) -> Bool { live?.acceptsInteractions == true }

    func isRequired(in validation: FormValidation?) -> Bool {
        validation != nil && rules.contains { $0.type == "required" }
    }

    func error(in validation: FormValidation?) -> String? {
        rules.isEmpty ? nil : validation?.errors[name]
    }

    func value(in live: LiveProgram?, form: String?) -> JSONValue? {
        if case let .binding(marker)? = element.prop("value") { return live?.value(of: marker.target) }
        if let stored = live?.field(form: form, name: name), stored != .null { return stored }
        return element.prop("value")?.json
    }

    func text(in live: LiveProgram?, form: String?) -> String {
        guard let value = value(in: live, form: form) else { return "" }
        return (try? ListFunctions.text(value)) ?? ""
    }

    func validate(in live: LiveProgram?, form: String?, validation: FormValidation?) {
        guard !rules.isEmpty, isEnabled(in: live) else { return }
        validation?.validate(name, value: value(in: live, form: form), rules: rules)
    }

    /// An invalid numeric draft is a successful edit with no store write.
    /// A refused edit leaves both the committed display and validation alone.
    func edit(_ text: String, in live: LiveProgram?, form: String?, validation: FormValidation?) -> Bool {
        guard let live, isEnabled(in: live), live.current(element, form: form) != nil,
              text.utf8.count <= defaultHostByteLimit else { return false }
        let write: JSONValue?
        if numeric {
            write = Self.numberValue(text)
        } else {
            write = .string(text)
        }
        if let write {
            guard write.compact(byteLimit: defaultHostByteLimit) != nil else { return false }
            do {
                if case let .binding(marker)? = element.prop("value") {
                    try live.write(marker, write)
                } else {
                    try live.setField(form: form, name: name, to: write)
                }
            } catch { return false }
        }
        if !rules.isEmpty { validation?.clear(name) }
        return true
    }

    private static func numberValue(_ text: String) -> JSONValue? {
        if JavaScript.trim(text).isEmpty { return .null }
        let withoutCommas = text.replacingOccurrences(of: ",", with: "", options: .literal)
        let trimmed = JavaScript.trim(withoutCommas)
        if trimmed.isEmpty { return .number(0) }
        let number: Double?
        if trimmed.starts(with: [0x30, 0x78]) || trimmed.starts(with: [0x30, 0x58]) {
            number = radixNumber(trimmed.dropFirst(2), bitsPerDigit: 4)
        } else if trimmed.starts(with: [0x30, 0x6f]) || trimmed.starts(with: [0x30, 0x4f]) {
            number = radixNumber(trimmed.dropFirst(2), bitsPerDigit: 3)
        } else if trimmed.starts(with: [0x30, 0x62]) || trimmed.starts(with: [0x30, 0x42]) {
            number = radixNumber(trimmed.dropFirst(2), bitsPerDigit: 1)
        } else {
            number = trimmed.allSatisfy { (0x30...0x39).contains($0) || "+-.eE".utf16.contains($0) }
                ? Double(String(decoding: trimmed, as: UTF16.self)) : nil
        }
        guard let number, number.isFinite else { return nil }
        return .number(number)
    }

    /// Convert unsigned binary, octal and hex as Number does. Keep the leading
    /// 54 bits and a sticky bit so Double rounds correctly even beyond UInt64.
    private static func radixNumber(_ digits: Substring.UTF16View, bitsPerDigit: Int) -> Double? {
        guard !digits.isEmpty else { return nil }
        var leading: UInt64 = 0
        var width = 0
        var sticky = false
        for unit in digits {
            let digit: UInt64
            switch unit {
            case 0x30...0x39: digit = UInt64(unit - 0x30)
            case 0x41...0x46: digit = UInt64(unit - 0x41 + 10)
            case 0x61...0x66: digit = UInt64(unit - 0x61 + 10)
            default: return nil
            }
            guard digit < 1 << bitsPerDigit else { return nil }
            if width == 0 {
                width = UInt64.bitWidth - digit.leadingZeroBitCount
                leading = digit
            } else if width + bitsPerDigit <= 54 {
                leading = (leading << bitsPerDigit) | digit
                width += bitsPerDigit
            } else {
                let room = max(0, 54 - width)
                let excess = bitsPerDigit - room
                leading = (leading << room) | (digit >> excess)
                sticky = sticky || digit & ((1 << excess) - 1) != 0
                width += bitsPerDigit
                if width > 1024 { return nil }
            }
        }
        guard width >= 54 else { return Double(leading) }
        let mantissa = leading >> 1
        let roundUp = leading & 1 == 1 && (sticky || mantissa & 1 == 1)
        return Double(sign: .plus, exponent: width - 53, significand: Double(mantissa + (roundUp ? 1 : 0)))
    }
}

/// Mounts the inputs with their field scope and blur validation context.
struct IntelligentFormScope: View {
    let element: Element
    let path: String
    @State private var validation = FormValidation()

    static func name(_ element: Element) -> String? { element.statementId ?? element.path }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(element.layoutChildren) { $0.draw(in: path) }
        }
        .environment(\.formName, Self.name(element) ?? path)
        .environment(\.formContext, FormContext(validation: validation))
    }
}
