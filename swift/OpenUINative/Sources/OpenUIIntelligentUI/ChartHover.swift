// Category hover selects one row. Scatter compares the present points and at
// most one implicit zero per row, with upstream's dataset-first tie breaking.
import Charts
import SwiftUI

extension CartesianChartData {
    func scatterHit(at pointer: CGPoint, visible: Visibility,
                    position: (Double, Double) -> CGPoint?) -> (row: Row, series: Series)? {
        var nearest: (row: Row, series: Series)?
        var distance = CGFloat.infinity
        for row in rows where row.x.isFinite {
            func offer(_ series: Series, _ y: Double) {
                guard let point = position(row.x, y) else { return }
                let delta = hypot(point.x - pointer.x, point.y - pointer.y)
                guard delta <= 30 else { return }
                let earlier = nearest.map { series.id < $0.series.id } ?? true
                if delta < distance || (delta == distance && earlier) {
                    distance = delta
                    nearest = (row, series)
                }
            }
            if let missing = visible.firstMissing(in: row) { offer(missing, 0) }
            for cell in row.cells where visible.ranks[cell.series.id] != nil {
                offer(cell.series, cell.value)
            }
        }
        return nearest
    }

    func scatterTooltip(row: Row, series: Series) -> String {
        (name(forSeriesID: series.id) ?? "") + "\nX: " + JavaScript.numberString(row.x) + "\nY: " +
            JavaScript.numberString(row.values[series.id, default: 0])
    }

    func categoryTooltip(index: Int, visible: Visibility) -> String? {
        // Upstream uses the category domain's ordinal to read the input row.
        guard index >= 0, let row = rows.dropFirst(index).first else { return nil }
        return ([row.label] + visible.first.map { series in
            (name(forSeriesID: series.id) ?? "") + ": " + JavaScript.numberString(row.values[series.id, default: 0])
        }).joined(separator: "\n")
    }
}

// Optional instrumentation for a mounted plot. The proxy measures positions
// in the plot, whose frame is reported in the caller's coordinate space.
struct CartesianHoverObservation {
    let coordinateSpace: String
    let layout: (ChartProxy, CGRect) -> Void
    let phase: (HoverPhase) -> Void
    let selection: (Bool) -> Void
}

struct ChartHover: View {
    let proxy: ChartProxy
    let viewport: CGRect
    let plot: CGRect
    let data: CartesianChartData
    let visible: CartesianChartData.Visibility
    var observation: CartesianHoverObservation? = nil
    @State private var pointer: CGPoint?

    var body: some View {
        let text = pointer.flatMap { tooltip(at: CGPoint(
            x: $0.x + viewport.minX - plot.minX, y: $0.y + viewport.minY - plot.minY)) }
        Color.clear.contentShape(Rectangle())
            .onContinuousHover { phase in
                observation?.phase(phase)
                if case let .active(point) = phase {
                    pointer = point
                } else {
                    pointer = nil
                }
            }
            .overlay(alignment: .topLeading) {
                if let text {
                    ChartTooltip(text: text)
                }
            }
            .background {
                if let observation {
                    let coordinateSpace = observation.coordinateSpace
                    Color.clear
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(coordinateSpace)) } action: { frame in
                            let origin = CGPoint(x: frame.minX + plot.minX - viewport.minX,
                                                 y: frame.minY + plot.minY - viewport.minY)
                            observation.layout(proxy, CGRect(origin: origin, size: plot.size))
                        }
                        .allowsHitTesting(false)
                }
            }
            .onChange(of: text != nil) { _, selected in observation?.selection(selected) }
    }

    private func tooltip(at point: CGPoint) -> String? {
        if data.kind == .scatter {
            guard let hit = data.scatterHit(at: point, visible: visible, position: {
                proxy.position(for: data.scatterCoordinates(x: $0, y: $1))
            }) else { return nil }
            return data.scatterTooltip(row: hit.row, series: hit.series)
        }
        guard let index: Double = proxy.value(atX: point.x) else { return nil }
        let nearest = data.kind == .bar ? floor(index + 0.5) : ceil(index - 0.5)
        guard let category = Int(exactly: nearest) else { return nil }
        return data.categoryTooltip(index: max(0, min(data.categories.count - 1, category)), visible: visible)
    }
}

// Shared native tooltip treatment. The surrounding plot supplies its own
// hover selection and value formatting.
struct ChartTooltip: View {
    let text: String

    var body: some View {
        Text(verbatim: text)
            .font(.caption)
            .padding(6)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 4))
            .allowsHitTesting(false)
    }
}
