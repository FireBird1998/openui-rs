import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

// Size the mounted text independently of its container while retaining native selection and links.
struct IntrinsicText: TextRenderer {
    var minimumHeight: CGFloat = 0
    func sizeThatFits(proposal: ProposedViewSize, text: TextProxy) -> CGSize {
        let size = text.sizeThatFits(.unspecified)
        return CGSize(width: size.width, height: max(size.height, minimumHeight))
    }
    func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
        for line in layout { ctx.draw(line) }
    }
}

// Send real mouse events in window coordinates. Recording openURL observes
// the same action that the system normally delivers to the default browser.
@MainActor final class TextWindow {
    let live: LiveProgram
    let window: NSWindow
    let host: NSHostingView<AnyView>
    var frames: [String: CGRect] = [:]
    var opened: [URL] = []
    var hits = 0

    init(_ source: String, overflow: String? = nil, overflowAbove: Bool = false, width: CGFloat = 300, height: CGFloat = 350,
         renderer: IntrinsicText? = nil,
         mode: IntelligentUIMode = .light, lineHeight: CGFloat? = nil, textCase: Text.Case? = nil,
         markdownWidth: CGFloat? = nil, scrollable: Bool = false, contentOverride: AnyView? = nil,
         colorSpace: WindowColorSpace? = nil) async throws {
        live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: source, streaming: false)
        #expect(live.errors.map(\.message).isEmpty)
        _ = NSApplication.shared
        host = NSHostingView(rootView: AnyView(EmptyView()))
        window = NSWindow(contentRect: CGRect(x: -20000, y: -20000, width: width, height: height),
                          styleMask: [.borderless], backing: .buffered, defer: false)
        if let colorSpace { window.colorSpace = colorSpace.colorSpace }
        let drawing = ElementDrawing(component: { [weak self] element, _ in
            guard element.typeName == overflow else { return nil }
            // Report twenty points to the parent but draw a 100-point button.
            return AnyView(Button { self?.hits += 1 } label: {
                Color.red.frame(width: 100, height: overflowAbove ? 80 : 20)
            }.buttonStyle(.plain).fixedSize().frame(width: 20, height: 20, alignment: overflowAbove ? .bottomLeading : .leading))
        }, chrome: { [weak self] view, element, path in
            let measured = view.onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { self?.frames[path] = $0 }
            if element.typeName == "Markdown", let markdownWidth {
                return AnyView(measured.frame(width: markdownWidth, alignment: .leading))
            }
            return AnyView(measured)
        })
        let content = ElementView(live: live).environment(\.elementDrawing, drawing)
            .environment(\.openURL, OpenURLAction { [weak self] url in self?.opened.append(url); return .handled })
            .textCase(textCase)
            .lineHeight(lineHeight.map { .exact(points: $0) }).intelligentUIMode(mode)
        let drawn = contentOverride ?? AnyView(content)
        let page = (scrollable ? AnyView(ScrollView { drawn }) : drawn)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        host.rootView = renderer.map { AnyView(page.textRenderer($0)) } ?? AnyView(page)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        await settle()
    }

    func attributed(_ label: String) throws -> NSAttributedString {
        let view = try textView(label)
        return try #require(view.accessibilityAttributedString(for: NSRange(location: 0, length: label.utf16.count)))
    }

    func textView(_ label: String) throws -> NSView {
        var pending = [host as NSView]
        while let view = pending.popLast() {
            if view.accessibilityValue() as? String == label,
               view.accessibilityAttributedString(for: NSRange(location: 0, length: label.utf16.count)) != nil {
                return view
            }
            pending.append(contentsOf: view.subviews)
        }
        throw MissingText(label: label)
    }

    private struct MissingText: Error { let label: String }

    func settle() async {
        await layoutOnMainQueue(host)
    }

    func click(_ path: String, x: CGFloat = 4, y: CGFloat? = nil, count: Int = 1) async throws {
        let frame = try #require(frames[path], "\(path)")
        try await click(CGPoint(x: frame.minX + x, y: y.map { frame.minY + $0 } ?? frame.midY), count: count)
    }

    func click(_ local: CGPoint, count: Int = 1) async throws {
        let point = host.convert(local, to: nil)
        func mouse(_ type: NSEvent.EventType) throws -> NSEvent {
            try #require(NSEvent.mouseEvent(with: type, location: point, modifierFlags: [],
                                           timestamp: ProcessInfo.processInfo.systemUptime,
                                           windowNumber: window.windowNumber, context: nil,
                                           eventNumber: 0, clickCount: count, pressure: 1))
        }
        let down = try mouse(.leftMouseDown), up = try mouse(.leftMouseUp)
        window.sendEvent(down)
        window.sendEvent(up)
        await settle()
    }
}

