import AppKit
import Charts
import OpenUINative
import SwiftUI
import Testing

private let largeSchema = #"""
{"$defs": {
"Stack":{"properties":{"children":{"type":"array"}}},
"Card":{"properties":{"children":{"type":"array"}}},
"TextContent":{"properties":{"text":{"type":"string"}}},
"Table":{"properties":{"columns":{"type":"array"},"rows":{"type":"array"}}},
"Col":{"properties":{"label":{"type":"string"},"type":{"type":"string"}}},
"LineChart":{"properties":{"labels":{"type":"array"},"series":{"type":"array"}}},
"Series":{"properties":{"category":{"type":"string"},"values":{"type":"array"}}}
}}
"""#

@MainActor private func largeProgram(_ source: String) async throws -> LiveProgram {
    let core = Core()
    let library = try await core.library(schema: largeSchema)
    let live = try LiveProgram(library: library)
    try live.show(try await core.parseOneShot(source, library: library))
    return live
}

@MainActor private final class Reachability {
    var seen: Set<String> = []
    var fallbacks: Set<String> = []
    var proxy: ScrollViewProxy?
    var lastFrame = CGRect.zero
}

/// A real scroll viewport, with the app's public chrome hook observing which
/// children SwiftUI actually realizes. Scrolling must make every item reachable.
@MainActor private final class ScrollingDrawing {
    let observed = Reachability()
    let host: NSHostingView<AnyView>
    let window: NSWindow

    init(_ live: LiveProgram, last: String) async {
        _ = NSApplication.shared
        let observed = observed
        let drawing = ElementDrawing(chrome: { view, element, path in
            let content = element.typeName == "TextContent"
                ? AnyView(view.onAppear { observed.seen.insert(path) }) : view
            if path == last {
                return AnyView(content.id(path).onGeometryChange(for: CGRect.self) {
                    $0.frame(in: .named("viewport"))
                } action: { observed.lastFrame = $0 })
            }
            return AnyView(content.id(path))
        }, fallback: { element, _ in
            observed.fallbacks.insert(element.typeName)
            return AnyView(Text("Unexpected fallback"))
        })
        host = NSHostingView(rootView: AnyView(ScrollViewReader { proxy in
            ScrollView {
                ElementView(live: live).environment(\.elementDrawing, drawing)
            }
            .coordinateSpace(name: "viewport")
            .onAppear { observed.proxy = proxy }
        }))
        window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 600, height: 400), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        await settle()
    }

    func settle() async {
        await layoutOnMainQueue(host)
    }

    func reveal(_ path: String, until visible: () -> Bool) async throws {
        let proxy = try #require(observed.proxy)
        if path.hasPrefix("/children/"), let index = Int(path.split(separator: "/").last ?? "") {
            // ForEach's public scroll identity is its index; the app chrome's
            // nested path ID need not be discovered before lazy realization.
            proxy.scrollTo(index, anchor: .center)
        } else { proxy.scrollTo(path, anchor: .center) }
        // A deep container can require hundreds of jumps. Pay for another
        // run-loop turn only while SwiftUI has yet to realize the target.
        let deadline = ContinuousClock.now + .seconds(5)
        while .now < deadline {
            host.layoutSubtreeIfNeeded()
            guard .now < deadline else { break }
            let isVisible = visible()
            guard .now < deadline else { break }
            if isVisible { return }
            try await Task.sleep(until: min(deadline, .now + .milliseconds(5)), clock: .continuous)
        }
        try #require(ContinuousClock.now < deadline, "timed out waiting for \(path) to become visible after scrolling")
    }

    var lastIsVisible: Bool {
        observed.lastFrame.intersects(CGRect(x: 0, y: 0, width: 600, height: 400))
    }

    func requireLastVisible() throws {
        #expect(lastIsVisible)
        #expect(!observed.lastFrame.isEmpty)
        #expect(observed.fallbacks.isEmpty)
    }
}

extension NativeUITests {
@Suite @MainActor struct LargeDataDrawing {
    @Test(arguments: ["Stack", "Card"], [1_001, 5_000])
    func everyChildOfALargeContainerRemainsNativeAndReachable(_ type: String, _ count: Int) async throws {
        let children = (0..<count).map { "TextContent(\"child-\($0)\")" }.joined(separator: ",")
        let live = try await largeProgram("root = \(type)([\(children)])")
        let root = try #require(live.root)
        let native = NativeComponents.view(for: root, path: "/") != nil
        try #require(native)
        let state = live.state
        let version = live.version
        let paths = (0..<count).map { "/children/\($0)" }
        let window = await ScrollingDrawing(live, last: try #require(paths.last))
        defer { window.window.close() }
        // Jump to each unrealized child; require progress after every jump.
        for path in paths where !window.observed.seen.contains(path) {
            try await window.reveal(path) { window.observed.seen.contains(path) }
        }
        #expect(window.observed.seen == Set(paths))
        try await window.reveal(try #require(paths.last)) { window.lastIsVisible }
        try window.requireLastVisible()
        #expect(live.state == state && live.version == version)
    }

    @Test(arguments: [200, 2_000])
    func tablesKeepEveryCellNativeAndTheirLastRowReachable(_ rows: Int) async throws {
        let columns = rows == 200 ? 6 : 3
        let headers = (0..<columns).map { "Col(\"C\($0)\")" }.joined(separator: ",")
        let data = (0..<rows).map { r in
            "[" + (0..<columns).map { "TextContent(\"r\(r)c\($0)\")" }.joined(separator: ",") + "]"
        }.joined(separator: ",")
        let live = try await largeProgram("root = Table([\(headers)], [\(data)])")
        let native = NativeComponents.view(for: try #require(live.root), path: "/") != nil
        try #require(native)
        let state = live.state
        let version = live.version
        let last = "/rows/\(rows - 1)/\(columns - 1)"
        let window = await ScrollingDrawing(live, last: last)
        defer { window.window.close() }
        let paths = Set((0..<rows).flatMap { r in (0..<columns).map { "/rows/\(r)/\($0)" } })
        #expect(window.observed.seen == paths)
        try await window.reveal(last) { window.lastIsVisible }
        try window.requireLastVisible()
        #expect(live.state == state && live.version == version)
    }

    @Test func allTwelveHundredChartPointsReachTheChartDomain() async throws {
        let labels = (0..<1_200).map { "\"p\($0)\"" }.joined(separator: ",")
        let values = (0..<1_200).map(String.init).joined(separator: ",")
        let live = try await largeProgram("root = LineChart([\(labels)], [Series(\"S\", [\(values)])])")
        let native = NativeComponents.view(for: try #require(live.root), path: "/") != nil
        try #require(native)
        let state = live.state
        let version = live.version
        var reachable: Set<Int> = []
        let window = await Window(live, component: { element, path in
            NativeComponents.view(for: element, path: path).map { view in
                AnyView(view.chartOverlay { proxy in
                    Color.clear.onAppear {
                        reachable = Set((0..<1_200).filter { proxy.position(forX: "p\($0)") != nil })
                    }
                })
            }
        })
        defer { window.close() }
        #expect(reachable == Set(0..<1_200))
        #expect(live.state == state && live.version == version)
    }

    private func selectProgram() async throws -> LiveProgram {
        let items = (0..<1_500).map { "SelectItem(\"v\($0)\", \"Choice \($0)\")" }.joined(separator: ",")
        return try await live("$v = \"v0\"\nroot = Select(\"s\", [\(items)], $v)", bindings: [("Select", "value")])
    }

    @Test func aFifteenHundredOptionSelectStillDrawsNatively() async throws {
        let live = try await selectProgram()
        let native = NativeComponents.view(for: try #require(live.root), path: "/") != nil
        #expect(native)
        let state = live.state
        let version = live.version
        let window = await Window(live)
        defer { window.close() }
        #expect(live.state == state && live.version == version)
        #expect(live.value(of: "$v") == .string("v0"))
    }

    @Test(.requiresDisplay) func everyOptionOfALongSelectIsInItsMenuAndTheLastCanBePicked() async throws {
        let live = try await selectProgram()
        let window = await Window(live)
        defer { window.close() }
        let menu = try await window.pick("Choice 1499", at: "/")
        #expect(menu.map(\.title) == ["s"] + (0..<1_500).map { "Choice \($0)" })
        #expect(live.value(of: "$v") == .string("v1499"))
    }
}
}
