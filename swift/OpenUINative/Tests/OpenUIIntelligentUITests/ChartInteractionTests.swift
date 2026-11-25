import AppKit
import Charts
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

extension CollectionDrawingTests {
@Suite(NativeTestWatchdog()) @MainActor struct ChartInteractionTests {
    @Test func clippedLegendRejectsOutsideClicksAndTogglesOnlyItsSeries() async throws {
        let window = try await CollectionWindow("""
        root = Col([Chart("bar", [{name:"Jan", sales:3, cost:2}], null, null, null, null, 9), Divider()], 0)
        """)
        defer { window.window.closeForTest() }
        let sales = try #require(window.accessibleButtons.first { $0.accessibilityLabel?() == "Hide sales" })
        let chart = try window.rect("/children/0")
        let button = try #require(sales.accessibilityFrame?())
        let local = window.host.convert(window.window.convertFromScreen(button), from: nil)
        let outside = CGPoint(x: local.midX, y: chart.maxY + 1)
        let inside = CGPoint(x: local.midX, y: chart.intersection(local).midY)
        try #require(local.contains(outside) && !chart.contains(outside))
        try #require(local.contains(inside) && chart.contains(inside))
        try await window.click(outside)
        #expect(window.buttonLabels.contains("Hide sales"))
        try await window.click(inside)
        #expect(window.buttonLabels.contains("Show sales"))
        let cost = try #require(window.accessibleButtons.first { $0.accessibilityLabel?() == "Hide cost" })
        let costFrame = try #require(cost.accessibilityFrame?())
        let costLocal = window.host.convert(window.window.convertFromScreen(costFrame), from: nil)
        try await window.click(CGPoint(x: costLocal.midX, y: inside.y))
        #expect(window.buttonLabels.contains("Hide cost"), "The last visible series stays drawn")
        try await window.click(inside)
        #expect(window.buttonLabels.contains("Hide sales"))
    }
}
}

