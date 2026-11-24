// Mount the real library and deliver AppKit input, as in OpenUINative's ClickTests.
import AppKit
import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

@Suite(.serialized) struct CollectionDrawingTests {}

// Swift Testing prints a caught error's description on the CI issue line.
struct CollectionInputFailure: Error, CustomStringConvertible {
    let description: String
}

extension Trait where Self == ConditionTrait {
    static var requiresDisplay: Self {
        .enabled("No macOS display is available") { @MainActor in !NSScreen.screens.isEmpty }
    }
}

@MainActor final class CollectionWindow {
    let window: NSWindow
    let host: NSHostingView<AnyView>
    private var frames: [String: CGRect] = [:]

    init(_ program: String, width: CGFloat = 320, scrollable: Bool = false,
         component: @escaping @MainActor @Sendable (Element, String) -> AnyView? = { _, _ in nil }) async throws {
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: program, streaming: false)
        #expect(live.errors.map(\.message) == [])
        _ = NSApplication.shared
        host = NSHostingView(rootView: AnyView(EmptyView()))
        window = NSWindow(contentRect: CGRect(x: 60, y: 60, width: width, height: 200),
                          styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let drawing = ElementDrawing(component: component, chrome: { [weak self] view, _, path in
            AnyView(view.onGeometryChange(for: CGRect.self) { $0.frame(in: .named("window")) } action: {
                self?.frames[path] = $0
            })
        })
        let page = ElementView(live: live).environment(\.elementDrawing, drawing)
        let content = scrollable
            ? AnyView(ScrollView { page.frame(minHeight: 400, alignment: .topLeading) })
            : AnyView(page)
        host.rootView = AnyView(content
            .environment(\.accessibilityEnabled, true)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).coordinateSpace(name: "window")
            .transaction { if NSScreen.screens.isEmpty { $0.animation = nil } })
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
        await settle()
    }

    func settle() async {
        try? await Task.sleep(for: .milliseconds(100))
        host.layoutSubtreeIfNeeded()
    }

    func rect(_ path: String) throws -> CGRect { try #require(frames[path]) }

    var scrollView: NSScrollView? {
        var pending = [host as NSView]
        while let view = pending.popLast() {
            if let scroll = view as? NSScrollView { return scroll }
            pending.append(contentsOf: view.subviews)
        }
        return nil
    }

    var accessibleButtons: [AnyObject] {
        var pending: [AnyObject] = [host]
        var buttons: [AnyObject] = []
        while let node = pending.popLast() {
            if node.accessibilityRole?() == .button {
                buttons.append(node)
            }
            pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
        }
        return buttons
    }

    var buttonLabels: [String] {
        accessibleButtons.map { $0.accessibilityLabel?() ?? $0.accessibilityTitle?() ?? "" }
    }

    func click(_ point: CGPoint, throughApplication: Bool = false) async throws {
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try #require(NSEvent.mouseEvent(with: type, location: host.convert(point, to: nil),
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
            if throughApplication { NSApp.sendEvent(event) } else { window.sendEvent(event) }
        }
        await settle()
    }
}
