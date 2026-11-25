// A drawing contains only present bars/points and the edges needed to show
// implicit zeros. In particular, a run of missing stacked area series shares
// one zero-width band edge until a present value changes its running total.
import SwiftUI

struct CartesianChartGeometry {
    struct Bar {
        let category: Double
        let rank: Int
        let colour: Int
        let start: Double
        let end: Double
    }

    struct Dot {
        let point: ScatterCoordinate
        let series: CartesianChartData.Series
        let rowID: Int
    }

    struct Cubic: Equatable {
        let left: CGPoint
        let control1: CGPoint
        let control2: CGPoint
        let right: CGPoint

        init(left: CGPoint, right: CGPoint, before: CGPoint?, after: CGPoint?,
             verticalBefore: Int, verticalAfter: Int) {
            self.left = left
            self.right = right
            let width = right.x - left.x
            let dx = width / 3
            let secant = width == 0 ? 0 : (right.y - left.y) / width
            // A vertical neighbor has an infinite slope. d3 limits it to
            // twice the secant if both steps travel in the same Y direction.
            let direction = right.y > left.y ? 1 : -1
            let leftSlope = verticalBefore == 0 ? before.map { Self.tangent($0, left, right) }
                : (verticalBefore == direction ? 2 * secant : 0)
            let rightSlope = verticalAfter == 0 ? after.map { Self.tangent(left, right, $0) }
                : (verticalAfter == direction ? 2 * secant : 0)
            let start = leftSlope ?? rightSlope.map { (3 * secant - $0) / 2 } ?? secant
            let end = rightSlope ?? leftSlope.map { (3 * secant - $0) / 2 } ?? secant
            control1 = CGPoint(x: left.x + dx, y: left.y + dx * start)
            control2 = CGPoint(x: right.x - dx, y: right.y - dx * end)
        }

        // d3 curveMonotoneX uses Steffen's limited interior slope.
        private static func tangent(_ a: CGPoint, _ b: CGPoint, _ c: CGPoint) -> CGFloat {
            let h0 = b.x - a.x, h1 = c.x - b.x
            let s0 = (b.y - a.y) / h0, s1 = (c.y - b.y) / h1
            let weighted = (s0 * h1 + s1 * h0) / (h0 + h1)
            if weighted.isNaN { return 0 }
            let sign0 = s0 < 0 ? -1.0 : 1.0, sign1 = s1 < 0 ? -1.0 : 1.0
            return (sign0 + sign1) * min(abs(s0), abs(s1), abs(weighted) / 2)
        }
    }

    struct Edge {
        let stroke: Cubic
        let baseline: Cubic?
    }

    let bars: [Bar]
    let dots: [Dot]
    let curves: [Int: [Edge]]

    init(data: CartesianChartData, visible: CartesianChartData.Visibility) {
        switch data.kind {
        case .bar:
            bars = Self.bars(data, visible)
            dots = []
            curves = [:]
        case .scatter:
            bars = []
            dots = Self.dots(data, visible)
            curves = [:]
        case .line, .area:
            bars = []
            dots = []
            curves = Self.curves(data, visible)
        }
    }

    private static func bars(_ data: CartesianChartData,
                             _ visible: CartesianChartData.Visibility) -> [Bar] {
        var bars: [Bar] = []
        var stacks: [Int: [Bar]] = [:]
        let transform = visible.yMagnitude.transform
        for row in data.rows {
            var stack = CartesianChartData.Stack()
            for cell in row.cells where cell.value != 0 {
                guard let rank = visible.ranks[cell.series.id] else { continue }
                let value = transform.position(cell.value)
                let (start, end) = data.stacked ? stack.adding(value, diverges: true) : (0, value)
                let bar = Bar(category: Double(row.categoryIndex), rank: rank,
                              colour: cell.series.lastIndex, start: start, end: end)
                if data.stacked {
                    stacks[rank, default: []].append(bar)
                } else {
                    bars.append(bar)
                }
            }
        }
        return data.stacked ? visible.last.indices.flatMap { stacks[$0] ?? [] } : bars
    }

    private static func dots(_ data: CartesianChartData,
                             _ visible: CartesianChartData.Visibility) -> [Dot] {
        var byCoordinate: [ScatterCoordinate: Dot] = [:]
        for row in data.rows where row.x.isFinite {
            func offer(_ series: CartesianChartData.Series, _ y: Double) {
                let coordinate = ScatterCoordinate(x: row.x, y: y)
                if let previous = byCoordinate[coordinate], previous.series.lastIndex > series.lastIndex { return }
                byCoordinate[coordinate] = Dot(point: coordinate, series: series, rowID: row.id)
            }
            if let missing = visible.lastMissing(in: row) { offer(missing, 0) }
            for cell in row.cells where visible.ranks[cell.series.id] != nil {
                offer(cell.series, cell.value)
            }
        }
        return byCoordinate.values.sorted {
            ($0.series.lastIndex, $0.rowID) < ($1.series.lastIndex, $1.rowID)
        }
    }

