// The four Cartesian branches of Intelligent UI's lib/Chart/index.tsx.
import OpenUINative
import SwiftUI

enum CartesianChartKind: String {
    case bar, line, area, scatter
}

struct CartesianChartData {
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

    struct Row {
        let id: Int
        let label: String
        let categoryIndex: Int
        let x: Double
        let values: [Int: Double]
        let cells: [Cell]
    }

    struct Cell {
        let series: Series
        let value: Double
    }

    struct Series {
        // The first dataset index is also the interned ID. Scatter duplicates
        // share a value but use the last dataset's paint and first one's hover.
        let id: Int
        let lastIndex: Int
    }

    struct Visibility {
        let first: [Series]
        let last: [Series]
        let ranks: [Int: Int]
        let yMagnitude: Magnitude

        func firstMissing(in row: Row) -> Series? {
            first.first { row.values[$0.id] == nil }
        }

        func lastMissing(in row: Row) -> Series? {
            last.reversed().first { row.values[$0.id] == nil }
        }
    }

    // Swift Charts converts numeric tick positions to Int. Keep its chart
    // coordinates on a workable scale and recover the original tick values.
    struct NumericTransform {
        let origin: Double
        let unit: Double

        init(peak: Double, origin: Double = 0) {
            self.origin = origin
            if peak > 1e9 || (peak > 0 && peak < 1e-9) {
                let exponent = max(-323, Int(floor(log10(peak))))
                unit = pow(10, Double(exponent))
            } else {
                unit = 1
            }
        }

        var isIdentity: Bool { origin == 0 && unit == 1 }
        func position(_ value: Double) -> Double { (value - origin) / unit }
        func value(at position: Double) -> Double { origin.addingProduct(position, unit) }
    }

    struct Magnitude {
        let transform: NumericTransform
        let domain: [Double]

        init(transform: NumericTransform, domain: [Double]) {
            self.transform = transform
            self.domain = domain
        }

        init(_ samples: some Sequence<Double>, includesZero: Bool = false) {
            var low = includesZero ? 0 : Double.infinity
            var high = includesZero ? 0 : -Double.infinity
            for sample in samples where sample.isFinite {
                low = min(low, sample)
                high = max(high, sample)
            }
            if low > high {
                low = 0
                high = 100
            }
            let peak = max(abs(low), abs(high))
            var transform = NumericTransform(peak: peak)
            // Subtract nearby same-sign endpoints before scaling so that
            // rounding the scaled coordinates cannot merge them. For a
            // mixed-sign range, scale about zero to avoid overflowing a span.
            if low != high, low.sign == high.sign {
                let span = high - low
                if span <= peak / 2 {
                    transform = NumericTransform(peak: span, origin: low)
                }
            }
            self.transform = transform
            if low == high {
                let center = transform.position(low)
                domain = [center - 0.5, center + 0.5]
            } else {
                domain = [transform.position(low), transform.position(high)]
            }
        }
    }

    let xMagnitude: Magnitude
    private let scatterYMagnitude: Magnitude
    let kind: CartesianChartKind
    let category: String
    let categories: [String]
    let legendKeys: [Name]
    let rows: [Row]
    let stacked: Bool
    let explicitHeight: CGFloat?

    private let paletteCount: Int
    let seriesByKey: [Name: Series]
    private let seriesByID: [Int: Name]
    private let firstSeries: [Series]
    private let lastSeries: [Series]
    private let firstRowsByCategory: [Int: Row]

