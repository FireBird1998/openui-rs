import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

@Suite @MainActor struct ChartDrawingTests {
    // The sales and cost rows from the library's all-components example.
    // Scatter uses a numeric month so the horizontal distance has meaning.
    private func picture(_ kind: String, _ options: String = "") async throws -> LayoutPicture {
        let rows = kind == "scatter"
            ? "[{month: 1, sales: 3, cost: 2}, {month: 2, sales: 5, cost: 4}]"
            : #"[{month: "Jan", sales: 3, cost: 2}, {month: "Feb", sales: 5, cost: 4}]"#
        return try await LayoutPicture("""
        root = Col([Chart("\(kind)", \(rows), "month", ["sales", "cost"]\(options))], 0)
        """, width: 320, height: 320)
    }

    @Test(arguments: ["bar", "line", "area", "scatter"])
    func drawsRecordedValuesInTheirSeriesColours(kind: String) async throws {
        let image = try await picture(kind)
        let blue = IntelligentTheme.light.chart1.sRGB
        let green = IntelligentTheme.light.chart2.sRGB
        // In January, sales = 3 (blue) and cost = 2 (green). Before the
        // midpoint of the plot the higher value must reach further up.
        let sales = try #require(try firstPaint(in: image, colour: blue, xs: 0..<160, ys: 0..<285))
        let cost = try #require(try firstPaint(in: image, colour: green, xs: 0..<160, ys: 0..<285))
        #expect(sales < cost)
        #expect(try firstPaint(in: image, colour: blue, xs: 0..<40, ys: 296..<320) != nil)
        #expect(try firstPaint(in: image, colour: green, xs: 40..<120, ys: 296..<320) != nil)
    }

    private func firstPaint(in picture: LayoutPicture, colour: NSColor, xs: Range<Int>, ys: Range<Int>) throws -> Int? {
        for y in ys {
            for x in xs where matches(try picture.pixel(x, y), colour) {
                return y
            }
        }
        return nil
    }

    private func matches(_ pixel: NSColor, _ swatch: NSColor, alpha: CGFloat = 0.3) -> Bool {
        pixel.alphaComponent > alpha &&
            abs(pixel.redComponent - swatch.redComponent) < 0.05 &&
            abs(pixel.greenComponent - swatch.greenComponent) < 0.05 &&
            abs(pixel.blueComponent - swatch.blueComponent) < 0.05
    }
}

