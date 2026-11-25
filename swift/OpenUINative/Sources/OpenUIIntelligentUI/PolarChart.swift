import Charts
import SwiftUI

// A mounted client can supply hover phases without sending AppKit events.
@Observable @MainActor final class PolarChartHoverInput {
    enum Target: Equatable {
        case plot(HoverPhase)
        case legend(PolarChartData.Label?)
    }

    var target: Target?
}

struct PolarChart: View {
    let kind: PolarChartKind
    let data: PolarChartData
    var hoverInput: PolarChartHoverInput? = nil
    var onPlotHover: ((HoverPhase) -> Void)? = nil
    var onPlotSelection: ((Bool) -> Void)? = nil
    @State private var hidden: Set<PolarChartData.Label> = []
    @State private var showAll = false
    @State private var activeLabel: PolarChartData.Label?

    var body: some View {
        let hiddenKeys = data.liveHiddenKeys(hidden)
        let visible = data.slices.filter { !hiddenKeys.contains($0.label) }
        let geometry = PolarChartGeometry(kind: kind, visible: visible)
        let total = data.visibleTotal(hiding: hiddenKeys)
        ViewThatFits(in: .horizontal) {
            layout(beside: true, geometry: geometry, visible: visible, total: total, hiddenKeys: hiddenKeys)
            layout(beside: false, geometry: geometry, visible: visible, total: total, hiddenKeys: hiddenKeys)
        }
        .clipContainer()
        .transaction {
            $0.animation = nil
            $0.disablesAnimations = true
        }
        .onChange(of: hiddenKeys) { _, current in
            if hidden != current { hidden = current }
        }
        .onChange(of: hoverInput?.target, initial: true) { _, target in
            if case let .legend(label)? = target { activeLabel = label }
        }
    }

    private func layout(beside: Bool, geometry: PolarChartGeometry, visible: [PolarChartData.Slice], total: Double,
                        hiddenKeys: Set<PolarChartData.Label>) -> some View {
        PolarChartLayout(beside: beside) {
            PolarPlot(geometry: geometry, data: data, visible: visible, total: total, activeLabel: $activeLabel,
                      hoverInput: hoverInput, onPlotHover: onPlotHover, onPlotSelection: onPlotSelection)
            PolarChartLegend(data: data, total: total, beside: beside, hidden: hiddenKeys, showAll: $showAll,
                             activeLabel: $activeLabel, tracksPointer: hoverInput == nil) { label in
                hidden = data.toggling(label, in: hiddenKeys)
            }
        }
    }
}

private struct PolarChartLayout: Layout {
    let beside: Bool
    private let gap: CGFloat = 20

    private func panelWidth(_ width: CGFloat) -> CGFloat {
        beside ? (width - gap) / 2 : width
    }

    private func plotSize(_ width: CGFloat) -> CGFloat {
        // The wrapper omits height, leaving the orchestrator's 300-point fallback.
        max(150, min(300, panelWidth(width)))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let offered = proposal.width.flatMap { $0.isFinite ? layoutPoints($0) : nil } ?? 300
        let width = max(beside ? 400 : 0, offered)
        let plot = plotSize(width)
        let legend = subviews.last?.sizeThatFits(.init(width: panelWidth(width), height: nil)).height ?? 0
        let height = beside ? max(plot, PolarChartLegend.sideHeight) : plot + gap + legend
        return CGSize(width: width, height: layoutPoints(height))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard let plot = subviews.first, let legend = subviews.last else { return }
        let width = panelWidth(bounds.width)
        let size = plotSize(bounds.width)
        let plotCenter = CGPoint(x: bounds.minX + width / 2, y: beside ? bounds.midY : bounds.minY + size / 2)
        plot.place(at: plotCenter, anchor: .center, proposal: .init(width: size, height: size))
        let legendCenter = CGPoint(x: beside ? bounds.midX + (width + gap) / 2 : bounds.midX,
                                   y: beside ? bounds.midY : bounds.minY + size + gap)
        legend.place(at: legendCenter, anchor: beside ? .center : .top,
                     proposal: .init(width: width, height: nil))
    }
}

private struct PolarPlot: View {
    let geometry: PolarChartGeometry
    let data: PolarChartData
    let visible: [PolarChartData.Slice]
    let total: Double
    @Binding var activeLabel: PolarChartData.Label?
    let hoverInput: PolarChartHoverInput?
    let onPlotHover: ((HoverPhase) -> Void)?
    let onPlotSelection: ((Bool) -> Void)?
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.locale) private var locale
    @State private var pointer: CGPoint?

    var body: some View {
        GeometryReader { bounds in
            let size = min(bounds.size.width, bounds.size.height)
            let hit = pointer.flatMap { geometry.hit(at: $0, size: size) }
            let activeID = hit?.slice.id ?? activeLabel.flatMap { label in visible.first { $0.label == label }?.id }
            PolarSectorDrawing(geometry: geometry, radius: size * 0.45, activeID: activeID)
                .equatable()
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text(verbatim: geometry.kind == .pie ? "Pie chart" : "Donut chart"))
                .overlay {
                    Color.clear.contentShape(Rectangle())
                        .onContinuousHover { phase in
                            onPlotHover?(phase)
                            if hoverInput == nil { updateHover(phase, size: size) }
                        }
                        .onChange(of: hoverInput?.target, initial: true) { _, target in
                            switch target {
                            case let .plot(phase): updateHover(phase, size: size)
                            case .legend: pointer = nil
                            case nil: break
                            }
                        }
                }
                .overlay(alignment: .topLeading) {
                    if let hit {
                        PolarChartTooltip(label: hit.slice.label.text,
                                          value: data.tooltipValue(hit.slice, total: total, locale: locale),
                                          colour: ChartPalette.colour(hit.slice.colourIndex, in: theme))
                    }
                }
        }
        .clipContainer()
        .onChange(of: activeLabel != nil && pointer != nil) { _, selected in onPlotSelection?(selected) }
    }

    private func updateHover(_ phase: HoverPhase, size: CGFloat) {
        if case let .active(location) = phase {
            pointer = location
        } else {
            pointer = nil
        }
        let label = pointer.flatMap { geometry.hit(at: $0, size: size)?.slice.label }
        if activeLabel != label { activeLabel = label }
    }
}

