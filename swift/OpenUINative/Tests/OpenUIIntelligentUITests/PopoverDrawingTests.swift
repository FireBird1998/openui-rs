import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

extension CollectionDrawingTests {
@Suite(NativeTestWatchdog()) @MainActor struct PopoverDrawingTests {
    @Test func popoverTriggerRewritesNestedControlsAndKeepsPanelContentClosed() async throws {
        var drawn: [String] = []
        var rowText: [String] = []
        var icon: [String?] = []
        _ = try await LayoutPicture("""
        root = Col([Popover([Row([Button("First"), Link("Source", "https://example.org"), Pressable([IconButton("info")]), Input("value", "Name"), Form([])])], [Caption("Only when open")])])
        """, component: { element, _ in
            drawn.append(element.typeName)
            if element.typeName == "Row" {
                rowText = element.layoutChildren.compactMap { child in
                    if case let .text(text) = child.content { return text }
                    return nil
                }
            }
            if element.typeName == "Icon" {
                icon = ["name", "size"].map { element.prop($0)?.ruleScalar?.string }
            }
            return nil
        })
        #expect(rowText == ["FirstSource", "Name"])
        #expect(icon == ["info", "sm"])
        #expect(Set(drawn) == ["Col", "Popover", "Row", "Icon"])
    }

    @Test func unwrappingAPressablePreservesItsGridCellsEvaluatedPathAndTypography() async throws {
        struct BodyText: View {
            @Environment(\.cardTypography) var typography
            var body: some View { Text("Body").lineHeight(.exact(points: typography?.lineHeight ?? 32)) }
        }
        let picture = try await LayoutPicture("""
        root = Col([Popover([Grid([Pressable([Col([Text("Body")])])], 1)], "Details")])
        """, component: { element, _ in element.typeName == "Text" ? AnyView(BodyText()) : nil })
        let text = try picture.rect("/children/0/trigger/0/children/0/children/0/children/0")
        #expect(text.height == 22)
    }

    @Test func popoverNamesAnIconOnlyTriggerAndReadsFormattedTextAloud() async throws {
        let mounted = try await CollectionWindow("""
        root = Col([Popover([Icon("info", "sm")], "Definition"), Popover("**Term**", "Meaning")])
        """)
        defer { mounted.window.closeForTest() }
        #expect(Set(mounted.buttonLabels) == ["Show details", "Term"])
    }

    private final class Probe {
        weak var view: NSView?
        var presentations: Set<ObjectIdentifier> = []
        var theme: IntelligentTheme?
        var foreground: ThemeColor?
        var paragraphInk: ThemeColor?
    }

    private final class PanelLabel: NSTextField {
        weak var probe: Probe?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let window { probe?.presentations.insert(ObjectIdentifier(window)) }
        }
    }

    private struct Marker: NSViewRepresentable {
        let probe: Probe
        var text = "Definition"
        func makeNSView(context: Context) -> NSTextField {
            let label = PanelLabel(labelWithString: text)
            label.probe = probe
            probe.view = label
            return label
        }
        func updateNSView(_ view: NSTextField, context: Context) {
            probe.theme = context.environment.intelligentTheme
            probe.foreground = context.environment.intelligentForeground
            probe.paragraphInk = context.environment.paragraphType?.color
        }
    }

