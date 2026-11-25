import AppKit
import SwiftUI

struct MarkdownTableContent: Equatable {
    let head: [String]
    let rows: [[String]]
    let compact: Bool

    var inset: CGFloat { compact ? 12 : 16 }
    func padding(header: Bool) -> CGFloat { header || compact ? 11 : 13 }
    var columnCount: Int { rows.reduce(head.count) { max($0, $1.count) } }

    static let heading = TextMetrics(13, 1.5, weight: 560)
    static let body = TextMetrics(14, 1.5)

    @MainActor static func words(_ source: String, metrics: TextMetrics, theme: IntelligentTheme) -> StyledWords {
        StyledWords(words: markdownWords(source, size: metrics.size), metrics: metrics, color: theme.ink)
    }
}

struct MarkdownTableGeometry: Equatable {
    let tracks: TableTracks
    let rows: ViewportRows
}

struct MarkdownTableTextEnvironment: Equatable {
    let dynamicType: DynamicTypeSize
    let font: Font?
    let locale: Locale
    let textCase: Text.Case?
    let layoutDirection: LayoutDirection
    let lineLimit: Int?
    let minimumScaleFactor: CGFloat
    let allowsTightening: Bool
    let displayScale: CGFloat
    let theme: IntelligentTheme

    init(_ environment: EnvironmentValues) {
        dynamicType = environment.dynamicTypeSize
        font = environment.font
        locale = environment.locale
        textCase = environment.textCase
        layoutDirection = environment.layoutDirection
        lineLimit = environment.lineLimit
        minimumScaleFactor = environment.minimumScaleFactor
        allowsTightening = environment.allowsTightening
        displayScale = environment.displayScale
        theme = environment.intelligentTheme
    }
}

/// The layout and scroll content share geometry. Only the current text,
/// environment, and a few width proposals are retained.
@MainActor @Observable
final class MarkdownTableSizing {
    private struct Input: Equatable {
        let content: MarkdownTableContent
        let environment: MarkdownTableTextEnvironment
        let headings: [CGSize]
        let bodyLine: CGFloat
    }

    private struct Placement: Equatable {
        let size: CGSize
        let content: MarkdownTableContent
        let geometry: MarkdownTableGeometry?
    }

    @ObservationIgnored private var input: Input?
    @ObservationIgnored private var proposals: [(width: CGFloat?, geometry: MarkdownTableGeometry)] = []
    @ObservationIgnored private var placements: [Placement] = []
    @ObservationIgnored private var drawnSize: CGSize?
    @ObservationIgnored private var publishing = false
    private var revision = 0

    func geometry(for content: MarkdownTableContent, width: CGFloat?, environment: EnvironmentValues,
                  headings: [CGSize], bodyLine: CGFloat) -> MarkdownTableGeometry {
        let input = Input(content: content, environment: MarkdownTableTextEnvironment(environment),
                          headings: headings, bodyLine: bodyLine)
        if input != self.input {
            self.input = input
            proposals.removeAll(keepingCapacity: true)
        }
        if let known = proposals.first(where: { $0.width == width }) { return known.geometry }
        let geometry = measure(input: input, width: width, environment: environment)
        if proposals.count == 3 { proposals.removeFirst() }
        proposals.append((width, geometry))
        return geometry
    }

    func geometry(in size: CGSize, content: MarkdownTableContent) -> MarkdownTableGeometry? {
        // A stack can place its children at several trial sizes before the
        // reader gets the chosen size. Keep that placement across probes.
        _ = revision
        drawnSize = size
        return placements.first { $0.size == size && $0.content == content }?.geometry
    }

