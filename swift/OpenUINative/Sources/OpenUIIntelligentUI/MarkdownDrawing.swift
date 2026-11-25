import OpenUINative
import SwiftUI

struct MarkdownDrawing: View {
    let blocks: [MarkdownBlock]
    @Environment(\.intelligentTheme) private var theme

    init(element: Element) {
        blocks = MarkdownBlock.parse(element.word("text") ?? "")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(zip(blocks.indices, blocks)), id: \.0) { index, block in
                draw(block).padding(.top, block.gap(after: blocks.prefix(index).last))
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        .clipContainer()
    }

    @ViewBuilder private func draw(_ block: MarkdownBlock) -> some View {
        switch block {
        case let .heading(level, text):
            let metrics = switch level {
            case 1: TextMetrics(22, 1.3, weight: 640, tracking: -0.5)
            case 2: TextMetrics(19, 1.35, weight: 640, tracking: -0.3)
            default: TextMetrics(16, 1.4, weight: 640)
            }
            let heading: AccessibilityHeadingLevel = switch level {
            case 1: .h2
            case 2: .h3
            case 3: .h4
            default: .h5
            }
            words(text, metrics: metrics).accessibilityHeading(heading)
        case let .paragraph(text):
            words(text)
        case let .quote(text):
            words(text)
                .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                .clipContainer()
                .padding(.vertical, 12).padding(.leading, 19).padding(.trailing, 16)
                .background(theme.surface.color, in: UnevenRoundedRectangle(bottomTrailingRadius: 12, topTrailingRadius: 12))
                .overlay(alignment: .leading) {
                    Rectangle().fill(theme.borderStrong.color).frame(width: 3).allowsHitTesting(false)
                }
        case let .list(start, items):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(items.enumerated()), id: \.offset) { offset, item in
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        let marker = start.map { "\($0 + offset)." } ?? "•"
                        StyledWords(AttributedString(marker), style: WordStyle(
                            metrics: TextMetrics(16, 1.65, weight: start == nil ? 400 : 560),
                            color: theme.muted, tabular: start != nil))
                            .fixedSize()
                            .frame(width: 20, alignment: .trailing)
                        words(item)
                    }
                }
            }
            .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
            .clipContainer()
            .accessibilityElement(children: .contain)
        case let .code(text):
            let shape = RoundedRectangle(cornerRadius: theme.radiusMd)
            ScrollView(.horizontal) {
                var literal = AttributedString(text)
                let _ = literal.font = Font(NSFont.monospacedSystemFont(ofSize: 13, weight: .regular))
                StyledWords(literal, style: WordStyle(metrics: TextMetrics(13, 1.6), color: theme.ink), preserveLines: nil)
                    .fixedSize()
                    .padding(.vertical, 14).padding(.horizontal, 16)
            }
            .fixedSize(horizontal: false, vertical: true)
            .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
            .background(theme.surface.color, in: shape)
            .clipContainer(to: shape)
        case .rule:
            Rectangle().fill(theme.border.color)
                .frame(height: 1).padding(.bottom, 20)
                .accessibilityHidden(true)
        case let .table(head, rows):
            MarkdownTable(head: head, rows: rows)
        }
    }

    private func words(_ text: String, metrics: TextMetrics = TextMetrics(16, 1.65)) -> some View {
        StyledWords(words: markdownWords(text, size: metrics.size), metrics: metrics, color: theme.ink)
    }
}

private extension MarkdownBlock {
    func gap(after previous: Self?) -> CGFloat {
        if case .heading = previous { return 8 }
        if case .rule = self { return 20 }
        guard let previous else { return 0 }
        if case .heading = self { return 26 }
        if case .paragraph = previous, case .list = self { return 6 }
        return 12
    }
}

func markdownWords(_ source: String, size: CGFloat) -> AttributedString {
    var words = InlineText.parse(source)
    for run in words.runs where run.inlinePresentationIntent?.contains(.code) == true {
        let strong = run.inlinePresentationIntent?.contains(.stronglyEmphasized) == true
        words[run.range].font = Font(NSFont.monospacedSystemFont(ofSize: size * 0.88, weight: strong ? .semibold : .regular))
    }
    return words
}
