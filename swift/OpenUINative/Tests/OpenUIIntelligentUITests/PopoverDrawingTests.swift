import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

extension CollectionDrawingTests {
@Suite @MainActor struct PopoverDrawingTests {
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
        defer { mounted.window.close() }
        #expect(Set(mounted.buttonLabels) == ["Show details", "Term"])
    }

    private final class Probe {
        weak var view: NSView?
    }

    private struct Marker: NSViewRepresentable {
        let probe: Probe
        func makeNSView(context: Context) -> NSTextField {
            let label = NSTextField(labelWithString: "Definition")
            probe.view = label
            return label
        }
        func updateNSView(_ view: NSTextField, context: Context) {}
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
            defer { mounted.window.close() }
            let trigger = try mounted.rect(path)
            #expect((trigger.width == 240) == stretch)
            try await mounted.click(CGPoint(x: 200, y: trigger.midY))
            if !stretch {
                #expect(probe.view?.window == nil)
                try await mounted.click(CGPoint(x: trigger.midX, y: trigger.midY))
            }
            for _ in 0..<10 where probe.view?.window == nil { await mounted.settle() }
            let panel = try #require(probe.view?.window)
            #expect(panel !== mounted.window)
            panel.close()
        }
    }

    @Test(.requiresDisplay)
    func popoverOpensAtItsTriggerAndDismissesOnKeysOutsideClickAndScroll() async throws {
        let probe = Probe()
        let mounted = try await CollectionWindow("""
        root = Col([Popover("Term", [Caption("Definition")])], 0, "start")
        """, scrollable: true, component: { element, _ in element.typeName == "Caption" ? AnyView(Marker(probe: probe)) : nil })
        defer { mounted.window.close() }
        let trigger = try mounted.rect("/children/0")
        let point = CGPoint(x: trigger.midX, y: trigger.midY)
        #expect(probe.view?.window?.isVisible != true)
        enum Dismissal { case escape, tab, trigger, outside, scroll }
        for dismissal in [Dismissal.escape, .tab, .trigger, .outside, .scroll] {
            try await mounted.click(point)
            for _ in 0..<10 where probe.view?.window?.isVisible != true { await mounted.settle() }
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
                NSApp.sendEvent(key)
            case .trigger:
                try await mounted.click(point, throughApplication: true)
            case .outside:
                let outside = CGPoint(x: mounted.host.bounds.maxX - 5, y: mounted.host.bounds.maxY - 5)
                try await mounted.click(outside, throughApplication: true)
            case .scroll:
                try #require(mounted.scrollView?.contentView).scroll(to: CGPoint(x: 0, y: 24))
            }
            for _ in 0..<10 where panel.isVisible { await mounted.settle() }
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
