import Foundation
import OpenUINative

enum PolarChartKind: String {
    case pie, donut
}

struct PolarChartData {
    typealias Label = ChartRows.Name

    struct Slice: Identifiable, Equatable {
        let id: Int
        let label: Label
        let value: Double
        let colourIndex: Int
    }

    let slices: [Slice]
    let isPercentage: Bool

    init?(_ element: Element) {
        guard case let .list(data)? = element.prop("data"),
              let row = data.lazy.compactMap(ChartRows.fields).first else { return nil }
        let first = ChartRows.objectKeys(row, key: \.name)
        let category = Label(ChartRows.category(element, first: first))
        guard !category.text.isEmpty else { return nil }

        let series: Label
        if case let .list(keys)? = element.prop("series"),
           let key = keys.lazy.compactMap({ $0.ruleScalar?.string }).first {
            series = Label(key)
        } else {
            series = Label(first.first {
                Label($0.name) != category && $0.value.ruleScalar?.number != nil
            }?.name ?? "")
        }

        let sorted = data.lazy.compactMap { item -> (label: Label, value: Double)? in
            guard let fields = ChartRows.fields(item) else { return nil }
            var categoryCell: JSONValue?
            var valueCell: JSONValue?
            for field in fields {
                let key = Label(field.name)
                if key == category { categoryCell = field.value.ruleScalar }
                if key == series { valueCell = field.value.ruleScalar }
            }
            let label = categoryCell?.string ?? categoryCell?.number.map(JavaScript.numberString)
                ?? categoryCell?.bool.map(String.init) ?? (categoryCell == .null ? "null" : "undefined")
            let value = ChartRows.number(valueCell)
            return (Label(label), value == 0 || value.isNaN ? 0 : value)
        }.sorted { $0.value > $1.value }
        slices = sorted.enumerated().map { index, row in
            // Both the input palette and distributed slice list have the same
            // count here. The upstream wrapper's rotation leaves them in order.
            Slice(id: index, label: row.label, value: row.value, colourIndex: index % 6)
        }
        isPercentage = element.prop("format")?.ruleScalar?.string == "percent"
    }

    func liveHiddenKeys(_ hidden: Set<Label>) -> Set<Label> {
        guard !hidden.isEmpty else { return [] }
        let kept = Set(slices.lazy.map(\.label).filter { hidden.contains($0) })
        return kept.count < slices.count ? kept : []
    }

    func toggling(_ label: Label, in hidden: Set<Label>) -> Set<Label> {
        var hidden = hidden
        // The upstream guard counts rows minus hidden labels. Repeated
        // categories consequently toggle together, even when all rows match.
        if hidden.remove(label) == nil, slices.count - hidden.count > 1 {
            hidden.insert(label)
        }
        return hidden
    }

    func visibleTotal(hiding hidden: Set<Label>) -> Double {
        slices.reduce(0) { hidden.contains($1.label) ? $0 : $0 + $1.value }
    }

    func legendValue(_ slice: Slice, total: Double) -> String {
        Self.fixedPercent(total > 0 ? (slice.value / total) * 100 : 0)
    }

    func tooltipValue(_ slice: Slice, total: Double, locale: Locale) -> String {
        if isPercentage {
            return total == 0 ? "0%" : Self.fixedPercent((slice.value / total) * 100)
        }
        var magnitude = abs(slice.value)
        let sign = slice.value < 0 ? "-" : ""
        if magnitude < 100_000 {
            return sign + magnitude.formatted(
                .number.precision(.fractionLength(0...3)).rounded(rule: .toNearestOrAwayFromZero).locale(locale)
            )
        }
        var unit = ""
        for suffix in ["K", "M", "B", "T"] where magnitude >= 1_000 {
            magnitude /= 1_000
            unit = suffix
        }
        return sign + JavaScript.numberString(floor(magnitude * 10) / 10) + unit
    }

    private static func fixedPercent(_ value: Double) -> String {
        let magnitude = abs(value)
        guard value.isFinite, magnitude < 1e21 else {
            return JavaScript.numberString(value) + "%"
        }
        // At one decimal place, .25 is the exactly representable tie
        // where libc rounds toward even and JavaScript rounds away from zero.
        let whole = floor(magnitude)
        if magnitude - whole == 0.25 {
            return (value < 0 ? "-" : "") + String(format: "%.0f", whole) + ".3%"
        }
        return String(format: "%.1f", value == 0 ? 0 : value) + "%"
    }
}
