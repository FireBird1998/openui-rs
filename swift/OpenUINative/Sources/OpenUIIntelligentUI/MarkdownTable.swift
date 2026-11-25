import SwiftUI

struct MarkdownTable: View {
    let head: [String]
    let rows: [[String]]
    @State private var sizing = MarkdownTableSizing()
    @State private var measureInHost = false
    @Environment(\.intelligentViewport) private var viewport
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.self) private var environment

    var body: some View {
        let content = MarkdownTableContent(head: head, rows: rows, compact: (viewport?.width ?? .infinity) <= 640)
        let shape = RoundedRectangle(cornerRadius: 14)
        TableExtent(sizing: sizing, content: content, environment: environment, measureInHost: measureInHost) {
            GeometryReader { proxy in
                MarkdownTableClip(content: content, sizing: sizing, size: proxy.size,
                                  initialHeight: max(0, min(600, viewport?.height ?? 600)))
                    .clipContainer(to: shape)
            }
            .layoutValue(key: TableMeasurementRole.self, value: .drawing)
            ForEach(Array(head.enumerated()), id: \.offset) { _, value in
                measurementCell(value, header: true)
                    .layoutValue(key: TableMeasurementRole.self, value: .heading)
            }
            TableTextProbe {
                measurementCell("Layout sample")
                measurementCell("Layout sample").textRenderer(StandardTextSizing())
            }
            .onGeometryChange(for: Bool.self) { $0.size.width > 1 } action: { measureInHost = $0 }
            .layoutValue(key: TableMeasurementRole.self, value: .sample)
            if measureInHost {
                ForEach(Array(rows.enumerated()), id: \.offset) { row, values in
                    ForEach(Array(values.enumerated()), id: \.offset) { column, value in
                        measurementCell(value)
                            .layoutValue(key: TableMeasurementRole.self, value: .body(row, column))
                    }
                }
            }
        }
        .overlay {
            shape.strokeBorder(theme.border.color, lineWidth: 1).allowsHitTesting(false)
        }
        .accessibilityElement(children: .contain)
    }

    private func measurementCell(_ value: String, header: Bool = false) -> some View {
        let metrics = header ? MarkdownTableContent.heading : MarkdownTableContent.body
        return MarkdownTableContent.words(value, metrics: metrics, theme: theme)
            .fixedSize(horizontal: header, vertical: true)
            .hidden().accessibilityHidden(true)
    }
}

private struct TableMeasurementRole: LayoutValueKey {
    enum Role { case unspecified, drawing, heading, sample, body(Int, Int) }
    static let defaultValue = Role.unspecified
}

/// The host can replace both dimensions of Text with a TextRenderer. This
/// probe compares its response to narrow and intrinsic proposals.
private struct TableTextProbe: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let text = subviews.first, let standard = subviews.dropFirst().first else { return .zero }
        let intrinsic = text.sizeThatFits(.unspecified)
        let narrow = ProposedViewSize(width: 24, height: nil)
        let custom = intrinsic != standard.sizeThatFits(.unspecified)
            || text.sizeThatFits(narrow) != standard.sizeThatFits(narrow)
        return CGSize(width: custom ? 2 : 1, height: intrinsic.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {}
}

private struct StandardTextSizing: TextRenderer {
    func draw(layout: Text.Layout, in context: inout GraphicsContext) {
        for line in layout { context.draw(line) }
    }
}

