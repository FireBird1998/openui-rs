import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

@Suite(.serialized) @MainActor struct TextDrawingTests {
    @Test func inlineFormattingPreservesLiteralCodeAndFormatsLinkLabels() throws {
        let value = InlineText.parse("A __bold *both*__ ~~gone~~ `*code*` [**site**](https://example.org) [local](/no) <b>x</b>")
        #expect(String(value.characters) == "A bold both gone *code* site local <b>x</b>")
        func intent(_ word: String) throws -> InlinePresentationIntent {
            let range = try #require(value.range(of: word))
            return value[range].inlinePresentationIntent ?? []
        }
        #expect(try intent("bold") == .stronglyEmphasized)
        #expect(try intent("both") == [.stronglyEmphasized, .emphasized])
        #expect(try intent("gone") == .strikethrough)
        #expect(try intent("*code*") == .code)
        #expect(try intent("site") == .stronglyEmphasized)
        #expect(value.runs.compactMap(\.link).map(\.absoluteString) == ["https://example.org"])
    }
}

extension TextDrawingTests {
    @Test func titleTextCaptionAndLabelUseTheirLineMetrics() async throws {
        let source = """
        root = Col([Title("A","sm"), Title("A"), Title("A","lg"), Title("A","xl"), Title("A","2xl"), Title("A","3xl"), Text("A","xs"), Text("A","sm"), Text("A","md"), Text("A","lg"), Caption("A"), Label("A")], 0)
        """
        for width: CGFloat in [640, 641] {
            let picture = try await LayoutPicture(source, width: width, height: 400)
            let small = width == 640
            let heights = [22.5, 23.8, small ? 24.7 : 27.3, 30, small ? 32.2 : 36.8,
                           small ? 37.4 : 44, 19.2, 21.7, 25.6, 27.9, 19.2, 24]
            for (i, height) in heights.enumerated() {
                #expect(try abs(picture.rect("/children/\(i)").height - height) <= 1)
            }
        }
    }