    init?(_ element: Element, kind: CartesianChartKind) {
        guard case let .list(data)? = element.prop("data"),
              let row = data.lazy.compactMap(Self.fields).first else { return nil }
        let first = Self.objectKeys(row, key: \.name)
        let category = Self.category(element, first: first)
        guard !category.isEmpty else { return nil }

        let categoryKey = Name(category)
        var keys: [Name] = []
        if case let .list(list)? = element.prop("series") {
            keys = list.compactMap { $0.ruleScalar?.string.map(Name.init) }
        }
        if keys.isEmpty {
            keys = first.compactMap { field in
                let name = Name(field.name)
                return name != categoryKey && field.value.ruleScalar?.number != nil ? name : nil
            }
        }
        // narrow writes one object property per key. Assigning __proto__ to a
        // number does not create a property, unless the computed category did.
        var seen = Set<Name>()
        let names = kind == .scatter ? keys : Self.objectKeys(keys.filter {
            seen.insert($0).inserted && $0 != categoryKey && $0.text != "__proto__"
        }, key: \.text)
        legendKeys = names
        paletteCount = max(1, keys.count)
        var seriesByKey: [Name: Series] = [:]
        for (index, key) in names.enumerated() {
            seriesByKey[key] = Series(id: seriesByKey[key]?.id ?? index, lastIndex: index)
        }
        self.seriesByKey = seriesByKey
        seriesByID = Dictionary(uniqueKeysWithValues: seriesByKey.map { ($0.value.id, $0.key) })
        firstSeries = seriesByKey.values.sorted { $0.id < $1.id }
        lastSeries = kind == .scatter ? seriesByKey.values.sorted { $0.lastIndex < $1.lastIndex } : firstSeries

        self.kind = kind
        self.category = category
        stacked = (kind == .bar || kind == .area) && element.prop("stacked")?.ruleScalar == .bool(true)
        explicitHeight = element.prop("height")?.ruleScalar?.number.map { layoutPoints($0) }
        let narrowedCategory = kind != .scatter && seen.contains(categoryKey)
        var labels: [String] = []
        var labelIndices: [Name: Int] = [:]
        var firstRows: [Int: Row] = [:]
        var rows: [Row] = []
        var minimumX = Double.infinity
        for item in data {
            guard let fields = Self.fields(item) else { continue }
            var label = narrowedCategory ? "0" : ""
            var x = 0.0
            var present: [Int: Cell] = [:]
            for field in fields {
                let value = field.value.ruleScalar
                let key = Name(field.name)
                if key == categoryKey {
                    if kind == .scatter {
                        x = Self.scatterNumber(value)
                    } else if narrowedCategory {
                        label = JavaScript.numberString(value?.number ?? 0)
                    } else {
                        label = value?.string ?? value?.number.map(JavaScript.numberString) ?? ""
                    }
                }
                if let series = seriesByKey[key] {
                    let number = kind == .scatter ? Self.scatterNumber(value) : value?.number ?? 0
                    present[series.id] = Cell(series: series, value: number.isFinite ? number : 0)
                }
            }
            // NaN never wins this comparison. Negative infinity wins even
            // when some of the other scatter rows have finite coordinates.
            if x < minimumX { minimumX = x }
            let categoryIndex: Int
            if kind == .scatter {
                categoryIndex = 0
            } else if let index = labelIndices[Name(label)] {
                categoryIndex = index
            } else {
                categoryIndex = labels.count
                labelIndices[Name(label)] = categoryIndex
                labels.append(label)
            }
            let cells = present.values.sorted { $0.series.lastIndex < $1.series.lastIndex }
            let values = present.mapValues(\.value)
            let row = Row(id: rows.count, label: label, categoryIndex: categoryIndex, x: x,
                          values: values, cells: cells)
            if kind != .scatter, firstRows[categoryIndex] == nil { firstRows[categoryIndex] = row }
            rows.append(row)
        }

        self.rows = rows
        categories = labels
        firstRowsByCategory = firstRows
        if kind != .scatter || seriesByKey.isEmpty || !minimumX.isFinite {
            xMagnitude = Magnitude([0, 100])
            scatterYMagnitude = xMagnitude
        } else {
            xMagnitude = Magnitude(rows.lazy.map(\.x))
            scatterYMagnitude = Magnitude(rows.lazy.flatMap { $0.values.values },
                                          includesZero: rows.contains { $0.values.count < seriesByKey.count })
        }
    }

    func scatterCoordinates(x: Double, y: Double) -> (x: Double, y: Double) {
        (xMagnitude.transform.position(x), scatterYMagnitude.transform.position(y))
    }

    func row(forCategory index: Int) -> Row? { firstRowsByCategory[index] }

    func name(forSeriesID id: Int) -> String? { seriesByID[id]?.text }

    func liveHiddenKeys(_ hidden: Set<Name>) -> Set<Name> {
        hidden.filter { seriesByKey[$0] != nil }
    }

