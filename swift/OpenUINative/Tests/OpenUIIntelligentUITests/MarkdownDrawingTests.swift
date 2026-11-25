import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

@Suite(.serialized) @MainActor struct MarkdownDrawingTests {
    private static let document = """
        # Head one
        Paragraph **bold**, *italic*, ~~strike~~ and `inline`.
        - Bullet
          continued
        * Another
        + Last

        7) Ordered
        1. Next

        > Quoted
        > lines

        ```swift
            let value = 42
        ```

        | Name | Value |
        | ---: | :--- |
        | Answer | 42 |

        _ _ _
        ## Head two
        ### Head three
        #### Head four
        """

    @Test func everyMarkdownBlockUsesTheUpstreamStructure() {
        #expect(MarkdownBlock.parse(Self.document) == [
            .heading(1, "Head one"),
            .paragraph("Paragraph **bold**, *italic*, ~~strike~~ and `inline`."),
            .list(start: nil, items: ["Bullet continued", "Another", "Last"]),
            .list(start: 7, items: ["Ordered", "Next"]),
            .quote("Quoted lines"),
            .code("    let value = 42"),
            .table(head: ["Name", "Value"], rows: [["Answer", "42"]]),
            .rule,
            .heading(2, "Head two"), .heading(3, "Head three"), .heading(4, "Head four"),
        ])
    }

    @Test func upstreamBlockBoundariesPreserveStreamingAndRaggedInput() {
        let source = "intro\r\n---\r\n| A | B |\r\n| - | - |\r\n| only |\r\n\r\n"
            + "| A | B |\r\n| - | - |\r\n| only |\r\n| one | two | three |\r\n\r\n"
            + "> first\r  >> second\r\r~~~\r\r````js\rraw *text*\r"
        #expect(MarkdownBlock.parse(source) == [
            .paragraph("intro --- | A | B | | - | - | | only |"),
            .table(head: ["A", "B"], rows: [["only"], ["one", "two", "three"]]),
            .quote("first > second"),
            .paragraph("~~~"),
            .code("raw *text*\n"),
        ])
    }

    @Test func contrivedInputsKeepTheBlockParserMoving() {
        #expect(MarkdownBlock.parse("# first\u{2028}second\n# Next") == [
            .paragraph("# first\u{2028}second"), .heading(1, "Next"),
        ])
        #expect(MarkdownBlock.parse("9007199254740992. Large\n1. Next") == [
            .list(start: 1, items: ["Large", "Next"]),
        ])
    }

    @Test func everyMarkdownBlockDrawsWithItsTextAndType() async throws {
        let window = try await TextWindow(markdownProgram(Self.document), height: 800)
        defer { window.window.close() }
        let expected: [(String, CGFloat)] = [
            ("Head one", 22), ("Head two", 19), ("Head three", 16), ("Head four", 16),
            ("Paragraph bold, italic, strike and inline.", 16),
            ("Bullet continued", 16), ("Another", 16), ("Last", 16), ("Ordered", 16), ("Next", 16),
            ("Quoted lines", 16), ("    let value = 42", 13), ("Name", 13), ("Value", 13), ("Answer", 14), ("42", 14),
        ]
        for (label, size) in expected {
            let value = try window.attributed(label)
            let font = try #require(value.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
            #expect(font.pointSize == size, "\(label)")
        }
        let paragraph = try window.attributed("Paragraph bold, italic, strike and inline.")
        let plain = paragraph.string as NSString
        let strong = try #require(paragraph.attribute(.font, at: plain.range(of: "bold").location, effectiveRange: nil) as? NSFont)
        let emphasis = try #require(paragraph.attribute(.font, at: plain.range(of: "italic").location, effectiveRange: nil) as? NSFont)
        let code = try #require(paragraph.attribute(.font, at: plain.range(of: "inline").location, effectiveRange: nil) as? NSFont)
        #expect((CTFontCopyVariation(strong) as? [UInt32: NSNumber])?[0x77676874]?.intValue == 640)
        #expect(CTFontGetSymbolicTraits(emphasis).contains(.traitItalic))
        #expect(paragraph.attribute(.strikethroughStyle, at: plain.range(of: "strike").location, effectiveRange: nil) != nil)
        #expect(CTFontGetSymbolicTraits(code).contains(.traitMonoSpace))
        let fenced = try window.attributed("    let value = 42")
        let mono = try #require(fenced.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
        #expect(CTFontGetSymbolicTraits(mono).contains(.traitMonoSpace))
        for marker in ["•", "7.", "8."] { _ = try window.attributed(marker) }
    }

    @Test func malformedMarkdownDrawsLiteralText() async throws {
        let malformed = "##### Unsupported\n\n**unclosed [link](oops\n\n<img src=x> &amp;"
        let window = try await TextWindow(markdownProgram(malformed))
        defer { window.window.close() }
        for text in ["##### Unsupported", "**unclosed [link](oops", "<img src=x> &amp;"] {
            #expect(try window.attributed(text).string == text)
        }
    }

    @Test func deeplyNestedMarkdownStaysFlatAndDrawsTheRemainingText() async throws {
        let quotes = String(repeating: "> ", count: 20_000) + "end"
        let list = (0..<128).map { String(repeating: "  ", count: $0) + "- item" }.joined(separator: "\n")
        #expect(MarkdownBlock.parse(quotes) == [.quote(String(quotes.dropFirst(2)))])
        let expectedTail = Array(repeating: "- item", count: 126).joined(separator: " ")
        #expect(MarkdownBlock.parse(list) == [.list(start: nil, items: ["item", "item " + expectedTail])])
        let window = try await TextWindow(markdownProgram(String(repeating: "> ", count: 1_024) + "end\n\nAfter"))
        defer { window.window.close() }
        #expect(try window.attributed("After").string == "After")
    }
}