extension ChartDrawingTests {
    @Test(arguments: ["bar", "line", "area", "scatter"])
    func formatAndStackingAffectOnlyTheSpecifiedDrawings(kind: String) async throws {
        let basic = try await picture(kind)
        let percent = try await picture(kind, #", "percent", false"#)
        let stacked = try await picture(kind, #", "number", true"#)
        let original = try basic.renderedPixels()
        #expect(try percent.renderedPixels() == original)
        let changes = try stacked.renderedPixels() != original
        #expect(changes == (kind == "bar" || kind == "area"))
    }

    @Test func explicitHeightAndEmptyChartsTakeTheirActualSpace() async throws {
        for (rows, gap) in [("[]", ""), ("[{}]", ", 2")] {
            let picture = try await LayoutPicture("""
            root = Col([Chart("line", \(rows)), Divider()]\(gap))
            """)
            #expect(try picture.rect("/children/1").minY == 0)
        }
        let sized = try await picture("bar", ", null, null, 180")
        #expect(try sized.rect("/children/0").height == 180)
    }

    @Test func groupedBarsUseTheSixThemeColoursInOrderAndRepeat() async throws {
        let chart = try await LayoutPicture("""
        root = Col([Chart("bar", [{label: "Jan", a: 1, b: 1, c: 1, d: 1, e: 1, f: 1, g: 1}])], 0)
        """, width: 490, height: 300, mode: .dark)
        let theme = IntelligentTheme.dark
        let colours = try [theme.chart1, theme.chart2, theme.chart3, theme.chart4, theme.chart5, theme.chart6]
            .map { try LayoutPicture.swatch($0.color) }
        var bars: [Int] = []
        for x in 0..<490 {
            let pixel = try chart.pixel(x, 130)
            if pixel.alphaComponent > 0.95, let index = colours.firstIndex(of: pixel), bars.last != index {
                bars.append(index)
            }
        }
        #expect(bars == [0, 1, 2, 3, 4, 5, 0])
    }
}

extension ChartDrawingTests {
    @Test(arguments: ["bar", "line", "area", "scatter"])
    func markGeometryDistinguishesTheFourChartTypes(kind: String) async throws {
        let image = try await LayoutPicture("""
        root = Col([Chart("\(kind)", [{x: 1, y: 2}, {x: 2, y: 4}, {x: 10, y: 2}])], 0)
        """, width: 320, height: 320)
        let colour = IntelligentTheme.light.chart1.sRGB
        var groups: [ClosedRange<Int>] = []
        var longestColumn = 0
        // Count painted columns within the plot, clear of the legend and x labels.
        for x in 0..<320 {
            var height = 0
            for y in 0..<280 {
                let p = try image.pixel(x, y)
                if p.alphaComponent > 0.2 && abs(p.blueComponent - colour.blueComponent) < 0.05
                    && abs(p.redComponent - colour.redComponent) < 0.05
                    && abs(p.greenComponent - colour.greenComponent) < 0.05 { height += 1 }
            }
            if height > 0 {
                if let last = groups.last, last.upperBound == x - 1 {
                    groups.removeLast()
                    groups.append(last.lowerBound...x)
                } else {
                    groups.append(x...x)
                }
            }
            longestColumn = max(longestColumn, height)
        }
        let filled = kind == "bar" || kind == "area"
        #expect((longestColumn > 20) == filled)
        if kind == "bar" || kind == "scatter" {
            try #require(groups.count == 3)
            if kind == "scatter" {
                #expect(groups.allSatisfy { $0.count <= 9 })
                // Equal category spacing would put the middle dot halfway across.
                #expect(groups[1].lowerBound - groups[0].lowerBound < (groups[2].lowerBound - groups[1].lowerBound) / 3)
            }
        } else {
            #expect(groups.count == 1)
            #expect((groups.first?.count ?? 0) > 80)
        }
    }
}

extension ChartDrawingTests {
    @Test func extremeFiniteInputsSettleWithoutBreakingTheFollowingDrawing() async throws {
        for kind in ["bar", "line", "area", "scatter"] {
            let image = try await LayoutPicture("""
            root = Col([Chart("\(kind)", [{x:1e308, a:1e308, b:1e308}, {x:-1e308, a:-1e308, b:1e308}], "x", ["a","b"], null, true, 100), Divider()], 0)
            """, width: 320, height: 200)
            #expect(try image.rect("/children/1").height == 1)
        }
    }
}

extension ChartDrawingTests {
    @Test(arguments: ["bar", "area"])
    func stackingPlacesTheSecondSeriesAboveTheFirst(kind: String) async throws {
        let image = try await LayoutPicture("""
        root = Col([Chart("\(kind)", [{x:"one", low:1, high:2}, {x:"two", low:1, high:2}], "x", ["low","high"], null, true)], 0)
        """, width: 320, height: 320)
        let blue = IntelligentTheme.light.chart1.sRGB
        let green = IntelligentTheme.light.chart2.sRGB
        // Find the plot column with the most series paint rather than assuming
        // where the first category lands: axis label widths differ by screen.
        func column(_ x: Int) throws -> (lower: [Int], upper: [Int]) {
            var lower: [Int] = [], upper: [Int] = []
            for y in 0..<260 {
                let pixel = try image.pixel(x, y)
                guard pixel.alphaComponent > 0.25 else { continue }
                if abs(pixel.blueComponent - blue.blueComponent) < 0.04 && pixel.redComponent < 0.06 { lower.append(y) }
                if abs(pixel.blueComponent - green.blueComponent) < 0.04 && pixel.redComponent < 0.06 { upper.append(y) }
            }
            return (lower, upper)
        }
        var best = (x: 0, lower: [Int](), upper: [Int]())
        for x in stride(from: 40, to: 320, by: 4) {
            let found = try column(x)
            if min(found.lower.count, found.upper.count) > min(best.lower.count, best.upper.count) {
                best = (x, found.lower, found.upper)
            }
        }
        let (lower, upper) = (best.lower, best.upper)
        try #require(lower.count > 10 && upper.count > 10,
                     "Best column x=\(best.x) has \(lower.count) lower and \(upper.count) upper series pixels")
        #expect(try #require(upper.max()) < #require(lower.min()))
        #expect(upper.count > lower.count)
    }
}

extension ChartDrawingTests {
    @Test func scatterUsesTheRangeOfItsSamplesOnBothAxes() async throws {
        let image = try await LayoutPicture("""
        root = Col([Chart("scatter", [{x: 1000, y: 998}, {x: 1001, y: 999}, {x: 1002, y: 1000}])], 0)
        """, width: 320, height: 320)
        let swatch = try LayoutPicture.swatch(IntelligentTheme.light.chart1.color)
        var dots = CGRect.null
        for y in stride(from: 0, to: 265, by: 2) {
            for x in stride(from: 0, to: 320, by: 2) {
                if try image.pixel(x, y) == swatch {
                    dots = dots.union(CGRect(x: x, y: y, width: 1, height: 1))
                }
            }
        }
        #expect(dots.width > 100 && dots.height > 100)
    }
}

extension ChartDrawingTests {
    @Test(arguments: ["bar", "line", "area", "scatter"])
    func showsGridAndAxisLabels(kind: String) async throws {
        let image = try await picture(kind)
        var across: [Int: Int] = [:], down: [Int: Int] = [:]
        var yLabels = 0, xLabels = 0
        for y in 0..<296 {
            for x in 0..<320 {
                let pixel = try image.pixel(x, y)
                // Coloured points and swatches do not count as grid or text.
                guard pixel.alphaComponent > 0.01,
                      abs(pixel.redComponent - pixel.blueComponent) < 0.08 else { continue }
                if (32..<300).contains(x) && (12..<246).contains(y) {
                    across[y, default: 0] += 1
                    down[x, default: 0] += 1
                }
                if pixel.alphaComponent > 0.3 {
                    if x < 16 && (20..<240).contains(y) { yLabels += 1 }
                    if y >= 276 && x > 32 { xLabels += 1 }
                }
            }
        }
        #expect(across.values.filter { $0 > 90 }.count >= 2)
        if kind == "scatter" {
            #expect(down.values.filter { $0 > 100 }.count >= 2)
        }
        #expect(yLabels > 5 && xLabels > 5)
    }
}

extension ChartDrawingTests {
    @Test func aCategoryWithNoValueKeysStillDraws() async throws {
        func plot(_ kind: String, _ label: String) async throws -> Data {
            let image = try await LayoutPicture("""
            root = Col([Chart("\(kind)", [{month:"\(label)"}])], 0)
            """, height: 320)
            #expect(try image.rect("/children/0").height == 296)
            return try image.renderedPixels()
        }
        let jan = try await plot("bar", "January")
        let feb = try await plot("bar", "February")
        #expect(jan != feb, "The category axis still labels the row")
        let scatter = try await plot("scatter", "January")
        let expected = try await LayoutPicture(#"root = Col([Caption("No data available")], 0)"#, height: 320,
            component: { element, _ in
                guard element.typeName == "Caption" else { return nil }
                return AnyView(Text(verbatim: "No data available").frame(maxWidth: .infinity).frame(height: 296))
            })
        #expect(try expected.renderedPixels() == scatter)
    }
}

extension ChartDrawingTests {
    @Test(arguments: [false, true])
    func barsStackWithinARowEvenIfItsCategoryRepeats(stacked: Bool) async throws {
        let row = #"{x:"A", a:2, b:1}"#
        func pixels(repeatRow: Bool) async throws -> LayoutPicture {
            let rows = [row, #"{x:"B", a:5, b:3}"#] + (repeatRow ? [row] : [])
            return try await LayoutPicture("""
            root = Col([Chart("bar", [\(rows.joined(separator: ","))], "x", ["a", "b"], null, \(stacked))], 0)
            """, width: 320, height: 320)
        }
        let original = try await pixels(repeatRow: false)
        let repeated = try await pixels(repeatRow: true)
        // Duplicate rectangles may antialias their edges twice. Compare the
        // solid interiors through category A, which should have the same heights.
        for x in [70, 104, 128] {
            for y in stride(from: 15, to: 252, by: 4) {
                let a = try original.pixel(x, y), b = try repeated.pixel(x, y)
                if a.alphaComponent < 0.01 || a.alphaComponent > 0.99 {
                    try #require(abs(a.alphaComponent - b.alphaComponent) < 0.01 &&
                                 abs(a.redComponent - b.redComponent) < 0.01 &&
                                 abs(a.greenComponent - b.greenComponent) < 0.01 &&
                                 abs(a.blueComponent - b.blueComponent) < 0.01,
                                 "Repeated category at (\(x), \(y))")
                }
            }
        }
    }
}

extension ChartDrawingTests {
    @Test func adjacentScatterPointsPaintAtOppositePlotCorners() async throws {
        let image = try await LayoutPicture("""
        root = Chart("scatter", [{x:1.0002000000000001e308, a:1.0002000000000001e308},
                                 {x:1.0002000000000003e308, a:1.0002000000000003e308}])
        """, width: 320, height: 320)
        let blue = IntelligentTheme.light.chart1.sRGB
        // The Y labels for these numbers take up a different amount of
        // space from the labels for ordinary values.
        let tops = try (0..<320).compactMap { x in
            try firstPaint(in: image, colour: blue, xs: x..<(x + 1), ys: 0..<290).map { (x: x, y: $0) }
        }
        let left = try #require(tops.first)
        let right = try #require(tops.last)
        #expect(right.x - left.x > 80)
        #expect(left.y - right.y > 100)
    }

