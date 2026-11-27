import AppKit
import Darwin
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

@Suite @MainActor struct PolarChartScaleTests {
    @Test(NativeTestWatchdog()) func drawsOneHundredThousandPieSlices() throws {
        let start = ContinuousClock.now
        let rows = (0..<100_000).map { n in
            JSONValue.object([("name", .number(Double(n))), ("value", .number(Double(n % 3 + 1)))])
        }
        let element = try Element(json: chart(type: "pie", rows: rows))
        let drawing = ContinuousClock.now
        try expectPaint(ElementView(element: element).environment(\.elementDrawing, intelligentDrawing()))
        metrics("100,000 pie slices", start: start, drawing: drawing)
    }

    @Test func drawsChartFromAdmittedToolResult() async throws {
        let full = ProcessInfo.processInfo.environment["OPENUI_POLAR_CHART_ABSOLUTE_MAX"] == "1"
        let count = full ? 65_536 : 512
        let type = full ? "pie" : "donut"
        let start = ContinuousClock.now
        func rows(padding: Int = 0, remainder: Int = 0) -> [JSONValue] {
            let tail = String(repeating: "a", count: padding)
            return (0..<count).map { n in
                let label = String(n) + tail + (n < remainder ? "a" : "")
                return .object([("name", .string(label)), ("value", .number(Double(n % 3 + 1)))])
            }
        }

        // Fill the tool result with category text the chart actually uses.
        let target = 8 * 1024 * 1024 - 2
        let base = chart(type: type, rows: rows())
        let remaining = full ? target - base.compact.utf8.count : 0
        let payload = full ? chart(type: type, rows: rows(padding: remaining / count, remainder: remaining % count)) : base
        if full { try #require(payload.compact.utf8.count == target) }

        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        let funding = full ? String(repeating: "a", count: 132_000) : ""
        let source = """
        q = Query("chart", {})
        root = q == "\(funding)" ? Chart("donut", []) : q
        """
        try live.attachToolProvider { _, _ in payload }
        try await live.show(response: source, streaming: false)
        let deadline = ContinuousClock.now + .seconds(180)
        while live.root == nil, live.queries.errors.isEmpty, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        try #require(live.queries.errors.isEmpty)
        try #require(!live.sizeLimited && !live.depthLimited)
        let element = try #require(live.root, "Timed out waiting for the polar Chart tool result to become the live root")
        guard case let .list(actual)? = element.prop("data") else {
            Issue.record("The query did not return Chart rows")
            return
        }
        try #require(actual.count == count)
        let drawing = ContinuousClock.now
        try expectPaint(ElementView(live: live))
        metrics(full ? "65,536 slices in an 8,388,606-byte tool result" : "512-slice tool result", start: start, drawing: drawing)
    }

    private func chart(type: String, rows: [JSONValue]) -> JSONValue {
        let props: [(String, JSONValue)] = [
            ("type", .string(type)), ("x", .string("name")), ("series", .array([.string("value")])),
            ("data", .array(rows)),
        ]
        return .object([
            ("type", .string("element")), ("typeName", .string("Chart")), ("props", .object(props)),
            ("partial", .bool(true)), ("hasDynamicProps", .bool(true)),
        ])
    }

    private func expectPaint(_ plot: some View) throws {
        let view = plot.frame(width: 280, height: 320, alignment: .topLeading).intelligentUIMode(.light)
        NativeTestWatchdog.waitingFor("SwiftUI to lay out and render the polar chart")
        let image = try #require(ImageRenderer(content: view).cgImage)
        let bitmap = NSBitmapImageRep(cgImage: image)
        // Sample the ring and its interior, well above the legend, and allow
        // for mixed colours where many narrow sectors share a pixel.
        let hasSector = (25..<255).contains { y in
            (25..<255).contains { x in
                guard let pixel = bitmap.sRGBColor(x: x, y: y) else { return false }
                return pixel.alphaComponent > 0.25 &&
                    max(pixel.redComponent, pixel.greenComponent, pixel.blueComponent) -
                    min(pixel.redComponent, pixel.greenComponent, pixel.blueComponent) > 0.15
            }
        }
        #expect(hasSector)
    }

    private func intelligentDrawing() -> ElementDrawing {
        // The bare element uses the same chart renderer as the live library.
        ElementDrawing(component: { element, _ in AnyView(IntelligentChart(element: element)) })
    }

    private func metrics(_ name: String, start: ContinuousClock.Instant, drawing: ContinuousClock.Instant) {
        let end = ContinuousClock.now
        var usage = rusage()
        if getrusage(RUSAGE_SELF, &usage) == 0 {
            print("Polar chart \(name), prepare \(start.duration(to: drawing)), draw \(drawing.duration(to: end)), " +
                  "total \(start.duration(to: end)), peak RSS \(usage.ru_maxrss) bytes")
        }
    }
}