    // The wrapper rotates the raw palette before the components distribute
    // colours across their final keys. Narrowing can change the two counts.
    func colourIndex(forSeriesIndex index: Int) -> Int {
        let middle = paletteCount / 2
        let distributed: Int
        switch legendKeys.count {
        case 1: distributed = middle
        case 2: distributed = index == 0 ? max(0, middle - 1) : min(paletteCount - 1, middle + 1)
        default: distributed = (middle + index - (legendKeys.count - 1) / 2 + paletteCount) % paletteCount
        }
        if paletteCount > 2 && paletteCount.isMultiple(of: 2) {
            return (distributed + paletteCount - 1) % paletteCount
        }
        return distributed
    }

    func visibility(hiding hidden: Set<Name>) -> Visibility {
        let hiddenIDs = Set(hidden.compactMap { seriesByKey[$0]?.id })
        let first = firstSeries.filter { !hiddenIDs.contains($0.id) }
        let last = kind == .scatter ? lastSeries.filter { !hiddenIDs.contains($0.id) } : first
        let ranks = Dictionary(uniqueKeysWithValues: last.enumerated().map { ($0.element.id, $0.offset) })
        let yMagnitude = kind == .scatter ? scatterYMagnitude : categoricalYMagnitude(ranks: ranks)
        return Visibility(first: first, last: last, ranks: ranks, yMagnitude: yMagnitude)
    }

    // Bars diverge by sign; areas keep a single running total.
    struct Stack {
        var positive = 0.0
        var negative = 0.0

        mutating func adding(_ value: Double, diverges: Bool) -> (start: Double, end: Double) {
            if diverges && value < 0 {
                let end = negative
                negative += value
                return (negative, end)
            }
            let start = positive
            positive += value
            return (start, positive)
        }
    }

    private func categoricalYMagnitude(ranks: [Int: Int]) -> Magnitude {
        var low = 0.0, high = 0.0
        for row in rows {
            for cell in row.cells where ranks[cell.series.id] != nil {
                low = min(low, cell.value)
                high = max(high, cell.value)
            }
        }
        let transform = NumericTransform(peak: max(-low, high))
        guard stacked else {
            return Magnitude(transform: transform, domain: [transform.position(low), transform.position(high)])
        }
        low = 0
        high = 0
        for row in rows {
            var stack = Stack()
            for cell in row.cells where ranks[cell.series.id] != nil {
                let (start, end) = stack.adding(transform.position(cell.value), diverges: kind == .bar)
                low = min(low, start, end)
                high = max(high, start, end)
            }
        }
        return Magnitude(transform: transform, domain: [low, high])
    }

    private typealias Field = (name: String, value: PropValue)

    private static func fields(_ row: PropValue) -> [Field]? {
        switch row {
        case let .record(fields): fields.map { ($0.name, $0.value) }
        case let .list(items): items.enumerated().map { (String($0.offset), $0.element) }
        default: nil
        }
    }

    private static func category(_ element: Element, first: [Field]) -> String {
        if let x = element.prop("x")?.ruleScalar?.string, !x.isEmpty { return x }
        return first.first { $0.value.ruleScalar?.string != nil }?.name ?? first.first?.name ?? ""
    }

    // A null React child contributes no sibling margin. Only the first object
    // is needed to answer that question while the parent builds its layout.
    static func hasNoRowsOrCategory(_ element: Element) -> Bool {
        guard CartesianChartKind(rawValue: element.prop("type")?.ruleScalar?.string ?? "") != nil else { return false }
        guard case let .list(data)? = element.prop("data"),
              let first = data.lazy.compactMap(fields).first else { return true }
        return category(element, first: objectKeys(first, key: \.name)).isEmpty
    }

    // Object.keys lists canonical array indices first, then names in insertion order.
    private static func objectKeys<T>(_ fields: [T], key: KeyPath<T, String>) -> [T] {
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

    // Number(cell ?? 0) for the schema's numbers and strings. The series
    // value rule also replaces failed conversions with zero during normalization.
    private static func scatterNumber(_ cell: JSONValue?) -> Double {
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
                if head <= UInt64.max >> bits {
                    head = (head << bits) | UInt64(digit)
                } else {
                    shift += bits
                    if shift > 1024 { return .infinity }
                    sticky = sticky || digit != 0
                }
            }
            let number = Double(sign: .plus, exponent: shift, significand: Double(sticky ? head | 1 : head))
            return text.utf8.count > 2 ? number : .nan
        }
        guard text.utf8.allSatisfy({ (48...57).contains($0) || "+-.Ee".utf8.contains($0) }),
              let number = Double(text) else { return .nan }
        return number
    }
}