    private typealias Row = CartesianChartData.Row
    private typealias Cell = CartesianChartData.Cell
    private typealias Visibility = CartesianChartData.Visibility

    // Sweep series ranks in order. A missing rank preserves the subtotal for
    // a stack and contributes an implicit zero to an independent series.
    private struct Samples {
        var cells: ArraySlice<Cell>
        let visible: Visibility
        let transform: CartesianChartData.NumericTransform
        let stacked: Bool
        private(set) var total = 0.0

        init(_ row: Row?, _ visible: Visibility, _ data: CartesianChartData) {
            cells = row?.cells[...] ?? []
            self.visible = visible
            transform = visible.yMagnitude.transform
            stacked = data.stacked
        }

        mutating func nextRank() -> Int? {
            while let cell = cells.first {
                if let rank = visible.ranks[cell.series.id], cell.value != 0 { return rank }
                cells = cells.dropFirst()
            }
            return nil
        }

        mutating func advance(at rank: Int) -> Double {
            if !stacked { total = 0 }
            if nextRank() == rank, let cell = cells.first {
                total += transform.position(cell.value)
                cells = cells.dropFirst()
            }
            return total
        }
    }

    private struct Run {
        let first: Row
        let last: Row
        let rows: ArraySlice<Row>
        let changes: [DirectionChange]
    }

    private struct Directions: Equatable {
        // The first and last noncoincident vertical steps in this run.
        var first = 0
        var last = 0
    }

    private struct DirectionChange {
        let rank: Int
        let directions: Directions
    }

    private struct Variation {
        var changes: ArraySlice<DirectionChange>
        var directions = Directions()

        init(_ run: Run?) { changes = run?.changes[...] ?? [] }

        mutating func advance(at rank: Int) {
            if let change = changes.first, change.rank == rank {
                directions = change.directions
                changes = changes.dropFirst()
            }
        }
    }

    private static func curves(_ data: CartesianChartData, _ visible: Visibility) -> [Int: [Edge]] {
        var output: [Int: [Edge]] = [:]
        var remaining = data.rows[...]
        var previous: Run?
        var current = takeRun(from: &remaining, data, visible)
        while let run = current {
            for (left, right) in zip(run.rows, run.rows.dropFirst()) {
                appendCurves(from: left, to: right, before: nil, after: nil,
                             leftRun: nil, rightRun: nil, data, visible, output: &output)
            }
            let next = takeRun(from: &remaining, data, visible)
            if let next {
                appendCurves(from: run.last, to: next.first, before: previous?.last, after: remaining.first,
                             leftRun: run, rightRun: next, data, visible, output: &output)
            }
            previous = run
            current = next
        }
        return output
    }

    private static func takeRun(from remaining: inout ArraySlice<Row>, _ data: CartesianChartData,
                                _ visible: Visibility) -> Run? {
        guard let first = remaining.first else { return nil }
        let rows = remaining.prefix { $0.categoryIndex == first.categoryIndex }
        guard let last = rows.last else { return nil }
        remaining = remaining.dropFirst(rows.count)

        // Each row pair contributes a sparse step function over series ranks.
        // Merge adjacent pairs in balanced batches so long repeated categories
        // can resolve coincident points without walking the rows for each key.
        var batches: [(depth: Int, changes: [DirectionChange])] = []
        let count = visible.last.count
        for (left, right) in zip(rows, rows.dropFirst()) {
            var a = Samples(left, visible, data), b = Samples(right, visible, data)
            var changes: [DirectionChange] = []
            var rank = min(a.nextRank() ?? count, b.nextRank() ?? count)
            while rank < count {
                let leftY = a.advance(at: rank), rightY = b.advance(at: rank)
                let next = min(a.nextRank() ?? count, b.nextRank() ?? count)
                let direction = leftY == rightY ? 0 : (rightY > leftY ? 1 : -1)
                record(Directions(first: direction, last: direction), at: rank, in: &changes)
                if !data.stacked, next > rank + 1 {
                    record(Directions(), at: rank + 1, in: &changes)
                }
                rank = next
            }
            guard !changes.isEmpty else { continue }
            record(Directions(), at: count, in: &changes)
            var depth = 0
            while batches.last?.depth == depth, let previous = batches.popLast() {
                changes = merge(previous.changes, changes)
                depth += 1
            }
            batches.append((depth, changes))
        }
        let changes = batches.reversed().reduce(into: [DirectionChange]()) {
            $0 = merge($1.changes, $0)
        }
        return Run(first: first, last: last, rows: rows, changes: changes)
    }

