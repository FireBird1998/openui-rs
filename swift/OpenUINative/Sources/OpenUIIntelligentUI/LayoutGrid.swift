import OpenUINative
import SwiftUI

struct LayoutGrid: View {
    let element: Element
    let path: String

    var body: some View {
        let columns = element.prop("columns")?.ruleScalar?.number ?? 2
        let minimum = element.prop("minChildWidth")?.ruleScalar?.number ?? 0
        let gapValue = element.prop("gap")
        let gap = Spacing.points(gapValue)
        // An overflowing four-point step emits Infinitypx. The browser drops
        // the gap and, if it embeds that length, the responsive template too.
        let overflow = gap == nil && gapValue?.ruleScalar?.number?.isFinite == true
        let count = overflow && minimum != 0 ? 1 : Int(max(1, min(12, columns.rounded())))
        let responsiveMinimum: CGFloat? = minimum == 0 ? nil : max(0, minimum)
        EqualTracks(columns: count, minimum: responsiveMinimum, gap: layoutPoints(gap ?? (overflow ? 0 : 12))) {
            ForEach(element.layoutChildren) { child in
                child.drawCell(in: path, stretches: true)
            }
        }
        .clipContainer()
    }
}

// CSS auto-fill retains empty tracks in the last row. Each row takes the
// height of its tallest cell, then proposes that height to its auto-sized cells.
private struct EqualTracks: Layout {
    let columns: Int
    let minimum: CGFloat?
    let gap: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width: CGFloat
        if let proposed = proposal.width, proposed.isFinite {
            width = layoutPoints(proposed)
        } else {
            // Auto-repeat has one repetition until the grid has a definite width.
            let repetitions = minimum == nil ? columns : 1
            let widest = subviews.map { $0.sizeThatFits(.unspecified).width }.max() ?? 0
            width = layoutPoints(CGFloat(repetitions) * widest + CGFloat(repetitions - 1) * gap)
        }
        let rows = measure(width: width, views: subviews)
        let assigned = proposal.height.flatMap { $0.isFinite ? $0 : nil } ?? 0
        return CGSize(width: width, height: layoutPoints(max(rows.height, assigned)))
    }

    private struct Rows {
        let columns: Int
        let track: CGFloat
        let heights: [CGFloat]
        let height: CGFloat
    }

    private func measure(width: CGFloat, views: Subviews) -> Rows {
        let count = minimum.map {
            // CSS floors the track at one pixel when counting auto repetitions.
            Int(max(1, min(CGFloat(columns), (width + gap) / (max(1, $0) + gap))))
        } ?? columns
        let track = max(0, width - CGFloat(count - 1) * gap) / CGFloat(count)
        let natural = ProposedViewSize(width: track, height: nil)
        let heights = stride(from: 0, to: views.count, by: count).map { start in
            views[start..<min(start + count, views.count)].map { $0.sizeThatFits(natural).height }.max() ?? 0
        }
        let height = heights.reduce(0, +) + CGFloat(max(0, heights.count - 1)) * gap
        return Rows(columns: count, track: track, heights: heights, height: layoutPoints(height))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let rows = measure(width: bounds.width, views: subviews)
        guard !rows.heights.isEmpty else { return }
        let extra = max(0, bounds.height - rows.height) / CGFloat(rows.heights.count)
        var y = bounds.minY
        for (row, natural) in rows.heights.enumerated() {
            let height = natural + extra
            let start = row * rows.columns
            let cells = subviews[start..<min(start + rows.columns, subviews.count)]
            for (column, cell) in cells.enumerated() {
                let x = bounds.minX + CGFloat(column) * (rows.track + gap)
                cell.place(at: CGPoint(x: x, y: y), anchor: .topLeading,
                           proposal: ProposedViewSize(width: rows.track, height: height))
            }
            y += height + gap
        }
    }
}