extension TextDrawingTests {
    @Test func clickingStandaloneAndInlineLinksDeliversOnlyTheDrawnWebURL() async throws {
        let window = try await TextWindow(#"""
        root = Col([Link("", "https://example.org/one"), Link("local", "/no"), Text("[two](http://example.org/two)"), Caption("[local](javascript:no)")],1,"start")
        """#)
        defer { window.window.close() }
        for i in 0..<4 { try await window.click("/children/\(i)") }
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/one", "http://example.org/two"])
    }

    @Test func overflowingChildrenOfTextListAndListItemCannotBeClickedOutside() async throws {
        for (view, child, path) in [
            (#"Text(["", Badge("tag")])"#, "Badge", "/text/1"),
            (#"List([ListItem("item")],"none")"#, "ListItem", "/items/0"),
            (#"ListItem("title",null,null,[Badge("tag")])"#, "Badge", "/children/0")
        ] {
            let window = try await TextWindow("root = Col([\(view)], 0, \"start\")", overflow: child)
            defer { window.window.close() }
            let container = try #require(window.frames["/children/0"])
            let key = "/children/0" + path
            let inside = try #require(window.frames[key])
            try await window.click(key)
            #expect(window.hits == 1, "\(view)")
            try await window.click(key, x: container.maxX - inside.minX + 6)
            #expect(window.hits == 1, "\(view)")
        }
    }
}

extension TextDrawingTests {
    @Test func doubleClickSelectsTheDrawnWord() async throws {
        let window = try await TextWindow(#"root = Col([Text("selectable words")], 0, "start")"#)
        defer { window.window.close() }
        try await window.click("/children/0", x: 18, count: 2)
        let text = try #require(window.window.firstResponder as? NSAccessibilityProtocol)
        #expect(text.accessibilitySelectedText() == "selectable")

    }
}

extension TextDrawingTests {
    @Test func drawnTypeAndInkFollowPropsAndTheActiveTheme() async throws {
        let source = #"""
        root = Col([Title("Head","lg","purple"), Text("One","xs","success","normal"), Text("Two","sm","danger","medium",true,true,true), Text("Three","lg","ink","semibold"), Caption("Note"), Label("**Label**"), Badge("**Tag**","pink"), Link("**Local**","/here")],0,"start")
        """#
        for mode: IntelligentUIMode in [.light, .dark] {
            let theme = mode == .dark ? IntelligentTheme.dark : .light
            let window = try await TextWindow(source, mode: mode)
            defer { window.window.close() }
            let cases: [(String, CGFloat, Int, ThemeColor)] = [
                ("Head", 19, 640, theme.purple), ("One", 12, 440, theme.success), ("Two", 14, 560, theme.danger),
                ("Three", 18, 640, theme.ink), ("Note", 12, 400, theme.muted),
                ("**Label**", 16, mode == .dark ? 420 : 440, theme.ink), ("**Tag**", 13, 560, theme.badgePink),
                ("**Local**", 16, 400, theme.ink),
            ]
            for (i, (label, size, weight, ink)) in cases.enumerated() {
                let value = try window.attributed(label)
                #expect(value.string == label)
                let font = try #require(value.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
                let axis = CTFontCopyVariation(font) as? [UInt32: NSNumber]
                #expect(font.pointSize == size)
                #expect(axis?[0x77676874]?.intValue ?? 400 == weight, "\(label), \(String(describing: axis))")
                let traits = CTFontGetSymbolicTraits(font)
                #expect(traits.contains(.traitItalic) == (i == 2))
                #expect((value.attribute(.strikethroughStyle, at: 0, effectiveRange: nil) != nil) == (i == 2))
                let paint = try #require((value.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor)?.usingColorSpace(.extendedSRGB))
                #expect(abs(paint.redComponent - ink.red) < 0.005 && abs(paint.greenComponent - ink.green) < 0.005
                        && abs(paint.blueComponent - ink.blue) < 0.005 && abs(paint.alphaComponent - ink.alpha) < 0.005)
            }
        }
    }
}

extension TextDrawingTests {
    @Test func aClampedInlineControlAndTheQuotePaddingDoNotOpenLinks() async throws {
        let window = try await TextWindow(#"""
        root = Col([Text([Link("one","https://example.org/1"),"\n",Link("two","https://example.org/2")],null,null,null,null,null,null,1), Quote("[outside?](https://example.org/3)")],4,"start")
        """#)
        defer { window.window.close() }
        try await window.click("/children/0/text/0")
        try await window.click("/children/0/text/2")
        try await window.click("/children/1", x: 4)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/1"])
        try await window.click("/children/1", x: 23)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/1", "https://example.org/3"])
    }
}

extension TextDrawingTests {
    @Test func inlineLinksInheritParagraphTypeAndInkWhileBadgesKeepTheirTone() async throws {
        let window = try await TextWindow(#"""
        root = Col([Text(["Read ",Link("this","https://example.org"),Badge("tag","warning")],"lg","danger","medium",true)],0)
        """#)
        defer { window.window.close() }
        let link = try window.attributed("this ↗")
        let font = try #require(link.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
        #expect(font.pointSize == 18)
        #expect(CTFontGetSymbolicTraits(font).contains(.traitItalic))
        let ink = try #require((link.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor)?.usingColorSpace(.extendedSRGB))
        #expect(abs(ink.redComponent - IntelligentTheme.light.danger.red) < 0.005)
        let badge = try window.attributed("tag")
        let badgeFont = try #require(badge.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
        #expect(badgeFont.pointSize == 13)
    }
}

extension TextDrawingTests {
    @Test func listBodiesAndStandaloneLinksInheritLineHeightAndRichTitlesUseMediumWeight() async throws {
        let window = try await TextWindow(#"""
        root = Col([Link("", "https://EXAMPLE.org/path"), List(["plain", ListItem("rich", "detail")], "none"), ListItem("standalone"), Quote("quoted", "source")], 0)
        """#, lineHeight: 37)
        defer { window.window.close() }
        #expect(window.frames["/children/0"]?.height == 37)
        #expect(window.frames["/children/2"]?.height == 37)
        #expect(abs((window.frames["/children/3"]?.height ?? 0) - (28 + 25.6 + 4 + 37)) <= 1)
        _ = try window.attributed("example.org ↗")
        for (label, size, weight) in [("plain", 16.0, 400), ("rich", 16, 560), ("detail", 16 * 0.93, 400),
                                      ("standalone", 16, 400), ("quoted", 16, 400), ("source", 13, 400)] {
            let value = try window.attributed(label)
            let font = try #require(value.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
            let axes = CTFontCopyVariation(font) as? [UInt32: NSNumber]
            #expect(abs(font.pointSize - size) < 0.01)
            #expect(axes?[0x77676874]?.intValue ?? 400 == weight)
        }
    }
}

extension TextDrawingTests {
    @Test func aSpaceInsideAnInlineLinkOpensThatLinkInAMixedParagraph() async throws {
        let window = try await TextWindow(#"""
        root = Col([Text(["[MMMM MMMM](https://example.org/mixed) ", Badge("tag")],"md")],0)
        """#)
        defer { window.window.close() }
        let font = NSFont.systemFont(ofSize: 16)
        let word = ("MMMM" as NSString).size(withAttributes: [.font: font]).width
        try await window.click("/children/0", x: ceil(word) + 2)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/mixed"])
    }
}

extension TextDrawingTests {
    @Test func aLinkPassedAsAListEntryIsInertObjectText() async throws {
        let window = try await TextWindow(#"root = Col([List([Link("Surprise","https://example.org/list")],"none")])"#)
        defer { window.window.close() }
        try await window.click("/children/0")
        #expect(window.opened.isEmpty)
        _ = try window.attributed("[object Object]")
    }

    @Test func webLinksPreserveEscapesAndRejectScopedIPv6AtTheBrowserAction() async throws {
        let window = try await TextWindow(#"""
        root = Col([Link("go","https://example.org/%2F/分?q=%2b分"),Text("[go](https://example.org/%2F/分?q=%2b分)"),Link("bad","http://[::1%25en0]/"),Text("[bad](http://[::1%25en0]/)"),Link("bad","http://%5B%3A%3A1%5D/")],0,"start")
        """#)
        defer { window.window.close() }
        for i in 0..<5 { try await window.click("/children/\(i)") }
        let destination = "https://example.org/%2F/%E5%88%86?q=%2b%E5%88%86"
        #expect(window.opened.map(\.absoluteString) == [destination, destination])
    }

    @Test func inlineLinksKeepUnicodePathsAfterAPort() async throws {
        let window = try await TextWindow("""
        root = Col([Text("[path](https://example.org:443/\u{0301})"), Caption("[host](http://0x)")], 0, "start")
        """)
        defer { window.window.close() }
        try await window.click("/children/0")
        try await window.click("/children/1")
        #expect(window.opened.map(\.absoluteString) == ["https://example.org:443/%CC%81", "http://0.0.0.0"])
    }

    @Test func wrappingInsideAWordConsumesTheMixedParagraphLineBudget() async throws {
        let window = try await TextWindow(#"""
        root = Col([Text(["ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZ","\n",Link("hidden","https://example.org/hidden")],"md",null,null,null,null,null,2)],0)
        """#, width: 100)
        defer { window.window.close() }
        let rect = try #require(window.frames["/children/0"])
        #expect(abs(rect.height - 2 * 25.6) < 1.5)
        try await window.click("/children/0", y: 64)
        #expect(window.opened.isEmpty)
    }

    @Test func changingTheTextAroundAMountedBadgeReflowsTheParagraph() async throws {
        let window = try await TextWindow(#"root = Col([Text(["a",Badge("B")],"md")],0)"#, width: 100)
        defer { window.window.close() }
        let first = try #require(window.frames["/children/0/text/1"])
        try await window.live.show(response: #"root = Col([Text(["first line\nsecond line\n",Badge("B")],"md")],0)"#, streaming: false)
        await window.settle()
        let moved = try #require(window.frames["/children/0/text/1"])
        #expect(moved.minY >= first.maxY + 24)
        #expect(moved.minX < first.minX)
    }

    @Test func hebrewWordsReadRightToLeftBesideAnInlineBadge() async throws {
        let window = try await TextWindow(#"root = Col([Text(["שלום עולם ",Badge("B")],"md")],0)"#)
        defer { window.window.close() }
        let badge = try #require(window.frames["/children/0/text/1"])
        #expect(badge.minX > 65 && badge.minY < 5)
        for (x, word): (CGFloat, String) in [(15, "עולם"), (55, "שלום")] {
            try await window.click("/children/0", x: x, count: 2)
            let text = try #require(window.window.firstResponder as? NSAccessibilityProtocol)
            #expect(text.accessibilitySelectedText() == word)
        }
    }

    @Test func listItemFlowsInlineChildrenBeforeABlockAndPreservesTheirClicks() async throws {
        let source = #"""
        root = Col([ListItem("Facts",null,null,[Badge("New"),Link("Guide","https://example.org/guide"),Text("Details")])],0)
        """#
        for width: CGFloat in [300, 85] {
            let window = try await TextWindow(source, width: width)
            defer { window.window.close() }
            let base = "/children/0/children/"
            let badge = try #require(window.frames[base + "0"])
            let link = try #require(window.frames[base + "1"])
            let block = try #require(window.frames[base + "2"])
            if width == 300 {
                #expect(badge.maxX <= link.minX && abs(badge.midY - link.midY) < 4)
            } else {
                #expect(link.minY >= badge.maxY)
            }
            #expect(block.minY >= link.maxY && block.minX == badge.minX)
            try await window.click(base + "0")
            try await window.click(base + "1")
            try await window.click(base + "2")
            #expect(window.opened.map(\.absoluteString) == ["https://example.org/guide"])
        }
    }

    @Test func overflowingListChildrenCannotInterceptAClickAboveTheirLine() async throws {
        let window = try await TextWindow(#"root = Col([ListItem("Title",null,null,[Badge("B")])],0)"#,
                                          overflow: "Badge", overflowAbove: true)
        defer { window.window.close() }
        let path = "/children/0/children/0"
        try await window.click(path)
        #expect(window.hits == 1)
        try await window.click(path, y: -4)
        #expect(window.hits == 1)
    }

    @Test func richTextKeepsNestedArrayAndObjectValues() async throws {
        let window = try await TextWindow(#"""
        root = Col([Text(["First ",[["North","South"]]," Last"]),Text(["From ",{town:"Bath"}," today"])],0)
        """#)
        defer { window.window.close() }
        _ = try window.attributed("First North,South Last")
        _ = try window.attributed("From [object Object] today")
    }

    @Test func emptyProseCollapsesInAZeroGapColumn() async throws {
        let window = try await TextWindow(#"""
        root = Col([Title(""),Text(""),Caption(""),Text("    "),Text("visible","md")],0)
        """#)
        defer { window.window.close() }
        #expect(window.frames["/children/4"]?.minY == 0)
    }

    @Test func nestedPlainListItemInheritsRichTitleWeight() async throws {
        let window = try await TextWindow(#"""
        root = Col([List([ListItem("outer",null,null,[List([ListItem("inner")],"none")])],"none")],0)
        """#)
        defer { window.window.close() }
        for title in ["inner", "outer"] {
            let text = try window.attributed(title)
            let font = try #require(text.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
            let axes = CTFontCopyVariation(font) as? [UInt32: NSNumber]
            #expect(axes?[0x77676874]?.intValue ?? 400 == 560)
        }
    }

    @Test func overflowingLinkedQuoteGlyphOnlyOpensInsideItsInnerEdge() async throws {
        // The inner width is eight points. A single W is wider than that
        // and has no internal line break opportunity.
        let window = try await TextWindow(#"root = Col([Quote("[W](https://example.org/quote)")],0)"#, width: 43, renderer: IntrinsicText())
        defer { window.window.close() }
        try await window.click("/children/0", x: 22)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/quote"])
        try await window.click("/children/0", x: 30)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/quote"])
    }

    @Test func badgesAlignToTheStartOfCenteredAndEndAlignedColumns() async throws {
        for align in ["center", "end"] {
            let window = try await TextWindow("root = Col([Badge(\"Tag\")],0,\"\(align)\")", width: 240)
            defer { window.window.close() }
            #expect(window.frames["/children/0"]?.minX == 0)
        }
    }

    @Test(arguments: WindowColorSpace.allCases)
    func badgeDrawsTheNamedLucideGlyphAndFallsBackToCircleDot(space: WindowColorSpace) async throws {
        let window = try await TextWindow(#"""
        root = Col([Badge("Tag","info","alarm-clock"),Badge("Tag","info","AlarmClock"),Badge("Tag","info","unknown-name"),Badge("Tag","info","circle-dot"),Badge("Tag","info","Clock Alarm"),Badge("Tag","info","alarm-clock-unlisted")],0,"start")
        """#, colorSpace: space)
        defer { window.window.close() }
        let capture = try MountedViewPixels(capturing: window.host, for: "mounted badge icons", in: space)
        func glyph(_ index: Int) throws -> [CGFloat] {
            let rect = try #require(window.frames["/children/\(index)"])
            return try (6..<20).flatMap { y in
                try (8..<22).map { x in
                    let point = CGPoint(x: rect.minX + CGFloat(x), y: rect.minY + CGFloat(y))
                    return try capture.pixel(at: point).redComponent
                }
            }
        }
        let alarm = try glyph(0), clock = try glyph(1), unknown = try glyph(2)
        let missing = try glyph(3), loud = try glyph(4), unlisted = try glyph(5)
        #expect(alarm == clock)
        #expect(unknown == missing)
        #expect(alarm == loud)
        #expect(alarm == unlisted)
        #expect(alarm != unknown)
    }
}

extension TextDrawingTests {
    @Test func overflowingLinkedTitleCannotBeClickedOutsideItsListItem() async throws {
        // The second cell is empty at the click point. The title draws at its
        // intrinsic width, well beyond the 60 points available to the first cell.
        let window = try await TextWindow(#"""
        root = Col([Grid([ListItem("[WWWWWWWWWWWW](https://example.org/item)"), Spacer()],2,null,0)],0)
        """#, width: 120, renderer: IntrinsicText())
        defer { window.window.close() }
        let path = "/children/0/children/0"
        let item = try #require(window.frames[path])
        #expect(item.width == 60)
        try await window.click(path, x: 20)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/item"])
        try await window.click(path, x: item.width + 5)
        #expect(window.opened.count == 1)
    }

    @Test func overflowingLinkedFragmentCannotBeClickedBeyondItsLineRun() async throws {
        // Uppercasing expands each ß to SS in the mounted text while the
        // paragraph assigns the run the advance of the original three glyphs.
        let window = try await TextWindow(#"""
        root = Col([Text(["[ßßß](https://example.org/run)", "\n", Badge("B")],"md")],0)
        """#, textCase: .uppercase)
        defer { window.window.close() }
        let path = "/children/0"
        try await window.click(path, x: 5, y: 12)
        #expect(window.opened.map(\.absoluteString) == ["https://example.org/run"])
        try await window.click(path, x: 40, y: 12)
        #expect(window.opened.count == 1)
    }
}

extension TextDrawingTests {
    @Test func overflowingBadgeLabelCannotBeSelectedOutsideItsCapsule() async throws {
        let window = try await TextWindow(#"root = Col([Badge("Selectable label in a capsule")],0)"#,
                                          renderer: IntrinsicText(minimumHeight: 90))
        defer { window.window.close() }
        let path = "/children/0"
        let frame = try #require(window.frames[path])
        func selected() -> String? {
            (window.window.firstResponder as? NSAccessibilityProtocol)?.accessibilitySelectedText()
        }
        // Both clicks are in the selectable label's rectangular bounds. Only
        // the second point is also inside the capsule around that label.
        let capsule = Capsule().path(in: CGRect(origin: .zero, size: frame.size))
        let outside = CGPoint(x: 11, y: 8)
        let inside = CGPoint(x: 25, y: 12)
        try #require(!capsule.contains(outside) && capsule.contains(inside))
        try await window.click(path, x: outside.x, y: outside.y, count: 2)
        #expect(selected()?.isEmpty ?? true)
        try await window.click(path, x: inside.x, y: inside.y, count: 2)
        #expect(selected() == "Selectable")
    }
}