    @Test func clickingAnIconOnlyTriggerOpensItsOwnPanel() async throws {
        let probe = Probe()
        let other = Probe()
        let source = #"""
        root = Col([Popover([Icon("bell", "sm")], [Caption("Other details")]), Popover([Icon("info", "sm")], [Caption("Icon details")])], 1, "start")
        """#
        let picture = try await LayoutPicture(source)
        for index in 0..<2 {
            let frame = try picture.rect("/children/\(index)/trigger/0")
            #expect(try picture.alpha(in: frame).contains { $0 > 32 }, "Trigger \(index)")
        }
        let mounted = try await CollectionWindow(source, component: { element, _ in
            switch element.word("text") {
            case "Icon details": AnyView(Marker(probe: probe))
            case "Other details": AnyView(Marker(probe: other))
            default: nil
            }
        })
        defer { mounted.window.closeForTest() }
        defer { other.view?.window?.closeForTest() }
        let glyph = try mounted.rect("/children/1/trigger/0")
        #expect(glyph.size == CGSize(width: 14, height: 14))
        #expect(probe.view?.window == nil)
        try await mounted.click(CGPoint(x: glyph.midX, y: glyph.midY))
        for _ in 0..<10 where probe.view?.window == nil { await mounted.settle() }
        let panel = try #require(probe.view?.window)
        defer { panel.closeForTest() }
        #expect(panel !== mounted.window)
        #expect(probe.presentations == [ObjectIdentifier(panel)])
        #expect(other.presentations.isEmpty)
    }

    @Test(arguments: [false, true])
    func iconOnlyPopoverTriggerUsesItsForeground(_ hovered: Bool) async throws {
        let picture = try await LayoutPicture(#"""
        root = Col([Popover([Icon("minus", "lg"), Icon("minus", "lg", "currentColor"), Icon("minus", "lg", "#00ff00")], "Details")], 0, "start")
        """#, component: { element, path in
            guard element.typeName == "Popover" else { return nil }
            // Seed the live hover state so this renders the same Button without a pointer or a display.
            return AnyView(LayoutPopover(element: element, path: path, initiallyHovered: hovered))
        })
        let theme = IntelligentTheme.light
        let inherited = try LayoutPicture.swatch(hovered ? theme.muted.color : theme.ink.color)
        let explicit = try LayoutPicture.swatch(Color(red: 0, green: 1, blue: 0))
        let explicitFrame = try picture.rect("/children/0/trigger/2")
        let strokeCoverage = try picture.pixel(Int(explicitFrame.midX), Int(explicitFrame.midY)).alphaComponent
        #expect(strokeCoverage > 0.75)
        for index in 0..<3 {
            let frame = try picture.rect("/children/0/trigger/\(index)")
            let paint = try picture.pixel(Int(frame.midX), Int(frame.midY))
            let swatch = index == 2 ? explicit : inherited
            #expect(abs(paint.redComponent - swatch.redComponent) < 0.05)
            #expect(abs(paint.greenComponent - swatch.greenComponent) < 0.05)
            #expect(abs(paint.blueComponent - swatch.blueComponent) < 0.05)
            #expect(abs(paint.alphaComponent - swatch.alphaComponent * strokeCoverage) < 0.06)
        }
    }

    @Test(arguments: [IntelligentUIMode.light, .dark], ["", #", "currentColor""#])
    func popoverInABlackBoxUsesTheRootPalette(_ mode: IntelligentUIMode, _ colorArgument: String) async throws {
        try await expectOpenedPanelColors(#"""
        root = Col([Box([Popover([Icon("minus", "lg"\#(colorArgument))], [Icon("minus", "lg"\#(colorArgument))])], null, 0, 1, "#000")], 0, "start")
        """#, path: "/children/0/children/0", mode: mode, triggerInk: .hex(0xffffff))
    }

    @Test(arguments: [IntelligentUIMode.light, .dark])
    func popoverInInlineColoredTextUsesTheRootPalette(_ mode: IntelligentUIMode) async throws {
        try await expectOpenedPanelColors(#"""
        root = Col([Text(["Read ", Popover([Icon("minus", "lg")], [Icon("minus", "lg"), Text("Details"), Badge("Tag")])], "md", "#008000")], 0, "start")
        """#, path: "/children/0/text/1", mode: mode, triggerInk: .hex(0x008000))
    }

    private func expectOpenedPanelColors(_ source: String, path: String, mode: IntelligentUIMode,
                                         triggerInk: ThemeColor) async throws {
        let trigger = Probe(), content = Probe()
        let mounted = try await CollectionWindow(source, mode: mode, component: { element, slot in
            guard element.typeName == "Icon" else { return nil }
            let probe = slot == "\(path)/trigger/0" ? trigger : content
            return AnyView(IconDrawing(element: element).background {
                Marker(probe: probe, text: "").frame(width: 0, height: 0)
            })
        })
        defer { mounted.window.closeForTest() }
        #expect(trigger.foreground == triggerInk)
        #expect(content.presentations.isEmpty)
        let glyph = try mounted.rect("\(path)/trigger/0")
        try await mounted.click(CGPoint(x: glyph.midX, y: glyph.midY))
        for _ in 0..<10 where content.view?.window == nil { await mounted.settle() }
        let panel = try #require(content.view?.window)
        defer { panel.closeForTest() }
        #expect(panel !== mounted.window)
        let theme = IntelligentTheme.forScheme(mode.colorScheme)
        #expect(content.theme?.ink == theme.ink)
        #expect(content.theme?.canvas == theme.canvas)
        #expect(content.foreground == theme.ink)
        #expect(content.paragraphInk == nil)
    }

    @Test(arguments: [IntelligentUIMode.light, .dark])
    func popoverPanelPaintsItsChildrenWithRootThemeColors(_ mode: IntelligentUIMode) async throws {
        func picture(withTriggerColors: Bool) async throws -> LayoutPicture {
            try await LayoutPicture(#"""
            root = Col([Popover("Term", children)], 0, "start")
            children = [Icon("minus", "lg"), Icon("minus", "lg", "currentColor"), Text("Details", "md"), Badge("Tag", "neutral", "minus"), Caption("Note"), surface, subtitle]
            surface = Box([], null, 0, 0, "surface", null, null, 32, 10)
            subtitle = Col([Title("Heading", "sm"), Text("Subtitle")], 0, "start")
            """#, height: 260, mode: mode, component: { element, path in
                guard element.typeName == "Popover", let content = element.prop("content") else { return nil }
                let panel = PopoverPanel(content: content, path: path, label: "Term", limit: 280, close: {})
                guard withTriggerColors else { return AnyView(panel) }
                let paragraph = WordStyle(metrics: TextMetrics(16), color: .hex(0x008000))
                return AnyView(panel.environment(\.intelligentInk, .hex(0xffffff))
                    .environment(\.paragraphType, paragraph)
                    .environment(\.inOpeningRow, true))
            })
        }

        // Use the same presentation view at the theme root to compare pixels in each child.
        let root = try await picture(withTriggerColors: false)
        let scoped = try await picture(withTriggerColors: true)
        let children = [("0", "Icon"), ("1", "currentColor Icon"), ("2", "Text"), ("3", "Badge"),
                        ("4", "Caption"), ("5", "Box surface"), ("6/children/1", "Text after Title")]
        for (childPath, child) in children {
            let path = "/children/0/content/\(childPath)"
            let frame = try root.rect(path).integral
            let scopedFrame = try scoped.rect(path).integral
            try #require(!frame.isEmpty && scopedFrame == frame)
            let matches = try (Int(frame.minY)..<Int(frame.maxY)).allSatisfy { y in
                try (Int(frame.minX)..<Int(frame.maxX)).allSatisfy { x in
                    try scoped.pixel(x, y) == root.pixel(x, y)
                }
            }
            #expect(matches, "\(child) in \(mode)")
        }
    }

    @Test func popoverHitAreaFollowsColumnAlignmentAndGridStretch() async throws {
        for (parent, path, stretch) in [
            ("Col([p], 0)", "/children/0", true),
            ("Col([Grid([p], 1)], 0)", "/children/0/children/0", true),
            (#"Col([p], 0, "start")"#, "/children/0", false),
        ] {
            let probe = Probe()
            let mounted = try await CollectionWindow("""
            root = \(parent)
            p = Popover("Term", [Caption("Definition")])
            """, width: 240, component: { element, _ in
                element.typeName == "Caption" ? AnyView(Marker(probe: probe)) : nil
            })
            defer { mounted.window.closeForTest() }
            let trigger = try mounted.rect(path)
            #expect((trigger.width == 240) == stretch)
            try await mounted.click(CGPoint(x: 200, y: trigger.midY))
            if !stretch {
                #expect(probe.view?.window == nil)
                try await mounted.click(CGPoint(x: trigger.midX, y: trigger.midY))
            }
            try await mounted.waitForLayout("the popover panel to mount", for: .seconds(2)) { probe.view?.window != nil }
            let panel = try #require(probe.view?.window)
            #expect(panel !== mounted.window)
            panel.closeForTest()
        }
    }

    @Test(.requiresDisplay)
    func popoverOpensAtItsTriggerAndDismissesOnKeysOutsideClickAndScroll() async throws {
        let probe = Probe()
        let mounted = try await CollectionWindow("""
        root = Col([Popover("Term", [Caption("Definition")])], 0, "start")
        """, scrollable: true, component: { element, _ in element.typeName == "Caption" ? AnyView(Marker(probe: probe)) : nil })
        defer { mounted.window.closeForTest() }
        let trigger = try mounted.rect("/children/0")
        let point = CGPoint(x: trigger.midX, y: trigger.midY)
        #expect(probe.view?.window?.isVisible != true)
        enum Dismissal { case escape, tab, trigger, outside, scroll }
        for dismissal in [Dismissal.escape, .tab, .trigger, .outside, .scroll] {
            try await mounted.click(point)
            try await mounted.waitForLayout("the popover panel to become visible", for: .seconds(2)) { probe.view?.window?.isVisible == true }
            let panel = try #require(probe.view?.window, "Opening the panel for the \(dismissal) dismissal")
            try #require(panel.isVisible, "The panel must be visible before the \(dismissal) dismissal")
            #expect(panel !== mounted.window)
            let anchor = mounted.window.convertPoint(toScreen: mounted.host.convert(point, to: nil))
            #expect(panel.frame.insetBy(dx: -16, dy: -32).contains(anchor))
            switch dismissal {
            case .escape, .tab:
                let character = dismissal == .escape ? "\u{1b}" : "\t"
                let key = try #require(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: panel.windowNumber,
                    context: nil, characters: character, charactersIgnoringModifiers: character,
                    isARepeat: false, keyCode: dismissal == .escape ? 53 : 48))
                NativeTestWatchdog.waitingFor("AppKit to dispatch the popover dismissal key")
                NSApp.sendEvent(key)
            case .trigger:
                try await mounted.click(point, throughApplication: true)
            case .outside:
                let outside = CGPoint(x: mounted.host.bounds.maxX - 5, y: mounted.host.bounds.maxY - 5)
                try await mounted.click(outside, throughApplication: true)
            case .scroll:
                NativeTestWatchdog.waitingFor("AppKit to scroll the popover's source view")
                try #require(mounted.scrollView?.contentView).scroll(to: CGPoint(x: 0, y: 24))
            }
            try await mounted.waitForLayout("the popover panel to dismiss", for: .seconds(2)) {
                !panel.isVisible && probe.view?.window?.isVisible != true
            }
            if panel.isVisible || probe.view?.window?.isVisible == true {
                let responder = panel.firstResponder.map { String(describing: type(of: $0)) } ?? "nil"
                let visiblePanel = probe.view?.window?.windowNumber
                let origin = mounted.scrollView?.contentView.bounds.origin
                throw CollectionInputFailure(description: """
                    Popover visible after \(dismissal), original panel=\(panel.windowNumber), \
                    current panel=\(String(describing: visiblePanel)), key window=\(String(describing: NSApp.keyWindow?.windowNumber)), \
                    first responder=\(responder), source scroll origin=\(String(describing: origin))
                    """)
            }
        }
    }

    // Measure the same panel the native popover presents, without a screen.
    @Test func popoverPanelSizesToShortTextWrapsLongTextAndSpacesItsElements() async throws {
        func size(_ string: String, limit: CGFloat) throws -> CGSize {
            let panel = PopoverPanel(content: .scalar(.string(string)), path: "/", label: "Term", limit: limit, close: {})
            NativeTestWatchdog.waitingFor("SwiftUI to lay out and render the popover panel")
            return try #require(ImageRenderer(content: panel).nsImage).size
        }
        let short = try size("**Term**", limit: 280)
        let plain = try size("Ship today please", limit: 280)
        let mixed = try size("Ship **today** please", limit: 280)
        #expect(mixed.height == plain.height + 2 * (21 + 8))
        let sentence = try size("Read the guide today", limit: 280)
        #expect(try size("Read [the guide](/guide) today", limit: 280) == sentence)
        #expect(try size("Read [the guide](https://example.org/guide) today", limit: 280).height
                == sentence.height + 2 * (21 + 8))
        #expect(try size("Read [the **guide**](/guide) today", limit: 280)
                == size("Read the **guide** today", limit: 280))
        for separator in ["  ", "\n", "\n\n", "\t\r\n "] {
            #expect(try size("Read the\(separator)guide today", limit: 280) == sentence)
        }
        #expect(try size("Read the\u{a0}\u{a0}guide today", limit: 280).width > sentence.width)
        let wrapped = try size(String(repeating: "definition ", count: 12), limit: 160)
        #expect(short.width < 100)
        #expect(short.height >= 45)
        #expect(wrapped.width == 160)
        #expect(wrapped.height > short.height)

        let picture = try await LayoutPicture("""
        root = Col([Popover("Term", [a, a])], 0, "start")
        a = Box([], null, null, null, "blue", null, null, 32, 10)
        """, component: { element, path in
            guard element.typeName == "Popover", let content = element.prop("content") else { return nil }
            return AnyView(PopoverPanel(content: content, path: path, label: "Term", limit: 280, close: {}))
        })
        let panel = try picture.rect("/children/0")
        let first = try picture.rect("/children/0/content/0"), second = try picture.rect("/children/0/content/1")
        #expect(first.minX - panel.minX == 14)
        #expect(first.minY - panel.minY == 12)
        #expect(second.minY - first.maxY == 8)
        #expect(panel.maxY - second.maxY == 12)
    }
}
}
