// The four Cartesian branches of Intelligent UI's lib/Chart/index.tsx.
import OpenUINative
import SwiftUI

enum CartesianChartKind: String {
    case bar, line, area, scatter
}

struct CartesianChartData {
    typealias Name = ChartRows.Name

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
              let row = data.lazy.compactMap(ChartRows.fields).first else { return nil }
        let first = ChartRows.objectKeys(row, key: \.name)
        let category = ChartRows.category(element, first: first)
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
        let names = kind == .scatter ? keys : ChartRows.objectKeys(keys.filter {
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
            guard let fields = ChartRows.fields(item) else { continue }
            var label = narrowedCategory ? "0" : ""
            var x = 0.0
            var present: [Int: Cell] = [:]
            for field in fields {
                let value = field.value.ruleScalar
                let key = Name(field.name)
                if key == categoryKey {
                    if kind == .scatter {
                        x = ChartRows.number(value)
                    } else if narrowedCategory {
                        label = JavaScript.numberString(value?.number ?? 0)
                    } else {
                        label = value?.string ?? value?.number.map(JavaScript.numberString) ?? ""
                    }
                }
                if let series = seriesByKey[key] {
                    let number = kind == .scatter ? ChartRows.number(value) : value?.number ?? 0
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

    func colourIndex(forSeriesIndex index: Int) -> Int {
        ChartPalette.index(index, paletteCount: paletteCount, itemCount: legendKeys.count)
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
}
