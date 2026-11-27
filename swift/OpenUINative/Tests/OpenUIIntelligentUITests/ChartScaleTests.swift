import AppKit
import Darwin
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

@Suite @MainActor struct ChartScaleTests {
    @Test(NativeTestWatchdog()) func drawsOneDenseLineOfOneHundredThousandPoints() throws {
        let start = ContinuousClock.now
        let data = (0..<100_000).map { n in
            JSONValue.object([("x", .number(Double(n))), ("y", .number(Double(n % 3 + 1)))])
        }
        let element = try Element(json: chart(type: "line", values: data, series: [.string("y")]))
        let clock = ContinuousClock.now
        let view = IntelligentChart(element: element)
        try expectPaint(view, colour: IntelligentTheme.light.chart1)
        metrics("100,000-point line", start: start, drawing: clock)
    }

    // Stays under the standard 60 s limit. On GitHub's runner, in the
    // ci/mainrisk-backstop-a run, this test and
    // PolarChartScaleTests.drawsChartFromAdmittedToolResult printed about
    // 30.6 s and 29.9 s of prepare. `metrics` prints the body time on every run.
    @Test(NativeTestWatchdog()) func drawsSparseScatterFromAdmittedToolResult() async throws {
        // The full review payload is opt-in so the ordinary gate need not
        // admit an 8 MiB tool result on every run.
        let full = ProcessInfo.processInfo.environment["OPENUI_CHART_ABSOLUTE_MAX"] == "1"
        let count = full ? 1_398_080 : 4_096
        let start = ContinuousClock.now
        let rows = [.array([.number(0)])] + Array<JSONValue>(repeating: .object([]), count: count - 1)
        let series = Array<JSONValue>(repeating: .string(""), count: count + 1)
        let payload = chart(type: "scatter", values: rows, series: series)
        if full { try #require(payload.compact.utf8.count == 8_388_606) }

        NativeTestWatchdog.waitingFor("Core to load the Intelligent UI library")
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        let funding = String(repeating: "a", count: 132_000)
        let source = """
        q = Query("chart", {})
        root = q == "\(funding)" ? Chart("scatter", []) : q
        """
        try live.attachToolProvider { _, _ in payload }
        NativeTestWatchdog.waitingFor("Core to show the next test program")
        try await live.show(response: source, streaming: false)
        // The watchdog bounds this wait.
        NativeTestWatchdog.waitingFor("the chart tool result to become the live root")
        while live.root == nil, live.queries.errors.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
        }
        try #require(live.queries.errors.isEmpty)
        try #require(!live.sizeLimited && !live.depthLimited)
        let element = try #require(live.root, "Timed out waiting for the chart tool result to become the live root")
        guard case let .list(actualRows)? = element.prop("data"),
              case let .list(actualSeries)? = element.prop("series") else {
            Issue.record("The query did not return a Chart")
            return
        }
        try #require(actualRows.count == count)
        try #require(actualSeries.count == count + 1)
        let clock = ContinuousClock.now
        let theme = IntelligentTheme.light
        let palette = [theme.chart1, theme.chart2, theme.chart3, theme.chart4, theme.chart5, theme.chart6]
        let swatch = try #require(palette.dropFirst(count % palette.count).first)
        try expectPaint(ElementView(live: live), colour: swatch)
        metrics("\(count) rows × \(count + 1) series entries", start: start, drawing: clock)
    }

    private func chart(type: String, values: [JSONValue], series: [JSONValue]) -> JSONValue {
        .object([
            ("type", .string("element")), ("typeName", .string("Chart")),
            ("props", .object([("type", .string(type)), ("data", .array(values)), ("series", .array(series))])),
            ("partial", .bool(true)), ("hasDynamicProps", .bool(true)),
        ])
    }

    private func expectPaint(_ source: some View, colour: ThemeColor) throws {
        let view = source.frame(width: 320, height: 320, alignment: .topLeading)
            .intelligentUIMode(.light)
        NativeTestWatchdog.waitingFor("SwiftUI to lay out and render the chart")
        let image = try #require(ImageRenderer(content: view).cgImage)
        let bitmap = NSBitmapImageRep(cgImage: image)
        let expected = colour.sRGB
        let hasMark = (8..<250).contains { y in
            (24..<312).contains { x in
                guard let pixel = bitmap.sRGBColor(x: x, y: y) else { return false }
                return pixel.alphaComponent > 0.3 &&
                    abs(pixel.redComponent - expected.redComponent) < 0.05 &&
                    abs(pixel.greenComponent - expected.greenComponent) < 0.05 &&
                    abs(pixel.blueComponent - expected.blueComponent) < 0.05
            }
        }
        #expect(hasMark)
    }

    private func metrics(_ name: String, start: ContinuousClock.Instant, drawing: ContinuousClock.Instant) {
        let end = ContinuousClock.now
        var usage = rusage()
        if getrusage(RUSAGE_SELF, &usage) == 0 {
            print("Chart scale \(name), prepare \(start.duration(to: drawing)), draw \(drawing.duration(to: end)), " +
                  "total \(start.duration(to: end)), peak RSS \(usage.ru_maxrss) bytes")
        }
    }
}
