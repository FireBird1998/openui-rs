import AppKit
import OpenUINative
import SwiftUI
import Testing
import ViewerKit

@MainActor private final class FallbackScroll {
    var proxy: ScrollViewProxy?
    var offset: CGFloat = 0
}

@Suite(.serialized) @MainActor struct LongViewerDrawing {
    @Test(arguments: ["props", "list", "record"], [1_001, 5_000])
    func everyFallbackCollectionKeepsItsLastEntryReachable(_ kind: String, _ count: Int) async throws {
        var element = try #require(ParseResult(json: #"{"root":{"type":"element","typeName":"Unsupported","props":{"data":[]},"partial":false,"hasDynamicProps":false},"meta":{}}"#, depthLimited: false, sizeLimited: false).root)
        let item = try #require(ParseResult(json: #"{"root":{"type":"element","typeName":"TextContent","props":{"text":"Item"},"partial":false,"hasDynamicProps":false},"meta":{}}"#, depthLimited: false, sizeLimited: false).root)
        let sample = try #require(element.props.first)
        let last: String
        if kind == "props" || kind == "record" {
            let fields = (0..<count).map { index in
                var field = sample
                field.name = "p\(index)"
                field.value = .element(item)
                return field
            }
            if kind == "props" { element.props = fields; last = "/p\(count - 1)" }
            else { element.props[0].value = .record(fields); last = "/data/p\(count - 1)" }
        } else {
            element.props[0].value = .list(Array(repeating: .element(item), count: count))
            last = "/data/\(count - 1)"
        }
        let root = element
        let observed = FallbackScroll()
        let host = NSHostingView(rootView: ScrollViewReader { proxy in
            ScrollView { ElementBox(element: root) }
                .onAppear { observed.proxy = proxy }
                .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y } action: { _, offset in observed.offset = offset }
        })
        let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 500, height: 400), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }
        // The view appears and scrolls on later run-loop turns, and how many it
        // takes depends on the machine: ask again until it has scrolled, up to a
        // bound. A notice, omitted final entry or clipped list never scrolls here.
        for _ in 0..<200 where observed.offset <= CGFloat(count) {
            try await Task.sleep(for: .milliseconds(50))
            host.layoutSubtreeIfNeeded()
            observed.proxy?.scrollTo(last, anchor: .bottom)
        }
        try #require(observed.proxy != nil)
        #expect(observed.offset > CGFloat(count))
    }
}