extension CollectionDrawingTests.ChartInteractionTests {
    @Test func scrollingPlotClipsAnOverflowingControlAtItsViewport() async throws {
        var clicks = 0
        let window = try await CollectionWindow(#"root = Col([Caption("plot")], 0, "start")"#) { _, path in
            guard path == "/children/0" else { return nil }
            let rows = (0..<10).map(String.init)
            let plot = Chart(rows, id: \.self) { x in
                BarMark(x: .value("x", x), y: .value("y", 1))
            }
            .chartOverlay { _ in
                Button { clicks += 1 } label: {
                    Color.blue.frame(width: 240, height: 26).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
            .modifier(CartesianChartViewport(categoryCount: rows.count))
            .frame(width: 144, height: 80)
            return AnyView(plot)
        }
        defer { window.window.closeForTest() }
        let scroll = try #require(window.scrollView)
        let clip = scroll.contentView
        #expect(clip.documentRect.width > clip.bounds.width)
        clip.scroll(to: CGPoint(x: clip.documentRect.width - clip.bounds.width, y: 0))
        scroll.reflectScrolledClipView(clip)
        await window.settle()
        #expect(clip.bounds.minX > 0)
        let rect = try window.rect("/children/0")
        let midY = rect.midY
        try await window.click(CGPoint(x: rect.maxX + 5, y: midY))
        #expect(clicks == 0)
        try await window.click(CGPoint(x: rect.midX, y: midY))
        #expect(clicks == 1)
    }
}

extension CollectionDrawingTests.ChartInteractionTests {
    @Test(arguments: ["bar", "line", "area"], [false, true])
    func hidingLargeSeriesRevealsBothSurvivingValues(kind: String, extreme: Bool) async throws {
        let (large, small, twiceSmall) = extreme ? ("1e308", "1e-308", "2e-308") : ("1e6", "1", "2")
        let window = try await CollectionWindow("""
        root = Chart("\(kind)", [{x:"A", large:\(large), small:\(small)},
                                {x:"B", large:\(large), small:\(twiceSmall)}],
                      "x", ["large", "small"], null, true)
        """, height: 340)
        defer { window.window.closeForTest() }
        window.host.appearance = NSAppearance(named: .aqua)
        let largeButton = try #require(window.accessibleButtons.first { $0.accessibilityLabel?() == "Hide large" })
        let frame = try #require(largeButton.accessibilityFrame?())
        let local = window.host.convert(window.window.convertFromScreen(frame), from: nil)
        try await window.click(CGPoint(x: local.midX, y: local.midY))
        #expect(Set(window.buttonLabels) == ["Show large", "Hide small"])

        let bounds = window.host.bounds
        let bitmap = try #require(window.host.bitmapImageRepForCachingDisplay(in: bounds))
        window.host.cacheDisplay(in: bounds, to: bitmap)
        let green = try LayoutPicture.swatch(IntelligentTheme.light.chart2.color)
        // Find the top of the remaining bar or stroke at either end of
        // the drawing. Upstream places 1 and 2 halfway up and at the top.
        let plotBottom = Int(285 * CGFloat(bitmap.pixelsHigh) / bounds.height)
        var tops: [(x: Int, y: Int)] = []
        for x in 0..<bitmap.pixelsWide {
            if let top = (0..<plotBottom).first(where: { y in
                guard let colour = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { return false }
                return colour.alphaComponent > 0.5 &&
                    abs(colour.redComponent - green.redComponent) < 0.06 &&
                    abs(colour.greenComponent - green.greenComponent) < 0.06 &&
                    abs(colour.blueComponent - green.blueComponent) < 0.06
            }) {
                tops.append((x, top))
            }
        }
        let left = try #require(tops.first)
        let right = try #require(tops.last)
        #expect(right.x - left.x > bitmap.pixelsWide / 4)
        #expect(left.y - right.y > plotBottom / 4)
    }

    @Test(arguments: ["bar", "line", "area", "scatter"])
    func removedLegendKeysStopRetainingVisibilityState(kind: String) async throws {
        func program(rotating key: String) -> String {
            """
            root = Col([Chart("\(kind)", [{x:1, fixed:5, stay:3, \(key):1}], "x",
                ["fixed", "stay", "\(key)"]), Divider()], 0)
            """
        }
        let window = try await CollectionWindow(program(rotating: "k1"), height: 340)
        defer { window.window.closeForTest() }
        func hide(_ key: String) async throws {
            let button = try #require(window.accessibleButtons.first { $0.accessibilityLabel?() == "Hide " + key })
            let frame = try #require(button.accessibilityFrame?())
            let local = window.host.convert(window.window.convertFromScreen(frame), from: nil)
            try await window.click(CGPoint(x: local.midX, y: local.midY))
            #expect(window.buttonLabels.contains("Show " + key))
        }

        try await hide("stay")
        try await hide("k1")
        NativeTestWatchdog.waitingFor("Core to show the updated chart program")
        try await window.live.show(response: program(rotating: "k2"), streaming: false)
        await window.settle()
        #expect(Set(window.buttonLabels) == ["Hide fixed", "Show stay", "Hide k2"])
        try await hide("k2")
        NativeTestWatchdog.waitingFor("Core to show the updated chart program")
        try await window.live.show(response: program(rotating: "k1"), streaming: false)
        await window.settle()
        #expect(Set(window.buttonLabels) == ["Hide fixed", "Show stay", "Hide k1"])
    }

    @Test func liveChartUpdateDisablesAnIncomingAnimation() async throws {
        func program(height: Int) -> String {
            """
            root = Col([Chart("line", [{x:1,a:2},{x:2,a:3}], "x", ["a"], null, null, \(height)), Divider()], 0)
            """
        }
        let trace = ChartTransactionTrace()
        let window = try await CollectionWindow(program(height: 130), height: 340) { element, path in
            guard path == "/children/0", let data = CartesianChartData(element, kind: .line),
                  let height = data.explicitHeight else { return nil }
            // The production layout owns the animation boundary. Host a view
            // that can read its incoming SwiftUI transaction as the data updates.
            return AnyView(CartesianChartLayout(explicitHeight: height) {
                ChartTransactionProbe(height: height, trace: trace)
            } legend: {
                Color.clear.frame(height: 14)
            }
            .transaction(value: height) { transaction in
                transaction.animation = .linear(duration: 30)
                trace.animatedInputs.insert(height)
            })
        }
        defer { window.window.closeForTest() }
        NativeTestWatchdog.waitingFor("Core to show the updated chart program")
        try await window.live.show(response: program(height: 170), streaming: false)
        await window.settle()

        #expect(trace.animatedInputs.contains(170))
        let update = try #require(trace.observed.last { $0.height == 170 })
        #expect(update.transaction.animation == nil)
        #expect(update.transaction.disablesAnimations)
        #expect(try window.rect("/children/1").minY == 170)
    }
}

@MainActor private final class ChartTransactionTrace {
    var animatedInputs: Set<CGFloat> = []
    var observed: [(height: CGFloat, transaction: Transaction)] = []
}

private struct ChartTransactionProbe: NSViewRepresentable {
    let height: CGFloat
    let trace: ChartTransactionTrace

    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ view: NSView, context: Context) {
        trace.observed.append((height, context.transaction))
    }
}