    private static func record(_ directions: Directions, at rank: Int, in changes: inout [DirectionChange]) {
        guard (changes.last?.directions ?? Directions()) != directions else { return }
        changes.append(DirectionChange(rank: rank, directions: directions))
    }

    private static func merge(_ left: [DirectionChange], _ right: [DirectionChange]) -> [DirectionChange] {
        if left.isEmpty { return right }
        if right.isEmpty { return left }
        var a = left[...], b = right[...]
        var previous = Directions(), next = Directions()
        var merged: [DirectionChange] = []
        while let upcoming = a.first ?? b.first {
            let rank = min(upcoming.rank, b.first?.rank ?? upcoming.rank)
            if let change = a.first, change.rank == rank {
                previous = change.directions
                a = a.dropFirst()
            }
            if let change = b.first, change.rank == rank {
                next = change.directions
                b = b.dropFirst()
            }
            record(Directions(first: previous.first == 0 ? next.first : previous.first,
                              last: next.last == 0 ? previous.last : next.last), at: rank, in: &merged)
        }
        return merged
    }

    private static func appendCurves(from left: Row, to right: Row, before: Row?, after: Row?,
                                     leftRun: Run?, rightRun: Run?, _ data: CartesianChartData,
                                     _ visible: Visibility, output: inout [Int: [Edge]]) {
        var a = Samples(before, visible, data), b = Samples(left, visible, data)
        var c = Samples(right, visible, data), d = Samples(after, visible, data)
        var leftVariation = Variation(leftRun), rightVariation = Variation(rightRun)
        let count = visible.last.count
        let vertical = left.categoryIndex == right.categoryIndex

        func cubic(_ a: Double, _ b: Double, _ c: Double, _ d: Double, _ previousDirection: Int, _ nextDirection: Int) -> Cubic {
            Cubic(left: CGPoint(x: Double(left.categoryIndex), y: b),
                  right: CGPoint(x: Double(right.categoryIndex), y: c),
                  before: before.map { CGPoint(x: Double($0.categoryIndex), y: a) },
                  after: after.map { CGPoint(x: Double($0.categoryIndex), y: d) },
                  verticalBefore: previousDirection, verticalAfter: nextDirection)
        }

        func paint(_ rank: Int, _ edge: Edge) {
            guard let series = visible.last.dropFirst(rank).first else { return }
            output[series.id, default: []].append(edge)
        }

        var pending: (rank: Int, edge: Edge)?
        func append(_ rank: Int, _ stroke: Cubic, baseline: Cubic?) {
            guard !vertical || stroke.left != stroke.right else { return }
            let edge = Edge(stroke: stroke, baseline: baseline == stroke ? nil : baseline)
            if let previous = pending,
               !(previous.edge.baseline == nil && edge.baseline == nil && previous.edge.stroke == stroke) {
                paint(previous.rank, previous.edge)
            }
            pending = (rank, edge)
        }

        var rank = 0
        while rank < count {
            let lower = data.kind == .area && !vertical
                ? (data.stacked
                    ? cubic(a.total, b.total, c.total, d.total, leftVariation.directions.last, rightVariation.directions.first)
                    : cubic(0, 0, 0, 0, 0, 0))
                : nil
            let hasChange = min(a.nextRank() ?? count, b.nextRank() ?? count,
                                c.nextRank() ?? count, d.nextRank() ?? count,
                                leftVariation.changes.first?.rank ?? count, rightVariation.changes.first?.rank ?? count) == rank
            leftVariation.advance(at: rank)
            rightVariation.advance(at: rank)
            let upper = cubic(a.advance(at: rank), b.advance(at: rank), c.advance(at: rank), d.advance(at: rank),
                              leftVariation.directions.last, rightVariation.directions.first)
            if hasChange {
                append(rank, upper, baseline: lower)
                rank += 1
            } else {
                // With no changes in the neighboring rows or run directions,
                // every rank through the next change has the same stroke.
                let next = min(a.nextRank() ?? count, b.nextRank() ?? count, c.nextRank() ?? count,
                               d.nextRank() ?? count, leftVariation.changes.first?.rank ?? count,
                               rightVariation.changes.first?.rank ?? count)
                append(next - 1, upper, baseline: lower)
                rank = next
            }
        }
        if let pending { paint(pending.rank, pending.edge) }
    }
}

struct ScatterCoordinate: Hashable {
    let x: Double
    let y: Double
}
