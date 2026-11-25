import OpenUINative
import SwiftUI

enum ChartRows {
    // JavaScript compares keys and category labels by code units. Swift
    // String equality also merges canonically equivalent spellings.
    struct Name: Hashable {
        let text: String

        init(_ text: String) { self.text = text }

        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.text.utf8.elementsEqual(rhs.text.utf8)
        }

        func hash(into hasher: inout Hasher) {
            for byte in text.utf8 { hasher.combine(byte) }
        }
    }

    // A null React child contributes no sibling margin. Only the first object
    // is needed to answer that question while the parent builds its layout.
    static func hasNoRowsOrCategory(_ element: Element) -> Bool {
        let type = element.prop("type")?.ruleScalar?.string ?? ""
        guard CartesianChartKind(rawValue: type) != nil || PolarChartKind(rawValue: type) != nil else {
            return false
        }
        guard case let .list(data)? = element.prop("data"),
              let first = data.lazy.compactMap(fields).first else { return true }
        return category(element, first: objectKeys(first, key: \.name)).isEmpty
    }

    typealias Field = (name: String, value: PropValue)

    static func fields(_ row: PropValue) -> [Field]? {
        switch row {
        case let .record(fields): fields.map { ($0.name, $0.value) }
        case let .list(items): items.enumerated().map { (String($0.offset), $0.element) }
        default: nil
        }
    }

    static func category(_ element: Element, first: [Field]) -> String {
        if let x = element.prop("x")?.ruleScalar?.string, !x.isEmpty { return x }
        return first.first { $0.value.ruleScalar?.string != nil }?.name ?? first.first?.name ?? ""
    }

    // Object.keys lists canonical array indices first, then names in insertion order.
    static func objectKeys<T>(_ fields: [T], key: KeyPath<T, String>) -> [T] {
        var indices: [(UInt32, T)] = []
        var names: [T] = []
        for field in fields {
            let name = field[keyPath: key]
            if let index = UInt32(name), index < UInt32.max, String(index) == name {
                indices.append((index, field))
            } else {
                names.append(field)
            }
        }
        return indices.sorted { $0.0 < $1.0 }.map(\.1) + names
    }

    // Number(cell) for the chart schema's numbers and strings. A missing
    // cell is already treated as zero by both callers.
    static func number(_ cell: JSONValue?) -> Double {
        if let number = cell?.number { return number }
        if let boolean = cell?.bool { return boolean ? 1 : 0 }
        guard let string = cell?.string else { return 0 }
        let text = String(decoding: JavaScript.trim(string), as: UTF16.self)
        if text.isEmpty { return 0 }
        if text == "Infinity" || text == "+Infinity" { return .infinity }
        if text == "-Infinity" { return -.infinity }
        for (prefix, bits) in [("0x", 4), ("0o", 3), ("0b", 1)]
            where text.prefix(2).lowercased() == prefix {
            // Keep more than 53 significant bits, plus a sticky bit for
            // discarded digits. Converting each prefix to Double rounds twice.
            var head: UInt64 = 0
            var shift = 0
            var sticky = false
            for byte in text.utf8.dropFirst(2) {
                guard let digit = Character(UnicodeScalar(byte)).hexDigitValue, digit < (1 << bits) else { return .nan }
                if shift > 1024 { continue }
                if head <= UInt64.max >> bits {
                    head = (head << bits) | UInt64(digit)
                } else {
                    shift += bits
                    sticky = sticky || digit != 0
                }
            }
            if shift > 1024 { return .infinity }
            let number = Double(sign: .plus, exponent: shift, significand: Double(sticky ? head | 1 : head))
            return text.utf8.count > 2 ? number : .nan
        }
        guard text.utf8.allSatisfy({ (48...57).contains($0) || "+-.Ee".utf8.contains($0) }),
              let number = Double(text) else { return .nan }
        return number
    }
}

enum ChartPalette {
    // The wrapper rotates the raw palette before the components distribute
    // colours across their final keys. Narrowing can change the two counts.
    static func index(_ index: Int, paletteCount: Int, itemCount: Int) -> Int {
        let middle = paletteCount / 2
        let distributed: Int
        switch itemCount {
        case 1: distributed = middle
        case 2: distributed = index == 0 ? max(0, middle - 1) : min(paletteCount - 1, middle + 1)
        default: distributed = (middle + index - (itemCount - 1) / 2 + paletteCount) % paletteCount
        }
        if paletteCount > 2 && paletteCount.isMultiple(of: 2) {
            return (distributed + paletteCount - 1) % paletteCount
        }
        return distributed
    }

    static func colour(_ index: Int, in theme: IntelligentTheme) -> Color {
        token(index, in: theme).color
    }

    static func token(_ index: Int, in theme: IntelligentTheme) -> ThemeColor {
        switch index % 6 {
        case 0: theme.chart1
        case 1: theme.chart2
        case 2: theme.chart3
        case 3: theme.chart4
        case 4: theme.chart5
        default: theme.chart6
        }
    }
}
