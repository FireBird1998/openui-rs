import AppKit
@testable import OpenUIIntelligentUI
import Testing
// The mounted geometry oracle is MarkdownTable from 612b923.
import SwiftUI

private struct GridMarkdownTable: View {
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
            GridMarkdownTableWidth(viewport: width) {
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
private struct GridMarkdownTableWidth: Layout {
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

@MainActor
@Suite(.serialized)
struct MarkdownTableParityTests {
    @Observable fileprivate final class Input {
        var rows: [[String]]
        var width: CGFloat
        var minimumHeight: CGFloat?
        var mode: IntelligentUIMode = .light
        var typeSize: DynamicTypeSize = .large
        var fontSize: CGFloat = 14

        init(rows: [[String]], width: CGFloat, minimumHeight: CGFloat? = nil) {
            self.rows = rows
            self.width = width
            self.minimumHeight = minimumHeight
        }
    }

    private struct Table: View {
        let reference: Bool
        let head: [String]
        let input: Input

        var body: some View {
            Group {
                if let minimumHeight = input.minimumHeight {
                    table.textRenderer(IntrinsicText(minimumHeight: minimumHeight))
                } else {
                    table
                }
            }
            .frame(width: input.width)
            .intelligentUIMode(input.mode)
            .dynamicTypeSize(input.typeSize)
            .font(.system(size: input.fontSize))
        }

        @ViewBuilder private var table: some View {
            if reference {
                GridMarkdownTable(head: head, rows: input.rows)
            } else {
                MarkdownTable(head: head, rows: input.rows)
            }
        }
    }

    private func mount(reference: Bool, head: [String], input: Input,
                       height: CGFloat = 800, scrolling: Bool = false) async throws -> TextWindow {
        let table = Table(reference: reference, head: head, input: input)
        return try await TextWindow("root = Text(\"probe\")", width: 460, height: height,
                                    scrollable: scrolling, contentOverride: AnyView(table.intelligentUIViewport(CGSize(width: 460, height: height))))
    }

    private func frame(_ label: String, in mount: TextWindow) throws -> CGRect {
        let view = try mount.textView(label)
        return mount.host.convert(view.bounds, from: view)
    }

    private func document(_ label: String, in mount: TextWindow) throws -> NSView {
        try #require(mount.textView(label).enclosingScrollView?.documentView)
    }

    private func compare(_ labels: [String], actual: TextWindow, reference: TextWindow) throws {
        for label in labels {
            let expected = try #require(try? frame(label, in: reference), "Reference cell \(label)")
            let measured = try #require(try? frame(label, in: actual), "Candidate cell \(label)")
            #expect(abs(measured.minX - expected.minX) < 0.51, "\(label) at \(measured), expected \(expected)")
            #expect(abs(measured.minY - expected.minY) < 0.51, "\(label) at \(measured), expected \(expected)")
            #expect(measured.height == expected.height, "\(label) at \(measured), expected \(expected)")
        }
        if let first = labels.first {
            let expected = try document(first, in: reference).bounds.size
            let measured = try document(first, in: actual).bounds.size
            #expect(abs(measured.width - expected.width) < 0.51)
            #expect(abs(measured.height - expected.height) < 0.51)
        }
    }

    @Test func tracksAndLineCountsMatchGridAtThreeWidths() async throws {
        let words = "Several short words fit this column when given another line"
        let input = Input(rows: [["First", words], ["Next", "Short"]], width: 240)
        let head = ["Key", "Value"]
        let reference = try await mount(reference: true, head: head, input: input)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input)
        defer { actual.window.close() }
        for width: CGFloat in [240, 320, 150] {
            input.width = width
            await reference.settle()
            await actual.settle()
            try compare(head + ["First", words, "Next", "Short"], actual: actual, reference: reference)
            if width == 240 {
                let key = try frame("Key", in: reference)
                let value = try frame("Value", in: reference)
                #expect(value.minX == 132)
                #expect(value.minX - key.minX == 119)
                #expect(try frame(words, in: reference).height == 5 * 21)
            }
        }
    }

    @Test(arguments: [["` `"], ["First\u{2028}Second"], ["First\u{2029}Second"], ["**Strong** `code` 👩🏽‍💻"]])
    func nativeTextDeterminesRowOffsets(middle: [String]) async throws {
        let head = ["Header"]
        let input = Input(rows: [middle, ["Last"]], width: 320)
        let reference = try await mount(reference: true, head: head, input: input)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input)
        defer { actual.window.close() }
        let visible = middle.flatMap { source -> [String] in
            let words = InlineText.whitespace(markdownWords(source, size: 14), preserveLines: false)
            return words.characters.isEmpty ? [] : [String(words.characters)]
        }
        try compare(head + visible + ["Last"], actual: actual, reference: reference)
    }

    @Test func sameTableReflowsAfterHostTextSizingChanges() async throws {
        let edited = "First edited with a much wider cell"
        let input = Input(rows: [[edited, "Short"], ["Second", "Added"]], width: 180, minimumHeight: 35)
        let head = ["Key", "Value"]
        let reference = try await mount(reference: true, head: head, input: input)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input)
        defer { actual.window.close() }
        try compare(head + [edited, "Short", "Second", "Added"], actual: actual, reference: reference)

        input.minimumHeight = 85
        await reference.settle()
        await actual.settle()
        try compare(head + [edited, "Short", "Second", "Added"], actual: actual, reference: reference)

        input.width = 320
        input.mode = .dark
        input.typeSize = .accessibility3
        input.fontSize = 32
        await reference.settle()
        await actual.settle()
        try compare(head + [edited, "Short", "Second", "Added"], actual: actual, reference: reference)
    }