private struct PolarChartTooltip: View {
    let label: String
    let value: String
    let colour: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(verbatim: label)
                .fontWeight(.semibold)
                .accessibilityAddTraits(.isHeader)
            HStack(alignment: .top, spacing: 4) {
                RoundedRectangle(cornerRadius: 2)
                    .fill(colour)
                    .frame(width: 10, height: 10)
                    .padding(.top, 2)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: label)
                    Text(verbatim: value)
                }
            }
        }
        .font(.caption)
        .padding(6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 4))
        .allowsHitTesting(false)
    }
}

// Pointer movement within a wedge updates the tooltip without rebuilding all
// of the sector marks. The active wedge is drawn separately, over the rest.
private struct PolarSectorDrawing: View, Equatable {
    private struct Paint {
        let sector: PolarChartGeometry.Sector
        let colour: Color
        let inset: CGFloat

        var start: Double { sector.start }
        var end: Double { sector.end }
    }

    let geometry: PolarChartGeometry
    let radius: CGFloat
    let activeID: PolarChartData.Slice.ID?
    @Environment(\.intelligentTheme) private var theme

    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.radius == rhs.radius && lhs.activeID == rhs.activeID && lhs.geometry == rhs.geometry
    }

    var body: some View {
        sectors(geometry.sectors, highlight: false)
            .overlay {
                if let activeID, let active = geometry.sectors.first(where: { $0.slice.id == activeID }) {
                    sectors([active], highlight: true)
                        .offset(active.hoverOffset)
                }
            }
            .allowsHitTesting(false)
    }

    private func sectors(_ sectors: [PolarChartGeometry.Sector], highlight: Bool) -> some View {
        let inset = geometry.inset(radius: radius)
        let donut = geometry.kind == .donut
        let paints = sectors.compactMap { sector -> Paint? in
            if !highlight, sector.slice.id == activeID { return nil }
            var colour = ChartPalette.token(sector.slice.colourIndex, in: theme)
            if highlight {
                colour.red *= 1.08
                colour.green *= 1.08
                colour.blue *= 1.08
            }
            if !highlight, activeID != nil { colour.alpha *= 0.4 }
            return Paint(sector: sector, colour: colour.color, inset: sector.isFullCircle ? 0 : inset)
        }
        let band = donut ? radius * PolarChartGeometry.bandInner : 0
        let hole = radius * PolarChartGeometry.hole
        let track = theme.surface.color.opacity(!highlight && activeID != nil ? 0.4 : 1)
        let center = CGPoint(x: radius / 0.45, y: radius / 0.45)
        var bandPaints: [Paint] = []
        var trackPaints: [Paint] = []
        var tapered: [(Path, Color)] = []
        if donut {
            for paint in paints {
                if let path = paint.sector.collapsedPath(center: center, inner: band, outer: radius, inset: paint.inset) {
                    tapered.append((path, paint.colour))
                } else {
                    bandPaints.append(paint)
                }
                if let path = paint.sector.collapsedPath(center: center, inner: hole, outer: band, inset: paint.inset) {
                    tapered.append((path, track))
                } else {
                    trackPaints.append(paint)
                }
            }
        } else {
            bandPaints = paints
        }
        let tips = tapered
        return Chart {
            // Include the full turn in the scale even when only the highlight
            // is drawn or zero-weight rows occupy part of the donut's padding.
            SectorMark(angle: .value("Angle", 0..<PolarChartGeometry.turn), outerRadius: .fixed(0))
                .foregroundStyle(Color.clear)
            if donut {
                SectorPlot(trackPaints, angle: .value("Angle", \.start, \.end),
                           innerRadius: .fixed(hole), outerRadius: .fixed(band), angularInset: \.inset)
                    .foregroundStyle(track)
            }
            SectorPlot(bandPaints, angle: .value("Angle", \.start, \.end),
                       innerRadius: .fixed(band), outerRadius: .fixed(radius), angularInset: \.inset)
                .foregroundStyle(\.colour)
        }
        .chartLegend(.hidden)
        .overlay {
            if !tips.isEmpty {
                Canvas { context, _ in
                    for (path, colour) in tips { context.fill(path, with: .color(colour)) }
                }
            }
        }
    }
}
