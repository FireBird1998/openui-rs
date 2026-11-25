import SwiftUI

struct MarkdownTable: View {
    let head: [String]
    let rows: [[String]]
    @State private var width: CGFloat = 0
    @State private var hoveredRow: Int?
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.intelligentViewport) private var viewport

    private var compact: Bool { viewport.map { $0.width <= 640 } ?? false }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 14)
        ScrollView(.horizontal) {
            MarkdownTableWidth(viewport: width) {
                Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                    GridRow { cells(head, header: true) }
                    rule
                    ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                        GridRow { cells(row, row: index) }
                        if index < rows.count - 1 { rule }
                    }
                }
            }
            .padding(1)
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        .onGeometryChange(for: CGFloat.self) { max(0, $0.size.width - 2) } action: { width = $0 }
        .clipContainer(to: shape)
        .overlay {
            shape.strokeBorder(theme.border.color, lineWidth: 1).allowsHitTesting(false)
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private func cells(_ values: [String], header: Bool = false, row: Int? = nil) -> some View {
        ForEach(Array(values.enumerated()), id: \.offset) { _, value in
            StyledWords(words: markdownWords(value, size: header ? 13 : 14),
                        metrics: TextMetrics(header ? 13 : 14, 1.5, weight: header ? 560 : 400), color: theme.ink)
                .fixedSize(horizontal: header, vertical: true)
                .padding(.horizontal, compact ? 12 : 16)
                .padding(.vertical, header || compact ? 11 : 13)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                .background(header ? theme.surface.color : hoveredRow == row ? theme.surface.color.opacity(0.7) : .clear)
                .onHover { if let row { hoveredRow = $0 ? row : (hoveredRow == row ? nil : hoveredRow) } }
        }
    }

    private var rule: some View {
        Rectangle().fill(theme.border.color)
            .frame(height: 1)
            .gridCellUnsizedAxes(.horizontal)
            .accessibilityHidden(true)
    }
}

// The horizontal scroll view proposes an unspecified width. Give the grid
// the viewport width so body cells wrap, growing the scroll document when
// header cells or other minimum widths cannot fit.
private struct MarkdownTableWidth: Layout {
    let viewport: CGFloat

    private var gridProposal: ProposedViewSize {
        ProposedViewSize(width: viewport > 0 ? viewport : nil, height: nil)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let grid = subviews.first else { return .zero }
        let size = grid.sizeThatFits(gridProposal)
        return CGSize(width: max(viewport, size.width), height: size.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, anchor: .topLeading, proposal: gridProposal)
    }
}
