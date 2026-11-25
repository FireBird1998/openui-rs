import SwiftUI

struct PolarChartGeometry: Equatable {
    struct Sector: Equatable {
        let slice: PolarChartData.Slice
        let start: Double
        let end: Double

        var isFullCircle: Bool { end - start >= PolarChartGeometry.turn - 1e-12 }

        var hoverOffset: CGSize {
            guard end - start < PolarChartGeometry.turn - 1e-9 else { return .zero }
            let middle = (start + end) / 2
            return CGSize(width: sin(middle) * 8, height: -cos(middle) * 8)
        }

        // d3 joins the two straight edges to a single point on the
        // inner circle when the pad would otherwise eat the entire inner arc.
        // Swift Charts' parallel edges would meet farther out from the hole.
        func collapsedPath(center: CGPoint, inner: CGFloat, outer: CGFloat, inset: CGFloat) -> Path? {
            guard !isFullCircle, inner > 0, inset > 0 else { return nil }
            let half = (end - start) / 2
            guard half <= asin(min(1, inset / inner)) else { return nil }
            let trim = asin(min(1, inset / outer))
            guard half > trim else { return Path() }
            let middle = (start + end) / 2
            var path = Path()
            path.move(to: CGPoint(x: center.x + inner * sin(middle), y: center.y - inner * cos(middle)))
            path.addRelativeArc(center: center, radius: outer, startAngle: .radians(start - .pi / 2 + trim),
                                delta: .radians((half - trim) * 2))
            path.closeSubpath()
            return path
        }
    }

    static let turn = 2 * Double.pi
    static let bandInner = 0.9
    static let hole = bandInner * 0.28

    let kind: PolarChartKind
    let sectors: [Sector]
    private let pad: Double

    init(kind: PolarChartKind, visible: [PolarChartData.Slice]) {
        self.kind = kind
        let turn = Self.turn
        let count = Double(visible.count)
        pad = kind == .donut && count > 0 ? min(turn / count, .pi / 360) : 0
        let sum = visible.reduce(0.0) { $0 + max(0, $1.value) }
        // d3 allocates paint only for positive values; a non-positive row
        // still owns its pad angle. An infinite positive sum yields no paint.
        guard sum > 0, sum.isFinite else {
            sectors = []
            return
        }
        let available = max(0, turn - pad * count)
        let multiplier = available / sum
        var start = 0.0
        var sectors: [Sector] = []
        for slice in visible {
            let positive = max(0, slice.value)
            let angle = multiplier.isFinite ? positive * multiplier : (positive / sum) * available
            let end = min(turn, start + angle + pad)
            if end - start - pad > 1e-12 {
                sectors.append(Sector(slice: slice, start: start, end: end))
            }
            start = end
        }
        self.sectors = sectors
    }

    func inset(radius: Double) -> Double { sin(pad / 2) * radius }

    func hit(at point: CGPoint, size: CGFloat) -> Sector? {
        let x = point.x - size / 2
        let y = point.y - size / 2
        let radius = hypot(x, y)
        let outer = size * 0.45
        let inner = kind == .donut ? outer * Self.hole : 0
        guard radius <= outer, radius >= inner else { return nil }
        if radius == 0 { return sectors.last }
        let inset = inset(radius: outer)
        let bearing = atan2(x, -y)
        let angle = bearing < 0 ? bearing + Self.turn : bearing
        // The sectors are ordered and disjoint. Hover does no per-row work.
        var lower = 0
        var upper = sectors.count
        while lower < upper {
            let middle = lower + (upper - lower) / 2
            guard let candidate = sectors.dropFirst(middle).first else { return nil }
            if angle >= candidate.end {
                lower = middle + 1
            } else if angle < candidate.start {
                upper = middle
            } else {
                if candidate.isFullCircle { return candidate }
                let center = CGPoint(x: size / 2, y: size / 2)
                if let path = candidate.collapsedPath(center: center, inner: inner, outer: outer, inset: inset) {
                    return path.contains(point) ? candidate : nil
                }
                let edge = min(angle - candidate.start, candidate.end - angle, .pi / 2)
                return sin(edge) * radius >= inset ? candidate : nil
            }
        }
        return nil
    }
}