@MainActor
private struct TableExtent: @preconcurrency Layout {
    let sizing: MarkdownTableSizing
    let content: MarkdownTableContent
    let environment: EnvironmentValues
    let measureInHost: Bool

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        // The horizontal scroll view can collapse to zero width. Stack
        // flexibility probes at zero width do not need a row layout.
        guard proposal.width != 0 else { return .zero }
        let width = proposal.width.flatMap { $0.isFinite ? max(0, $0 - 2) : nil }
        let geometry = geometry(width: width, subviews: subviews)
        // A host renderer that changes sizing activates measurement in the
        // host graph. Wait for those measurements before placing any rows.
        let height = geometry?.rows.height ?? 0
        return CGSize(width: (width ?? geometry?.tracks.width ?? 0) + 2, height: height + 2)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard bounds.width > 0 else { return }
        let width = proposal.width.flatMap { $0.isFinite ? max(0, $0 - 2) : nil }
        sizing.place(geometry(width: width, subviews: subviews), in: bounds.size, content: content)
        for subview in subviews {
            switch subview[TableMeasurementRole.self] {
            case .drawing:
                subview.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(bounds.size))
            case .sample:
                subview.place(at: bounds.origin, anchor: .topLeading, proposal: .unspecified)
            default: break
            }
        }
    }

    private func geometry(width: CGFloat?, subviews: Subviews) -> MarkdownTableGeometry? {
        var headings: [CGSize] = []
        var sample = CGSize.zero
        for subview in subviews {
            switch subview[TableMeasurementRole.self] {
            case .heading: headings.append(subview.sizeThatFits(.unspecified))
            case .sample: sample = subview.sizeThatFits(.unspecified)
            default: break
            }
        }
        if measureInHost {
            return nativeGeometry(width: width, headings: headings, subviews: subviews)
        }
        guard sample.width <= 1 else { return nil }
        return sizing.geometry(for: content, width: width, environment: environment,
                               headings: headings, bodyLine: sample.height)
    }

    // SwiftUI keeps TextRenderer in the view graph. When the host supplies
    // custom sizing, measure those cells in that graph using the same Text
    // views, without placing any of the measurement cells.
    private func nativeGeometry(width: CGFloat?, headings: [CGSize], subviews: Subviews) -> MarkdownTableGeometry {
        let inset = 2 * content.inset
        let columns = content.columnCount
        let share = width.map { $0 / CGFloat(max(1, columns)) }
        var widths = [Int: CGFloat]()
        var heights = [Int: CGFloat]()
        for (column, size) in headings.enumerated() { widths[column] = size.width + inset }
        for subview in subviews {
            if case let .body(_, column) = subview[TableMeasurementRole.self] {
                let size = subview.sizeThatFits(ProposedViewSize(width: share.map { max(0, $0 - inset) }, height: nil))
                widths[column] = max(widths[column, default: 0], size.width + inset)
            }
        }
        for subview in subviews {
            if case let .body(row, column) = subview[TableMeasurementRole.self] {
                let width = max(share ?? 0, widths[column, default: 0])
                let size = subview.sizeThatFits(ProposedViewSize(width: width - inset, height: nil))
                heights[row] = max(heights[row, default: 0], size.height)
            }
        }
        let headHeight = (headings.map(\.height).max() ?? 0) + 2 * content.padding(header: true) + 1
        let body = content.rows.indices.lazy.map { row in
            heights[row, default: 0] + 2 * content.padding(header: false) + (row + 1 < content.rows.count ? 1 : 0)
        }
        return MarkdownTableGeometry(
            tracks: TableTracks(minimums: (0..<columns).map { widths[$0, default: 0] }, available: width),
            rows: ViewportRows(heights: body, first: headHeight)
        )
    }
}

private struct MarkdownTableClip: View {
    let content: MarkdownTableContent
    let sizing: MarkdownTableSizing
    let size: CGSize
    let initialHeight: CGFloat
    @State private var visible: Range<CGFloat>?
    @State private var selectedRow: Int?

    var body: some View {
        ScrollView(.horizontal) {
            if let geometry = sizing.geometry(in: size, content: content) {
                MarkdownTableRows(content: content, geometry: geometry,
                                  visible: visible ?? 0..<initialHeight, selectedRow: selectedRow)
                    .padding(1)
                    .background {
                        ViewportReader { reading in
                            visible = reading?.range
                            selectedRow = reading?.selectedY.flatMap { geometry.rows.row(at: $0 - 1) }
                        }
                        .accessibilityHidden(true)
                    }
            }
        }
        .frame(width: size.width, height: size.height)
    }
}

private struct MarkdownTableRows: View {
    let content: MarkdownTableContent
    let geometry: MarkdownTableGeometry
    let visible: Range<CGFloat>
    let selectedRow: Int?

    var body: some View {
        // Convert from the scroll document to coordinates inside its border.
        let viewport = (visible.lowerBound - 1)..<(visible.upperBound - 1)
        ViewportRowStack(rows: geometry.rows, visible: viewport, retaining: selectedRow) { i, height in
            if i == 0 {
                row(content.head, height: height, header: true, divider: true)
            } else if let values = content.rows.dropFirst(i - 1).first {
                row(values, height: height, header: false, divider: i < content.rows.count)
            }
        }
        .frame(width: geometry.tracks.width, alignment: .leading)
    }

    private func row(_ values: [String], height: CGFloat, header: Bool, divider: Bool) -> some View {
        MarkdownTableRow(values: values, widths: geometry.tracks.widths, width: geometry.tracks.width,
                         height: height, inset: content.inset, header: header, divider: divider)
    }
}

private struct MarkdownTableRow: View {
    let values: [String]
    let widths: [CGFloat]
    let width: CGFloat
    let height: CGFloat
    let inset: CGFloat
    let header: Bool
    let divider: Bool
    @State private var hovered = false
    @Environment(\.intelligentTheme) private var theme

    var body: some View {
        let metrics = header ? MarkdownTableContent.heading : MarkdownTableContent.body
        let cellHeight = height - (divider ? 1 : 0)
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                ForEach(Array(zip(values, widths).enumerated()), id: \.offset) { _, pair in
                    let (text, width) = pair
                    MarkdownTableContent.words(text, metrics: metrics, theme: theme)
                        .fixedSize(horizontal: header, vertical: true)
                        .padding(.horizontal, inset)
                        .frame(width: width, height: cellHeight, alignment: .leading)
                        .background(header || hovered ? theme.surface.color.opacity(header ? 1 : 0.7) : .clear)
                }
            }
            .onHover { if !header { hovered = $0 } }
            .frame(width: width, height: cellHeight, alignment: .leading)
            if divider {
                Rectangle().fill(theme.border.color).frame(height: 1)
                    .allowsHitTesting(false).accessibilityHidden(true)
            }
        }
    }
}
