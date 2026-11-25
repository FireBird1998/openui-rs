// Native drawing for the Cartesian branches of Intelligent UI’s Chart.
import Charts
import OpenUINative
import SwiftUI

struct IntelligentChart: View {
    let element: Element

    var body: some View {
        let type = element.prop("type")?.ruleScalar?.string ?? ""
        if let kind = CartesianChartKind(rawValue: type) {
            if let data = CartesianChartData(element, kind: kind) {
                CartesianPlot(data: data)
            }
        } else if let kind = PolarChartKind(rawValue: type) {
            if let data = PolarChartData(element) {
                PolarChart(kind: kind, data: data)
            }
        } else {
            Placeholder(element: element)
        }
    }
}

struct CartesianPlot: View {
    let data: CartesianChartData
    var hoverObservation: CartesianHoverObservation? = nil
    @Environment(\.intelligentTheme) private var theme
    @State private var hidden: Set<CartesianChartData.Name> = []
    @State private var firstVisibleCategory = -0.5

    var body: some View {
        let visible = data.visibility(hiding: hidden)
        let geometry = CartesianChartGeometry(data: data, visible: visible)
        CartesianChartLayout(explicitHeight: data.explicitHeight) {
            drawing(geometry, visible: visible)
        } legend: {
            if !data.legendKeys.isEmpty {
                legend(visibleCount: visible.first.count)
            }
        }
        .onChange(of: data.legendKeys) {
            let kept = data.liveHiddenKeys(hidden)
            if kept.count != hidden.count { hidden = kept }
        }
    }