private func markdownProgram(_ text: String) throws -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = .withoutEscapingSlashes
    let prop = String(decoding: try encoder.encode(text), as: UTF8.self)
    return "root = Col([Markdown(\(prop))], 0)"
}

extension MarkdownDrawingTests {
    @Test func markdownRulePaintsAcrossItsMeasuredWidth() async throws {
        let picture = try await LayoutPicture(markdownProgram("---"))
        let frame = try picture.rect("/children/0")
        #expect(frame.height == 41)
        let ink = try LayoutPicture.swatch(IntelligentTheme.light.border.color)
        for x in [frame.minX + frame.width / 4, frame.maxX - frame.width / 4] {
            #expect(try picture.pixel(Int(x), Int(frame.midY)) == ink)
            #expect(try picture.pixel(Int(x), Int(frame.midY - 3)).alphaComponent == 0)
        }
    }

    @Test func markdownLinksOnlyOpenAbsoluteWebURLs() async throws {
        let text = "[web](https://example.org/one)\n\n[relative](/no)\n\n[unsafe](javascript:no)"
        let window = try await TextWindow(markdownProgram(text))
        defer { window.window.close() }
        for label in ["web", "relative", "unsafe"] {
            let r = try frame(of: label, in: window)
            try await window.click(CGPoint(x: r.midX, y: r.midY))
        }
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/one"])
    }

    @Test func markdownAndItsQuoteAndListClipOverflowingLinks() async throws {
        let label = String(repeating: "wide", count: 24)
        let link = "[\(label)](https://example.org/wide)"
        for (prefix, rightPadding) in [("", 0.0), ("> ", 16.0), ("- ", 0.0)] {
            let window = try await TextWindow(markdownProgram(prefix + link), renderer: IntrinsicText(), markdownWidth: 120)
            defer { window.window.close() }
            let bounds = try #require(window.frames["/children/0"])
            let text = try frame(of: label, in: window)
            let edge = bounds.maxX - rightPadding
            try #require(text.maxX > edge + 4)
            try await window.click(CGPoint(x: edge + 4, y: text.midY))
            #expect(window.opened.isEmpty, "\(prefix)")
            try await window.click(CGPoint(x: text.minX + 6, y: text.midY))
            #expect(window.opened.map(\.absoluteString) == ["https://example.org/wide"], "\(prefix)")
        }
    }