    @Test(arguments: [
        ("5e-324", "5e-324"),
        ("1e-307", "1.0000000000000002e-307"),
        ("1e-9", "1.0000000000000003e-9"),
        ("1000000000", "999999999.9999999"),
    ])
    func recordedAdjacentScatterCoordinatesDraw(first: String, second: String) async throws {
        let image = try await LayoutPicture("""
        root = Col([Chart("scatter", [{x:\(first), a:\(first)}, {x:\(second), a:\(second)}]), Divider()], 0)
        """, width: 320, height: 320)
        #expect(try image.rect("/children/0").height > 296)
        #expect(try image.rect("/children/1").height == 1)
    }

    @Test func recordedExtremeChartHeightSettles() async throws {
        let image = try await LayoutPicture("""
        root = Col([Chart("bar", [{x:"A", a:2},{x:"B",a:3}], "x", ["a"], null, false, 1e308), Divider()], 0)
        """, width: 320, height: 320)
        #expect(try image.rect("/children/0").height == layoutPoints(1e308))
        #expect(try image.rect("/children/1").height == 1)
    }
}

extension ChartDrawingTests {
    @Test(arguments: ["bar", "line", "area", "scatter"])
    func defaultDrawingHeightLeavesLegendBelow(kind: String) async throws {
        func draw(height: String) async throws -> LayoutPicture {
            try await LayoutPicture("""
            root = Col([Chart("\(kind)", [{x:1, sales:3, cost:2},{x:2,sales:5,cost:4}], "x", ["sales", "cost"], null, null\(height)), Divider()], 0)
            """, width: 320, height: 340)
        }

        let auto = try await draw(height: ""), fixed = try await draw(height: ", 296")
        let autoChart = try auto.rect("/children/0"), fixedChart = try fixed.rect("/children/0")
        let swatch = IntelligentTheme.light.chart1.sRGB
        let autoLegend = try #require(try firstPaint(in: auto, colour: swatch, xs: 0..<9, ys: 260..<330))
        let fixedLegend = try #require(try firstPaint(in: fixed, colour: swatch, xs: 0..<9, ys: 260..<330))
        #expect(autoLegend >= 296 && CGFloat(autoLegend) < autoChart.maxY)
        #expect(fixedLegend < 296 && CGFloat(fixedLegend) < fixedChart.maxY)
        #expect(autoChart.height > 296 && fixedChart.height == 296)
        #expect(try auto.rect("/children/1").minY == autoChart.maxY)
        #expect(try fixed.rect("/children/1").minY == fixedChart.maxY)
        #expect(autoLegend - fixedLegend == Int((autoChart.height - fixedChart.height).rounded()))

        func bottomAxisInk(_ picture: LayoutPicture, before legend: Int) throws -> Int? {
            // Chart labels are grey. The legend is concentrated at the left,
            // so the lower chart labels can be measured independently.
            for y in (250..<legend).reversed() {
                for x in 90..<318 {
                    let pixel = try picture.pixel(x, y)
                    if pixel.alphaComponent > 0.3 && abs(pixel.redComponent - pixel.blueComponent) < 0.08 {
                        return y
                    }
                }
            }
            return nil
        }
        let autoAxis = try #require(try bottomAxisInk(auto, before: autoLegend))
        let fixedAxis = try #require(try bottomAxisInk(fixed, before: fixedLegend))
        #expect(autoAxis < 296 && fixedAxis < fixedLegend)
        #expect(abs((autoAxis - fixedAxis) - (autoLegend - fixedLegend)) <= 1)
    }