    @ViewBuilder
    private func drawing(_ geometry: CartesianChartGeometry, visible: CartesianChartData.Visibility) -> some View {
        if data.kind == .scatter && data.legendKeys.isEmpty {
            Text(verbatim: "No data available").frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if data.kind == .scatter {
            plot(geometry, visible: visible, ticks: [])
                .chartXScale(domain: .automatic(includesZero: false, dataType: Double.self) {
                    $0 = data.xMagnitude.domain
                })
        } else {
            GeometryReader { bounds in
                let count = max(1, Int(layoutPoints(bounds.size.width) / 72))
                let last = max(1, data.categories.count)
                let first = Int(exactly: firstVisibleCategory.rounded(.down)) ?? 0
                let start = min(last, max(0, first))
                let end = start + min(last - start, count + 2)
                plot(geometry, visible: visible, ticks: (start..<end).map(Double.init))
                    .chartXScale(domain: -0.5...(Double(last) - 0.5))
                    .modifier(CartesianChartViewport(categoryCount: data.categories.count))
                    .chartScrollPosition(x: $firstVisibleCategory)
            }
        }
    }

    private func colour(_ index: Int) -> Color {
        ChartPalette.colour(data.colourIndex(forSeriesIndex: index), in: theme)
    }

    @ViewBuilder
    private func legend(visibleCount: Int) -> some View {
        if data.legendKeys.count <= 6 {
            FlexLayout(row: true, align: .center, justify: .start, wrap: true, gap: 8, inset: 0,
                       em: 12, acceptsHeight: false, definiteHeight: false) {
                legendButtons(visibleCount: visibleCount)
            }
            .fixedSize(horizontal: false, vertical: true)
        } else {
            ScrollView(.horizontal) {
                LazyHStack(spacing: 8) {
                    legendButtons(visibleCount: visibleCount)
                }
            }
            .scrollIndicators(.hidden)
            .frame(height: 14)
        }
    }

    private func legendButtons(visibleCount: Int) -> some View {
        ForEach(data.legendKeys.indices, id: \.self) { index in
            let key = data.legendKeys.dropFirst(index).first ?? .init("")
            Button {
                if hidden.contains(key) {
                    hidden.remove(key)
                } else if visibleCount > 1 {
                    hidden.insert(key)
                }
            } label: {
                HStack(spacing: 4) {
                    Circle().fill(colour(data.seriesByKey[key]?.lastIndex ?? index))
                        .frame(width: 8, height: 8)
                    Text(verbatim: key.text)
                }
                .font(.caption)
                .opacity(hidden.contains(key) ? 0.45 : 1)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text(verbatim: hidden.contains(key) ? "Show " + key.text : "Hide " + key.text))
        }
    }

    // Swift Charts supplies the scales, axes and scroll interaction. Canvas
    // takes the compressed geometry without constructing a view per mark.
    private func plot(_ geometry: CartesianChartGeometry, visible: CartesianChartData.Visibility,
                      ticks: [Double]) -> some View {
        let yMagnitude = visible.yMagnitude
        return Chart {
            RuleMark(y: .value(data.category, 0.0)).foregroundStyle(.clear)
            if data.kind == .scatter {
                RuleMark(x: .value(data.category, 0.0)).foregroundStyle(.clear)
            }
        }
        .chartYScale(domain: .automatic(includesZero: false, dataType: Double.self) { $0 = yMagnitude.domain })
        .chartLegend(.hidden)
        .chartOverlay { proxy in
            GeometryReader { bounds in
                if let anchor = proxy.plotFrame {
                    let plot = bounds[anchor]
                    let viewport = proxy.plotContainerFrame.map { bounds[$0] } ?? plot
                    ZStack {
                        Canvas { context, _ in
                            draw(geometry, visible: visible, proxy: proxy,
                                 offset: CGPoint(x: plot.minX - viewport.minX, y: plot.minY - viewport.minY),
                                 in: context)
                        }
                        .allowsHitTesting(false)
                        ChartHover(proxy: proxy, viewport: viewport, plot: plot, data: data, visible: visible,
                                   observation: hoverObservation)
                    }
                    .frame(width: layoutPoints(viewport.width), height: layoutPoints(viewport.height))
                    .clipContainer()
                    .position(x: viewport.midX, y: viewport.midY)
                }
            }
        }
        .chartXAxis {
            if data.kind == .scatter {
                AxisMarks { tick in
                    AxisGridLine()
                    AxisTick()
                    tickLabel(tick, data.xMagnitude.transform)
                }
            } else {
                AxisMarks(values: ticks) { tick in
                    AxisTick()
                    if let index = tick.as(Double.self).flatMap(Int.init(exactly:)),
                       let row = data.row(forCategory: index) {
                        AxisValueLabel { Text(verbatim: row.label) }
                    }
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading) { tick in
                AxisGridLine()
                AxisTick()
                tickLabel(tick, yMagnitude.transform)
            }
        }
    }

    @AxisMarkBuilder
    private func tickLabel(_ tick: AxisValue, _ transform: CartesianChartData.NumericTransform) -> some AxisMark {
        if !transform.isIdentity, let n = tick.as(Double.self) {
            AxisValueLabel { Text(verbatim: JavaScript.numberString(transform.value(at: n))) }
        } else {
            AxisValueLabel()
        }
    }

    private func draw(_ geometry: CartesianChartGeometry, visible: CartesianChartData.Visibility,
                      proxy: ChartProxy, offset: CGPoint, in context: GraphicsContext) {
        if data.kind == .scatter {
            for dot in geometry.dots {
                guard let point = proxy.position(for: data.scatterCoordinates(x: dot.point.x, y: dot.point.y)) else { continue }
                let rect = CGRect(x: point.x + offset.x - 4, y: point.y + offset.y - 4, width: 8, height: 8)
                context.fill(Path(ellipseIn: rect), with: .color(colour(dot.series.lastIndex)))
            }
            return
        }

        guard let x0 = proxy.position(forX: 0.0), let x1 = proxy.position(forX: 1.0),
              let y0 = proxy.position(forY: 0.0), let y1 = proxy.position(forY: 1.0) else { return }
        let dx = x1 - x0, dy = y1 - y0
        let origin = CGPoint(x: x0 + offset.x, y: y0 + offset.y)
        func point(_ model: CGPoint) -> CGPoint {
            CGPoint(x: origin.x + model.x * dx, y: origin.y + model.y * dy)
        }

        let groups = data.stacked ? 1 : max(1, visible.last.count)
        let slot = abs(dx) * 0.8 / CGFloat(groups)
        let width = min(data.stacked ? 28 : 16, slot * 0.95)
        for bar in geometry.bars {
            let shift = data.stacked ? 0 : (CGFloat(bar.rank) - CGFloat(groups - 1) / 2) * slot
            let center = point(CGPoint(x: bar.category, y: 0)).x + shift
            let start = origin.y + bar.start * dy, end = origin.y + bar.end * dy
            let rect = CGRect(x: center - width / 2, y: min(start, end), width: width, height: abs(end - start))
            context.fill(Path(roundedRect: rect, cornerRadius: 4), with: .color(colour(bar.colour)))
        }

        for series in visible.last {
            guard let edges = geometry.curves[series.id] else { continue }
            var fill = Path(), stroke = Path()
            var previousPoint: CGPoint?
            for edge in edges {
                let upper = edge.stroke
                let left = point(upper.left), right = point(upper.right)
                let firstControl = point(upper.control1), secondControl = point(upper.control2)
                if let lower = edge.baseline {
                    fill.move(to: left)
                    fill.addCurve(to: right, control1: firstControl, control2: secondControl)
                    fill.addLine(to: point(lower.right))
                    fill.addCurve(to: point(lower.left), control1: point(lower.control2), control2: point(lower.control1))
                    fill.closeSubpath()
                }
                if previousPoint != left { stroke.move(to: left) }
                stroke.addCurve(to: right, control1: firstControl, control2: secondControl)
                previousPoint = right
            }
            let color = colour(series.lastIndex)
            context.fill(fill, with: .color(color.opacity(0.4)))
            context.stroke(stroke, with: .color(color), style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
        }
    }
}

struct CartesianChartLayout<Drawing: View, Legend: View>: View {
    let explicitHeight: CGFloat?
    @ViewBuilder let drawing: Drawing
    @ViewBuilder let legend: Legend

    var body: some View {
        VStack(spacing: 0) {
            drawing.frame(height: explicitHeight == nil ? 296 : nil)
            legend
        }
        .frame(height: explicitHeight, alignment: .top)
        .clipContainer()
        .transaction {
            $0.animation = nil
            $0.disablesAnimations = true
        }
    }
}

// The default upstream density reserves 72 points per category. Swift Charts
// owns the horizontal scroll interaction and keeps the numeric axis in place.
struct CartesianChartViewport: ViewModifier {
    let categoryCount: Int

    func body(content: Content) -> some View {
        GeometryReader { bounds in
            let count = max(1, Int(layoutPoints(bounds.size.width) / 72))
            if categoryCount > count {
                content.chartScrollableAxes(.horizontal)
                    .chartXVisibleDomain(length: count)
                    .clipContainer()
            } else {
                content
            }
        }
    }
}
