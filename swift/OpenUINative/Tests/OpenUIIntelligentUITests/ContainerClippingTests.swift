import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

extension CollectionDrawingTests {
@Suite @MainActor struct ContainerClippingTests {
    // Keep the adjacent cell empty at the click point. A painted sibling can
    // intercept the event and hide a hit that escaped the container.
    private func checkCell(_ container: String) async throws {
        try await checkTrigger("""
        root = Col([Grid([\(container), tall], 2, null, 0)], 0)
        wide = Box([Popover("Term", [Caption("Meaning")])], null, 0, null, "red", null, null, 200, 50)
        tall = Box([], null, null, null, "red", null, null, 5, 100)
        """, width: 240, outside: [CGPoint(x: 160, y: 12)], inside: CGPoint(x: 12, y: 12))
    }

    private func checkTrigger(_ source: String, width: CGFloat, outside: [CGPoint], inside: CGPoint,
                              trigger: (path: String, size: CGSize)? = nil) async throws {
        var presentations = 0
        let mounted = try await CollectionWindow(source, width: width, component: { element, _ in
            guard element.typeName == "Caption" else { return nil }
            return AnyView(Text("Meaning").onAppear { presentations += 1 })
        })
        defer { mounted.window.close() }
        if let trigger { #expect(try mounted.rect(trigger.path).size == trigger.size) }
        for point in outside {
            try await mounted.click(point)
            #expect(presentations == 0)
        }
        try await mounted.click(inside)
        for _ in 0..<10 where presentations == 0 { await mounted.settle() }
        #expect(presentations == 1)
    }

    // Exercise AppKit's native target selection with a real sibling underneath
    // both the rectangular overhang and the clipped-off rounded corner.
    private struct NativeControl: NSViewRepresentable {
        let button: NSButton
        func makeNSView(context: Context) -> NSButton { button }
        func updateNSView(_ view: NSButton, context: Context) {}
    }

    @Test func sharedClipHitTestsOverlappingAppKitControls() async throws {
        let back = NSButton(title: "Back", target: nil, action: nil)
        let front = NSButton(title: "Front", target: nil, action: nil)
        back.isBordered = false
        front.isBordered = false
        let mounted = try await CollectionWindow(#"root = Col([Caption("controls")], 0, "start")"#,
                                                component: { element, _ in
            guard element.typeName == "Caption" else { return nil }
            return AnyView(ZStack(alignment: .leading) {
                NativeControl(button: back).frame(width: 120, height: 40)
                NativeControl(button: front).frame(width: 120, height: 40)
                    .frame(width: 80, height: 40, alignment: .leading)
                    .clipContainer(to: RoundedRectangle(cornerRadius: 12))
            })
        })
        defer { mounted.window.close() }
        let rect = try mounted.rect("/children/0")
        for (point, expected) in [(CGPoint(x: 95, y: 20), back), (CGPoint(x: 1, y: 1), back),
                                  (CGPoint(x: 40, y: 20), front)] {
            let local = CGPoint(x: rect.minX + point.x, y: rect.minY + point.y)
            let target = mounted.host.hitTest(mounted.host.convert(local, to: mounted.host.superview))
            #expect(target === expected, "AppKit must target the \(expected.title) button at \(point)")
        }
    }

    @Test func colClipsOverflowingControl() async throws { try await checkCell("Col([wide], 0)") }
    @Test func rowClipsOverflowingControl() async throws { try await checkCell(#"Row([wide], 0, "start")"#) }
    @Test func boxClipsOverflowingControl() async throws { try await checkCell("Box([wide])") }
    @Test func cardClipsOverflowingControl() async throws { try await checkCell(#"Card([wide], "outline", 0, 0)"#) }
    @Test func gridClipsOverflowingControl() async throws { try await checkCell("Grid([wide], 1, null, 0)") }

    @Test func carouselClipsOverflowingControlAtTheInnerTrack() async throws {
        try await checkTrigger("""
        root = Col([Carousel([wide], 120, 0)], 0)
        wide = Box([Popover("Term", [Caption("Meaning")])], null, 0, null, "red", null, null, 360, 50)
        """, width: 320, outside: [CGPoint(x: 200, y: 8)], inside: CGPoint(x: 12, y: 8))

        // A 34-point arrow also overflows a 10-point-high Carousel. Leave
        // space below it so only the Carousel can reject the outside click.
        let mounted = try await CollectionWindow("""
        root = Col([Carousel([tile,tile,tile], 80, 1), tall], 10)
        tile = Box([], null, null, null, "blue", null, null, null, 10)
        tall = Box([], null, null, null, "red", null, null, 5, 100)
        """, width: 120)
        defer { mounted.window.close() }
        let second = "/children/0/children/1"
        let initialX = try mounted.rect(second).minX
        try #require(initialX == 84, "Item two must start one item and gap from the leading edge")
        let track = try mounted.rect("/children/0")
        let outside = CGPoint(x: 95, y: 16)
        try #require(!track.contains(outside), "Outside click \(outside) must miss track \(track)")
        let inside = CGPoint(x: 95, y: 5)
        let next = try #require(mounted.accessibleButtons.first { $0.accessibilityLabel?() == "Next" })
        let buttonFrame = try #require(next.accessibilityFrame?())
        func screenPoint(_ local: CGPoint) -> CGPoint {
            mounted.window.convertPoint(toScreen: mounted.host.convert(local, to: nil))
        }
        #expect(!buttonFrame.contains(screenPoint(outside)))
        #expect(buttonFrame.contains(screenPoint(inside)))
        try await mounted.click(outside)
        let afterOutsideClick = try mounted.rect(second).minX
        guard afterOutsideClick == initialX else {
            let offset = mounted.scrollView?.contentView.bounds.minX
            throw CollectionInputFailure(description: """
                Carousel click at \(outside) outside \(track) moved item two from \(initialX) to \(afterOutsideClick), \
                native scroll x=\(String(describing: offset))
                """)
        }
        try await mounted.click(inside)
        for _ in 0..<10 where abs(try mounted.rect(second).minX) > 1 { await mounted.settle() }
        let afterInsideClick = try mounted.rect(second).minX
        #expect(abs(afterInsideClick) <= 1, "Clicking Next inside the track must bring item two to the leading edge")
    }

    @Test func popoverClipsOversizedTriggerAtTheGridCell() async throws {
        let source = """
        root = Col([Grid([Popover([f,f], [Caption("Meaning")]), tall], 2, null, 0)], 0)
        f = Box([], null, null, null, "red", null, null, 60, 10)
        tall = Box([], null, null, null, "red", null, null, 5, 100)
        """
        try await checkTrigger(source, width: 150, outside: [CGPoint(x: 95, y: 12), CGPoint(x: 95, y: 50)],
                               inside: CGPoint(x: 12, y: 12),
                               trigger: ("/children/0/children/0", CGSize(width: 75, height: 100)))

        // Mount the presented panel in a wider window so the window edge
        // cannot hide a hit escaping the panel's own bounds.
        var clicks = 0
        let panel = try await CollectionWindow(#"root = Col([Popover("Term", [Button("Choose")])], 0, "start")"#,
                                               width: 240, component: { element, path in
            if element.typeName == "Popover", let content = element.prop("content") {
                return AnyView(PopoverPanel(content: content, path: path, label: "Term", limit: 80, close: {}))
            }
            guard element.typeName == "Button" else { return nil }
            return AnyView(Button { clicks += 1 } label: {
                Text("Choose").frame(width: 180, height: 24).contentShape(Rectangle())
            }.buttonStyle(.plain))
        })
        defer { panel.window.close() }
        let bounds = try panel.rect("/children/0")
        let button = try panel.rect("/children/0/content/0")
        #expect(bounds.width == 80)
        #expect(button.maxX > bounds.maxX)
        try await panel.click(CGPoint(x: button.maxX - 5, y: button.midY))
        #expect(clicks == 0)
        try await panel.click(CGPoint(x: bounds.midX, y: button.midY))
        #expect(clicks == 1)
    }
}
}