    @Test func paragraphsWrapPreserveNewlinesAndClampRoundedMaxLines() async throws {
        let picture = try await LayoutPicture(#"""
        root = Col([Text("a\nb\nc", "md"), Text("a\nb\nc", "md", null, null, false, false, false, 1.5)], 0)
        """#, height: 200)
        #expect(try abs(picture.rect("/children/0").height - 3 * 25.6) < 1.5)
        #expect(try abs(picture.rect("/children/1").height - 2 * 25.6) < 1.5)
    }

    @Test func onlyAbsoluteWebDestinationsBecomeLinks() {
        for value in ["/here", "//example.com", "javascript:alert(1)", "file:///tmp/f", "mailto:x@y.com",
                      "https://", "https://bad host", "http://example.org:99999", "http://%2Fexample.org"] {
            #expect(webURL(value) == nil, "\(value)")
        }
        #expect(webURL("  HTTPS://example.org/a?b=2#c  ")?.host() == "example.org")
        #expect(webURL("http://example.org:80")?.scheme == "http")
    }
}

extension TextDrawingTests {
    @Test func badgesQuotesAndNestedInlineComponentsDrawInOrder() async throws {
        let source = #"""
        root = Col([Text(["In ", Badge("Season", "success", "clock"), " near ", Badge("Town")]), Quote("A short quote", "Source")], 0)
        """#
        let wide = try await LayoutPicture(source, width: 280)
        let narrow = try await LayoutPicture(source, width: 110)
        let first = try wide.rect("/children/0/text/1")
        let second = try wide.rect("/children/0/text/3")
        #expect(first.height == 26)
        #expect(first.minX > 10 && second.minX > first.maxX + 15)
        #expect(first.midY == second.midY)
        #expect(try narrow.rect("/children/0/text/3").minY >= narrow.rect("/children/0/text/1").maxY)
        let quote = try wide.rect("/children/1")
        #expect(quote.height > 70)
        let rule = try wide.pixel(Int(quote.minX + 1), Int(quote.midY))
        let surface = try wide.pixel(Int(quote.minX + 6), Int(quote.midY))
        #expect(rule.alphaComponent > 0.95)
        #expect(abs(surface.alphaComponent - 0.04) < 0.01)
    }

    @Test func listsAllocateMarkerLanesAndKeepNestedChildPaths() async throws {
        let source = #"""
        root = Col([List([a,b]), List([a,b],"number"), List([a,b],"none"), List([c,b],"bullet",true), ListItem("Alone","detail","A",[Divider()])],0)
        a = ListItem("Start","detail",null,[Divider()])
        b = ListItem("End")
        c = ListItem("Morning",null,"08:30",[Divider()])
        """#
        let pic = try await LayoutPicture(source, height: 500)
        for (index, x): (Int, CGFloat) in [(0,22), (1,36), (2,0), (3,84)] {
            let path = "/children/\(index)/items/0"
            let item = try pic.rect(path)
            let child = try pic.rect(path + "/children/0")
            #expect(item.minX == x)
            #expect(child.minX == x && child.minY > item.minY)
            #expect(try pic.rect("/children/\(index)/items/1").minY >= item.maxY + (index == 3 ? 18 : 10))
        }
        #expect(try pic.rect("/children/4/children/0").minX == 0)
    }
}

extension TextDrawingTests {
    @Test func webURLsUseTheBrowserAuthorityAndRejectMalformedHostsAndPorts() {
        for source in ["https://[abc]", "http://1.2.3.999", "http://example.org:999999999999999999999",
                       "http://ex%3Aample.org", "http://ex%00ample.org", "https://a.09"] {
            #expect(webURL(source) == nil, "\(source)")
        }
        #expect(webURL("https:example.org/page")?.absoluteString == "https://example.org/page")
        #expect(webURL(#"https:\example.org\page"#)?.absoluteString == "https://example.org/page")
        #expect(webURL("http://127.1/")?.host() == "127.0.0.1")
        #expect(webURL("http://[::1]:8080/")?.port == 8080)
    }
}

extension TextDrawingTests {
    @Test func whitespaceCollapsesAcrossFormattingWithoutChangingTokenBoundaries() {
        let source = "  first   **two **\t words \r\n   *third\nline*  "
        let words = InlineText.parse(source)
        #expect(String(InlineText.whitespace(words, preserveLines: true).characters) == "first two words\n*third\nline*")
        #expect(String(InlineText.whitespace(words, preserveLines: false).characters) == "first two words *third line*")
    }
}

extension TextDrawingTests {
    @Test func badgeTonesAndLeadingGlyphUseTheirThemeAndPillMetrics() async throws {
        let source = #"""
        root = Col([Badge("Tag"), Badge("Tag","info","clock"), Badge("Tag","success"), Badge("Tag","warning"), Badge("Tag","danger"), Badge("Tag","purple"), Badge("Tag","pink"), Badge("Tag","neutral","unrecognised-name")], 0, "start")
        """#
        for mode: IntelligentUIMode in [.light, .dark] {
            let pic = try await LayoutPicture(source, height: 230, mode: mode)
            let theme = mode == .light ? IntelligentTheme.light : .dark
            let fills = [theme.badgeNeutralSoft, theme.infoSoft, theme.badgeSuccessSoft, theme.badgeWarningSoft,
                         theme.badgeDangerSoft, theme.badgePurpleSoft, theme.badgePinkSoft]
            for (i, fill) in fills.enumerated() {
                let frame = try pic.rect("/children/\(i)")
                #expect(frame.height == 26)
                let paint = try pic.pixel(Int(frame.maxX) - 4, Int(frame.midY))
                let expected = fill.sRGB
                #expect(abs(paint.alphaComponent - expected.alphaComponent) < 0.006)
                #expect(abs(paint.redComponent - expected.redComponent) < 0.045)
                #expect(abs(paint.greenComponent - expected.greenComponent) < 0.045)
                #expect(abs(paint.blueComponent - expected.blueComponent) < 0.045)
            }
            let bare = try pic.rect("/children/0")
            for i in [1, 7] { #expect(try pic.rect("/children/\(i)").width == bare.width + 17) }
        }
    }

    @Test func timelinePaintsTheConnectorOnlyBetweenRows() async throws {
        let pic = try await LayoutPicture(#"""
        root = Col([List([ListItem("Start","detail","A"), ListItem("End",null,"B")], "bullet", true), Box([], null, null, null, null, null, null, null, 40)], 0)
        """#, height: 320)
        let end = try pic.rect("/children/0/items/1")
        // A transparent Box below the list keeps the point under the last row
        // inside the drawn picture on any screen.
        let spacer = try pic.rect("/children/1")
        let rows = "start \(try pic.rect("/children/0/items/0")), end \(end), spacer \(spacer)"
        try #require(end.maxY + 3 < spacer.maxY && spacer.maxY <= 320, "Rows outside the picture: \(rows)")
        #expect(try pic.pixel(12, Int(end.minY) - 7).alphaComponent > 0.03)
        #expect(try pic.pixel(15, Int(end.minY) - 7).alphaComponent == 0)
        // Report the paint down the connector's column on failure.
        let below = try pic.pixel(12, Int(end.maxY) + 3)
        let column = try stride(from: Int(end.minY) - 10, through: Int(end.maxY) + 12, by: 2).map { y in
            "\(y):\(String(format: "%.2f", try pic.pixel(12, y).alphaComponent))"
        }.joined(separator: " ")
        // A real display can leave one anti-aliasing step (alpha 1/255) at the
        // last row's edge; the connector itself paints at about 0.10.
        #expect(below.alphaComponent < 0.02, "Paint \(below) under the last row at y=\(Int(end.maxY) + 3); rows \(rows); alpha by y at x=12: \(column)")
    }
}

extension TextDrawingTests {
    @Test func badgeKeepsItsOwnWidthInARowAndTruncatesAtTheContainerWidth() async throws {
        let picture = try await LayoutPicture(#"""
        root = Col([Badge("Tag"), Row([Text("a long paragraph to wrap beside the tag"), Badge("Tag")],1), Row([Badge("more words than the entire container can fit")])], 0, "start")
        """#, width: 110, height: 260)
        let natural = try picture.rect("/children/0")
        #expect(try picture.rect("/children/1/children/1").size == natural.size)
        let wide = try picture.rect("/children/2/children/0")
        #expect(wide.width <= 110 && wide.height == 26)
    }
}

extension TextDrawingTests {
    @Test func markerNoneUsesTheListsSharedColumnsForVisibleAndHiddenCustomMarkers() async throws {
        let pic = try await LayoutPicture(#"""
        root = Col([List([ListItem("fruit",null,"🍎")],"none"), List([ListItem("first",null,"A"),ListItem("plain")],"none")],0)
        """#)
        let stacked = try pic.rect("/children/0/items/0")
        #expect(stacked.minX == 0 && stacked.minY > 0)
        #expect(try pic.rect("/children/1/items/0").minX == 36)
        let hidden = try pic.rect("/children/1/items/1")
        #expect(hidden.minX == 0 && hidden.width == 24)
    }
}