    @Test func repeatedBarCategoryUsesSeriesPainterOrder() async throws {
        let image = try await LayoutPicture("""
        root = Chart("bar", [{x:"A", a:1, b:2}, {x:"A", a:3, b:0}], "x", ["a", "b"], null, true)
        """, width: 320, height: 320)
        let blue = IntelligentTheme.light.chart1.sRGB
        let green = IntelligentTheme.light.chart2.sRGB
        // The pinned BarSeries paints a [0,3] blue bar before the green
        // [1,3] band at the same category. Their interiors overlap here.
        let xs = 140..<200, ys = 70..<145
        #expect(try firstPaint(in: image, colour: green, xs: xs, ys: ys) != nil)
        #expect(try firstPaint(in: image, colour: blue, xs: xs, ys: ys) == nil)
    }

    @Test func nonNumericScatterValuePaintsItsZeroMark() async throws {
        let image = try await LayoutPicture("""
        root = Chart("scatter", [{x:1, a:"bad"}, {x:2, a:5}], "x", ["a"])
        """, width: 320, height: 320)
        let blue = IntelligentTheme.light.chart1.sRGB
        #expect(try firstPaint(in: image, colour: blue, xs: 0..<150, ys: 160..<285) != nil)
    }

    @Test(arguments: ["line", "area"])
    func paintsTheMonotoneStrokeBetweenSamples(kind: String) async throws {
        let image = try await LayoutPicture("""
        root = Chart("\(kind)", [{x:"A", a:4}, {x:"B"}, {x:"C", a:4}], "x", ["a"])
        """, width: 320, height: 320)
        let blue = IntelligentTheme.light.chart1.sRGB
        var bounds = CGRect.null
        for y in 0..<284 {
            for x in 0..<320 where matches(try image.pixel(x, y), blue, alpha: 0.85) {
                bounds = bounds.union(CGRect(x: x, y: y, width: 1, height: 1))
            }
        }
        try #require(bounds.width > 100 && bounds.height > 100)
        let midway = Int((bounds.minX + bounds.width / 4).rounded())
        let y = try #require(try firstPaint(in: image, colour: blue, xs: midway..<(midway + 1), ys: 0..<284))
        let fraction = (bounds.maxY - CGFloat(y)) / bounds.height
        // The pinned controls put the midpoint at 1.25 out of a 4-unit range.
        #expect(abs(fraction - 1.25 / 4) < 0.055)
    }
}

extension ChartDrawingTests {
    @Test func paintsTheStackedBandBetweenEqualEndpoints() async throws {
        let image = try await LayoutPicture("""
        root = Chart("area", [{x:0,b:-2}, {x:1,a:1}, {x:2,a:3}, {x:3,a:4,b:-2}],
            "x", ["a", "b"], null, true)
        """, width: 320, height: 320)
        let blue = IntelligentTheme.light.chart1.sRGB
        let green = IntelligentTheme.light.chart2.sRGB
        for x in 155..<180 {
            let xs = x..<(x + 1)
            guard let upper = try firstPaint(in: image, colour: green, xs: xs, ys: 50..<230),
                  let lower = try firstPaint(in: image, colour: blue, xs: xs, ys: 50..<230),
                  upper + 6 < lower else { continue }
            let pixel = try image.pixel(x, (upper + lower) / 2)
            if matches(pixel, green, alpha: 0.25) && pixel.alphaComponent < 0.65 { return }
        }
        Issue.record("Missing green fill between the two stacked Bézier boundaries")
    }
}
