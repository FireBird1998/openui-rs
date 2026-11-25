import Foundation
@testable import OpenUIIntelligentUI
import Testing

struct MarkdownReplayTests {
    private struct Recording: Decodable {
        var cases: [Sample]
        var textComponents: [TextComponent]
    }

    private struct Sample: Decodable {
        var label: String
        var source: String
        var inline: [Inline]?
        var blocks: [Block]?
    }

    private struct Inline: Decodable {
        enum Kind: String, Decodable {
            case text, strong, em, s, code, a
        }

        var kind: Kind
        var text: String? = nil
        var target: String? = nil
        var children: [Self]? = nil

        private var decoration: InlinePresentationIntent {
            switch kind {
            case .strong: .stronglyEmphasized
            case .em: .emphasized
            case .s: .strikethrough
            case .code: .code
            default: []
            }
        }

        func append(events: inout [InlineText.Event], words: inout AttributedString,
                    intent: InlinePresentationIntent = [], link: URL? = nil) {
            switch kind {
            case .text:
                guard let text else { return }
                events.append(.text(text))
                var part = AttributedString(text)
                if !intent.isEmpty { part.inlinePresentationIntent = intent }
                part.link = link
                words.append(part)
            case .strong, .em, .s, .code, .a:
                events.append(.open(decoration, target: target))
                let url = target.flatMap(webURL)
                for child in children ?? [] {
                    child.append(events: &events, words: &words, intent: intent.union(decoration), link: url ?? link)
                }
                events.append(.close)
            }
        }
    }

    private struct Span: Decodable {
        var text: String
        var inline: [Inline]
    }

    private struct Block: Decodable {
        enum Kind: String, Decodable {
            case h, p, quote, list, code, hr, table
        }

        var kind: Kind
        var level: Int?
        var content: Span?
        var ordered: Bool?
        var start: Int?
        var items: [Span]?
        var text: String?
        var head: [Span]?
        var rows: [[Span]]?

        func parsed() throws -> MarkdownBlock {
            switch kind {
            case .h: return .heading(try #require(level), try #require(content).text)
            case .p: return .paragraph(try #require(content).text)
            case .quote: return .quote(try #require(content).text)
            case .list:
                let ordinal: Int? = if ordered == true { try #require(start) as Int } else { nil }
                return .list(start: ordinal, items: try #require(items).map(\.text))
            case .code: return .code(try #require(text))
            case .hr: return .rule
            case .table: return .table(head: try #require(head).map(\.text), rows: try #require(rows).map { $0.map(\.text) })
            }
        }

        var spans: [Span] {
            [content].compactMap { $0 } + (items ?? []) + (head ?? []) + (rows ?? []).flatMap { $0 }
        }
    }

    private struct TextComponent: Decodable {
        struct Part: Decodable {
            var text: String
            var inline: [Inline]?
            var component: String?
        }

        var component: String
        var prop: String
        var parts: [Part]
    }

    private func recording() throws -> Recording {
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let url = directory.appendingPathComponent("conformance/golden/intelligent-ui-markdown.json")
        return try JSONDecoder().decode(Recording.self, from: Data(contentsOf: url))
    }

    private func check(_ source: String, _ nodes: [Inline], _ label: String) {
        var actualEvents: [InlineText.Event] = []
        let actual = InlineText.parse(source) { actualEvents.append($0) }
        var expectedEvents: [InlineText.Event] = []
        var expected = AttributedString()
        for node in nodes { node.append(events: &expectedEvents, words: &expected) }
        if actualEvents != expectedEvents {
            Issue.record("\(label) \(source.debugDescription)\nactual inline: \(actualEvents)\nupstream: \(expectedEvents)")
        }
        if actual != expected || !String(actual.characters).utf8.elementsEqual(String(expected.characters).utf8) {
            Issue.record("\(label) \(source.debugDescription)\nactual: \(actual)\nupstream: \(expected)")
        }
    }

    @Test func replaysPinnedMarkdownBlocksAndInline() throws {
        let corpus = try recording().cases
        #expect(corpus.filter { $0.label == "seeded" }.count == 4_096)
        for sample in corpus {
            if let inline = sample.inline { check(sample.source, inline, sample.label) }
            if let blocks = sample.blocks {
                let actual = MarkdownBlock.parse(sample.source)
                let expected = try blocks.map { try $0.parsed() }
                if actual != expected {
                    Issue.record("\(sample.label) \(sample.source.debugDescription)\nactual blocks: \(actual)\nupstream: \(expected)")
                    continue
                }
                for (parsed, recorded) in zip(actual, blocks) {
                    let spans = recorded.spans
                    let sources = parsed.inlineSources
                    #expect(sources.count == spans.count)
                    for (source, span) in zip(sources, spans) {
                        #expect(source.utf8.elementsEqual(span.text.utf8))
                        check(source, span.inline, "\(sample.label) block inline")
                    }
                }
            }
        }
    }

    @Test func replaysInlineTextSlotsFrom169() throws {
        let components = try recording().textComponents
        #expect(Set(components.map(\.component)) == ["Title", "Text", "Caption", "Quote", "List", "ListItem"])
        for component in components {
            for part in component.parts {
                if let inline = part.inline {
                    #expect(part.component == nil)
                    check(part.text, inline, "\(component.component).\(component.prop)")
                } else {
                    #expect(part.component == "Badge")
                }
            }
        }
    }
}

private extension MarkdownBlock {
    var inlineSources: [String] {
        switch self {
        case let .heading(_, text), let .paragraph(text), let .quote(text): [text]
        case let .list(_, items): items
        case let .table(head, rows): head + rows.flatMap { $0 }
        case .code, .rule: []
        }
    }
}