    func place(_ geometry: MarkdownTableGeometry?, in size: CGSize, content: MarkdownTableContent) {
        let next = Placement(size: size, content: content, geometry: geometry)
        if let match = placements.firstIndex(where: { $0.size == size }) {
            guard placements.dropFirst(match).first != next else { return }
            placements.remove(at: match)
        }
        if placements.count == 4,
           let discarded = placements.firstIndex(where: { $0.size != drawnSize }) {
            placements.remove(at: discarded)
        }
        placements.append(next)
        guard drawnSize == size, !publishing else { return }
        publishing = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            // Notify the reader after SwiftUI finishes the current placement.
            revision &+= 1
            publishing = false
        }
    }

    private func measure(input: Input, width: CGFloat?, environment: EnvironmentValues) -> MarkdownTableGeometry {
        let content = input.content
        let inset = 2 * content.inset
        let columns = content.columnCount
        let share = width.map { $0 / CGFloat(max(1, columns)) }
        var minimums: [Int: CGFloat] = [:]
        for (column, size) in input.headings.enumerated() { minimums[column] = size.width + inset }
        func tracks() -> TableTracks {
            TableTracks(minimums: (0..<columns).map { minimums[$0, default: 0] }, available: width)
        }
        let headingHeight = (input.headings.map(\.height).max() ?? 0) + 2 * content.padding(header: true) + 1
        let rowPadding = 2 * content.padding(header: false)
        var plain = SingleLineCells(available: (share ?? 0) - inset, textCase: input.environment.textCase)
        func rowHeight(_ index: Int, _ textHeight: CGFloat) -> CGFloat {
            textHeight + rowPadding + (index + 1 < content.rows.count ? 1 : 0)
        }

        // All plain cells which fit are the same native single-line height.
        // The Core Text check only proves that the line fits. Other heights
        // come from the SwiftUI text engine at the chosen proposal.
        if share != nil, (share ?? 0) >= inset, content.rows.allSatisfy({ $0.allSatisfy { $0.isEmpty || plain.fits($0) } }) {
            let heights = content.rows.lazy.enumerated().map { index, row in
                rowHeight(index, row.contains { !$0.isEmpty } ? input.bodyLine : 0)
            }
            return MarkdownTableGeometry(tracks: tracks(),
                                         rows: ViewportRows(heights: heights, first: headingHeight))
        }

        var heights: [CGFloat] = []
        let sample = Canvas { context, _ in
            var measurer = TableTextContext(context: context, theme: input.environment.theme)
            // Grid gives every column the same finite proposal. A cell can
            // enlarge its track when it can't fit that proposal.
            if share == nil || (share ?? 0) < inset + 1 {
                for row in content.rows {
                    for (column, text) in row.enumerated() {
                        let measured = measurer.size(text, width: share.map { max(0, $0 - inset) })
                        minimums[column] = max(minimums[column, default: 0], measured.width + inset)
                    }
                }
            }
            let widths = tracks().widths
            heights = content.rows.enumerated().map { index, row in
                let height = zip(row, widths).reduce(CGFloat.zero) { result, cell in
                    let (text, width) = cell
                    if text.isEmpty { return result }
                    if plain.fits(text) { return max(result, input.bodyLine) }
                    return max(result, measurer.size(text, width: max(0, width - inset)).height)
                }
                return rowHeight(index, height)
            }
        }
        .frame(width: 1, height: 1)
        .lineHeight(MarkdownTableContent.body.height.map { .exact(points: $0) })
        .environment(\.self, environment)

        // A single canvas supplies the SwiftUI text context. It resolves and
        // measures text, and does not draw cells or place any row views.
        _ = ImageRenderer(content: sample).cgImage
        return MarkdownTableGeometry(tracks: tracks(),
                                     rows: ViewportRows(heights: heights, first: headingHeight))
    }
}

private struct SingleLineCells {
    let available: CGFloat
    let textCase: Text.Case?
    private let font = layoutPlatformFont(size: MarkdownTableContent.body.size, weight: MarkdownTableContent.body.weight)

    private var widths: [String: CGFloat] = [:]

    mutating func fits(_ text: String) -> Bool {
        if let known = widths[text] { return known <= available }
        // No markup, normalization, forced breaks, or locale casing. Leave
        // room for SwiftUI's pixel rounding before sharing the line height.
        guard textCase == nil, available >= 4, !text.isEmpty,
              text.first != " ", text.last != " ", !text.contains("  "),
              text.utf8.allSatisfy({ (32...126).contains($0) && $0 != 42
                  && $0 != 91 && $0 != 95 && $0 != 96 && $0 != 126 }) else { return false }
        let line = CTLineCreateWithAttributedString(NSAttributedString(
            string: text, attributes: [NSAttributedString.Key(kCTFontAttributeName as String): font]))
        let width = CTLineGetTypographicBounds(line, nil, nil, nil).rounded(.up) + 2
        if widths.count < 512, text.utf8.count <= 256 { widths[text] = width }
        return width <= available
    }
}

@MainActor private struct TableTextContext {
    struct Key: Hashable {
        let text: String
        let width: CGFloat?
    }

    let context: GraphicsContext
    let theme: IntelligentTheme
    private var sizes: [Key: CGSize] = [:]

    init(context: GraphicsContext, theme: IntelligentTheme) {
        self.context = context
        self.theme = theme
    }

    mutating func size(_ source: String, width: CGFloat?) -> CGSize {
        let key = Key(text: source, width: width)
        if let known = sizes[key] { return known }
        let words = MarkdownTableContent.words(source, metrics: MarkdownTableContent.body, theme: theme)
        let (text, empty) = words.renderedText(theme: theme)
        let resolved = context.resolve(text)
        let measured = resolved.measure(in: CGSize(width: width ?? .infinity, height: .infinity))
        let size = CGSize(width: measured.width, height: empty ? 0 : measured.height)
        if sizes.count < 512, source.utf8.count <= 256 { sizes[key] = size }
        return size
    }
}