    @Test func markdownTableScrollsAndOnlyVisibleLinksOpen() async throws {
        let label = String(repeating: "wide", count: 24)
        let source = "| [\(label)](https://example.org/left) | [Right](https://example.org/right) |\n"
            + "| --- | --- |\n| First | Last |"
        let window = try await TextWindow(markdownProgram(source), markdownWidth: 140)
        defer { window.window.close() }
        let text = try window.textView(label)
        let scroll = try #require(text.enclosingScrollView)
        let bounds = window.host.convert(scroll.contentView.bounds, from: scroll.contentView)
        let link = try frame(of: label, in: window)
        try #require(link.maxX > bounds.maxX + 4)
        try await window.click(CGPoint(x: bounds.maxX + 4, y: link.midY))
        #expect(window.opened.isEmpty)
        try await window.click(CGPoint(x: link.minX + 6, y: link.midY))
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/left"])

        let content = try #require(scroll.documentView)
        let farRight = content.bounds.maxX - scroll.contentSize.width
        try #require(farRight > 0)
        scroll.contentView.scroll(to: CGPoint(x: farRight, y: scroll.contentView.bounds.minY))
        scroll.reflectScrolledClipView(scroll.contentView)
        await window.settle()
        let right = try frame(of: "Right", in: window)
        try #require(bounds.contains(CGPoint(x: right.midX, y: right.midY)))
        try await window.click(CGPoint(x: right.midX, y: right.midY))
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/left", "https://example.org/right"])
    }

    @Test func markdownTableAlignsTracksAndWrapsBodyCellsBeforeScrolling() async throws {
        let words = "Several short words fit this column when given another line"
        let text = "| Key | Value |\n| ---: | :---: |\n| First | \(words) |\n| Next | Short |"
        let window = try await TextWindow(markdownProgram(text), markdownWidth: 240)
        defer { window.window.close() }
        let key = try frame(of: "Key", in: window)
        let value = try frame(of: "Value", in: window)
        let first = try frame(of: "First", in: window)
        let wrapped = try frame(of: words, in: window)
        let short = try frame(of: "Short", in: window)
        #expect(abs(first.minX - key.minX) <= 1)
        #expect(abs(wrapped.minX - value.minX) <= 1)
        #expect(abs(short.minX - value.minX) <= 1)
        #expect(wrapped.height > 2 * short.height)
        let view = try window.textView(words)
        let scroll = try #require(view.enclosingScrollView)
        let document = try #require(scroll.documentView)
        #expect(abs(document.bounds.width - scroll.contentSize.width) <= 1)
    }

    @Test func markdownCodeScrollsAndOnlyVisibleTextCanBeSelected() async throws {
        let label = String(repeating: "wide", count: 30)
        let window = try await TextWindow(markdownProgram("```\n\(label)\n```"), markdownWidth: 140)
        defer { window.window.close() }
        let text = try window.textView(label)
        let scroll = try #require(text.enclosingScrollView)
        let viewport = window.host.convert(scroll.contentView.bounds, from: scroll.contentView)
        let code = try frame(of: label, in: window)
        try #require(code.maxX > viewport.maxX + 4)
        try await window.click(CGPoint(x: viewport.maxX + 4, y: code.midY), count: 2)
        #expect(text.accessibilitySelectedText()?.isEmpty != false)
        try await window.click(CGPoint(x: code.minX + 6, y: code.midY), count: 2)
        #expect(text.accessibilitySelectedText() == label)
        let content = try #require(scroll.documentView)
        let farRight = content.bounds.maxX - scroll.contentSize.width
        try #require(farRight > 0)
        scroll.contentView.scroll(to: CGPoint(x: farRight, y: scroll.contentView.bounds.minY))
        scroll.reflectScrolledClipView(scroll.contentView)
        await window.settle()
        #expect(try frame(of: label, in: window).minX < code.minX)
    }

    private func frame(of label: String, in window: TextWindow) throws -> CGRect {
        let view = try window.textView(label)
        return window.host.convert(view.bounds, from: view)
    }
}