    @Test func longUnbrokenCellsMatchGridAt150Points() async throws {
        let word = String(repeating: "wide", count: 16)
        let head = ["A", "B", "C"]
        let input = Input(rows: [[word, word, word]], width: 150)
        let reference = try await mount(reference: true, head: head, input: input)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input)
        defer { actual.window.close() }
        try compare(head + [word], actual: actual, reference: reference)
        #expect(try frame(word, in: actual).height == 24 * 21)
    }

    @Test func editingAndAppendingRowsInvalidatesSameWidthGeometry() async throws {
        let head = ["Key", "Value"]
        let input = Input(rows: [["Short", "Marker"], ["Last", "Tail"]], width: 180)
        let reference = try await mount(reference: true, head: head, input: input)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input)
        defer { actual.window.close() }
        let labels = head + ["Marker", "Last", "Tail"]
        try compare(labels, actual: actual, reference: reference)
        let before = try frame("Last", in: actual).minY

        input.rows = [["A much longer cell that wraps onto multiple lines", "Marker"], ["Last", "Tail"], ["Added", "New"]]
        await reference.settle()
        await actual.settle()
        try compare(labels + ["Added", "New"], actual: actual, reference: reference)
        #expect(try frame("Last", in: actual).minY > before)
    }

    @Test func raggedColumnsAndWideHeadingsMatchGrid() async throws {
        let head = [String(repeating: "Wide", count: 8), "Value"]
        let input = Input(rows: [["Longer cell content", "Short", "Extra"], ["Sparse"]], width: 120)
        let reference = try await mount(reference: true, head: head, input: input)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input)
        defer { actual.window.close() }
        for width: CGFloat in [120, 297, 460] {
            input.width = width
            await reference.settle()
            await actual.settle()
            try compare(head + ["Longer cell content", "Short", "Extra", "Sparse"], actual: actual, reference: reference)
        }
    }

    @Test func selectionSurvivesRemovingOtherRowsFromTheViewport() async throws {
        let head = ["Key", "Value"]
        let input = Input(rows: (0..<200).map { ["Row \($0)", "Value\($0) tail"] }, width: 240)
        let reference = try await mount(reference: true, head: head, input: input, height: 210, scrolling: true)
        defer { reference.window.close() }
        let actual = try await mount(reference: false, head: head, input: input, height: 210, scrolling: true)
        defer { actual.window.close() }

        for mount in [reference, actual] {
            let label = "Value0 tail"
            let start = try frame(label, in: mount)
            try await mount.click(CGPoint(x: start.minX + 12, y: start.midY), count: 2)
            let cell = try mount.textView(label)
            let selected = NSRange(location: 0, length: 6)
            try #require(cell.accessibilitySelectedTextRange() == selected)
            let scroll = try #require(cell.enclosingScrollView?.enclosingScrollView)
            let clip = scroll.contentView
            let document = try #require(scroll.documentView)
            let top = clip.bounds.origin
            for y in [document.bounds.maxY - clip.bounds.height, top.y] {
                clip.scroll(to: CGPoint(x: top.x, y: y))
                scroll.reflectScrolledClipView(clip)
                await mount.settle()
            }
            let returned = try mount.textView(label)
            #expect(returned === cell)
            #expect(returned.accessibilitySelectedTextRange() == selected)
            #expect(returned.accessibilitySelectedText() == "Value0")
        }
    }
}
