// Feed ordered props through the same Element decoder used for evaluated trees.
import Darwin
import Foundation
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import Testing

@Suite struct ChartDataTests {
    private func element(_ props: String) throws -> Element {
        try Element(json: JSONValue.parse("""
        {"type":"element","typeName":"Chart","props":{\(props)},"partial":false,"hasDynamicProps":false}
        """))
    }

    private func chart(_ props: String, _ kind: CartesianChartKind = .bar) throws -> CartesianChartData? {
        CartesianChartData(try element(props), kind: kind)
    }

    private func value(_ key: String, in row: CartesianChartData.Row?, of data: CartesianChartData) -> Double? {
        guard let id = data.seriesByKey[.init(key)]?.id else { return nil }
        return row?.values[id]
    }

    @Test func firstRowDeterminesCategoryAndSeries() throws {
        for (props, x, series) in [
            (#""data":[false,null,{"sales":3,"month":"Jan","note":"ok","cost":2},{"later":6}]"#, "month", ["sales", "cost"]),
            (#""data":[{"time":1,"second":8,"first":3}],"x":"","series":[]"#, "time", ["second", "first"]),
            (#""data":[{"sales":3,"name":"a","cost":2}],"x":"sales""#, "sales", ["cost"]),
            (#""data":[{"month":"Jan","cost":2}],"x":"elsewhere","series":["cost","missing"]"#, "elsewhere", ["cost", "missing"]),
            (#""data":[{"name":"Jan"},{"later":5}]"#, "name", []),
            (#""data":[{"10":3,"2":4,"01":8,"4294967295":2}]"#, "2", ["10", "01", "4294967295"]),
        ] {
            let data = try #require(try chart(props))
            #expect(data.category == x)
            #expect(data.legendKeys.map(\.text) == series)
        }
    }

    @Test func narrowFillsMissingAndNonNumericCellsWithZero() throws {
        let props = #""data":[null,{"month":"Jan","a":2,"b":3},{"month":"Feb","a":"17"},{"b":"no"}],"series":["b","a"]"#
        for kind in [CartesianChartKind.bar, .line, .area] {
            let data = try #require(try chart(props, kind))
            #expect(data.legendKeys.map(\.text) == ["b", "a"])
            #expect(data.rows.map(\.label) == ["Jan", "Feb", ""])
            #expect(data.rows.map { (value("a", in: $0, of: data) ?? 0) } == [2, 0, 0])
            #expect(data.rows.map { (value("b", in: $0, of: data) ?? 0) } == [3, 0, 0])
        }
    }

    @Test func scatterConvertsBothCoordinatesAndDefaultsMissingValuesToZero() throws {
        let data = try #require(try chart(#""data":[{"x":"2.5","y":" 0x10 "},{"x":10,"y":"2e1"},{"y":"bad"},{}],"x":"x","series":["y"]"#, .scatter))
        #expect(data.rows.map(\.x) == [2.5, 10, 0, 0])
        #expect(data.rows.prefix(2).map { (value("y", in: $0, of: data) ?? 0) } == [16, 20])
        #expect(value("y", in: data.rows.dropFirst(2).first, of: data) == 0)
        #expect(value("y", in: data.rows.last, of: data) == nil)
    }

    @Test func longRadixCoordinatesRoundOnceAsJavaScriptNumbers() throws {
        let props = #""data":[{"x":"0x20000000000001001","y":"0x20000000000001000"}],"x":"x","series":["y"]"#
        let plot = try #require(try chart(props, .scatter))
        #expect(plot.rows.first?.x == Double(0x20000000000002000p0))
        #expect(value("y", in: plot.rows.first, of: plot) == Double(0x20000000000000000p0))
    }

    @Test func noRowsOrNoCategoryYieldsNoChart() throws {
        for props in [#""data":[],"x":"x""#, #""data":[false,null]"#, #""data":[{},{"late":2}]"#,
                      #""data":[{"":"text","n":2}]"#] {
            #expect(try chart(props) == nil)
        }
        #expect(try chart(#""data":[{}],"x":"missing""#) != nil)
    }
}

extension ChartDataTests {
    @Test func scatterHoverPicksOnlyTheNearestVisiblePointInsideThirtyPoints() throws {
        let plot = try #require(try chart(#""data":[{"x":3,"a":2,"b":19},{"x":28,"a":9,"b":1}]"#, .scatter))
        func hit(_ x: Double, _ y: Double, _ keys: [String]) -> String? {
            let visible = plot.visibility(hiding: Set(plot.legendKeys).subtracting(keys.map(CartesianChartData.Name.init)))
            let hit = plot.scatterHit(at: CGPoint(x: x, y: y), visible: visible) { CGPoint(x: $0, y: $1) }
            return hit.flatMap { plot.name(forSeriesID: $0.series.id) }
        }
        #expect(hit(3, 18, ["a", "b"]) == "b")
        #expect(hit(3, 18, ["a"]) == "a")
        #expect(hit(58, 1, ["b"]) == "b")
        #expect(hit(59, 1, ["a", "b"]) == nil)
    }
}

extension ChartDataTests {
    @Test func scatterPreservesDatasetOrderAndRepeatedKeysWhileNarrowUsesObjectKeys() throws {
        let props = #""data":[{"x":"Jan","a":2}],"x":"x","series":["a","x","a","absent"]"#
        let scatter = try #require(try chart(props, .scatter))
        #expect(scatter.legendKeys.map(\.text) == ["a", "x", "a", "absent"])
        let bar = try #require(try chart(props))
        #expect(bar.legendKeys.map(\.text) == ["a", "absent"])
        #expect(bar.rows.first?.label == "0")
    }

    @Test func upstreamRowFilterKeepsArrayObjects() throws {
        let plot = try #require(try chart(#""data":[null,4,["Jan",3,2]]"#))
        #expect(plot.category == "0")
        #expect(plot.legendKeys.map(\.text) == ["1", "2"])
        #expect(plot.rows.first?.label == "Jan")
    }

    @Test @MainActor func sparseGeometryAndHitTestingScaleWithPresentValues() throws {
        // At this shape even one bit per row/series pair takes 65,536² bits,
        // 512 MiB. Bound the bytes the chart itself holds instead of the
        // process's RSS, which other tests running in parallel move by
        // hundreds of MiB in either direction. Data, visibility and geometry
        // measured at most about 45 MiB of heap per kind with
        // malloc_zone_statistics, and up to 50 MiB for area depending on how
        // malloc rounds the array capacities; this sum agreed with malloc to
        // within 0.1 MiB. 64 MiB leaves growth slack and is still 8× below
        // the dense size this must catch.
        let count = Self.sparseCount
        for kind in [CartesianChartKind.bar, .line, .area, .scatter] {
            let (_, data, visible, geometry) = try Self.sparseChart(kind)
            #expect(data.rows.count == count)
            #expect(visible.last.count == count)
            #expect(data.rows.reduce(0) { $0 + $1.cells.count } == 1)
            #expect(geometry.bars.count + geometry.dots.count <= count + 1)
            #expect(geometry.curves.values.reduce(0) { $0 + $1.count } <= count)
            try Self.expectSparseQueriesVisitPresentValues(kind, data, visible)
            let held = ChartHeldBytes.of(data, visible, geometry)
            #expect(held < 64 * 1024 * 1024, "The \(kind) chart holds \(held) bytes")
        }
    }

    // The process's peak RSS can only be read for one test when nothing else
    // runs in the process, so each kind draws in an exit-test child. Run
    // alone, the four kinds in sequence measured a fixed 252 MiB; in its own
    // child each kind measured 127–145 MiB, alone or beside the full parallel
    // run. A dense row/series bitmap alone would add 512 MiB.
    @Test(.serialized, arguments: [CartesianChartKind.bar, .line, .area, .scatter])
    func sparseChartDrawsInBoundedPeakMemory(_ kind: CartesianChartKind) async throws {
        try await #require(processExitsWith: .success, "Drawing the sparse \(kind) chart in a child process") {
            [kind = kind.rawValue as String] in
            try await MainActor.run {
                try ChartDataTests.drawSparseChartInBoundedPeakMemory(#require(CartesianChartKind(rawValue: kind)))
            }
        }
    }

    @MainActor private static func drawSparseChartInBoundedPeakMemory(_ kind: CartesianChartKind) throws {
        let initialPeak = try #require(peakRSS())
        let (chart, data, visible, _) = try sparseChart(kind)
        try expectSparseQueriesVisitPresentValues(kind, data, visible)
        let view = IntelligentChart(element: chart).frame(width: 320, height: 320)
        let image = try #require(ImageRenderer(content: view).cgImage)
        #expect(image.width == 320)
        #expect(image.height == 320)
        let peak = try #require(peakRSS())
        try #require(peak - initialPeak < 384 * 1024 * 1024)
    }

    private static func expectSparseQueriesVisitPresentValues(_ kind: CartesianChartKind, _ data: CartesianChartData,
                                                              _ visible: CartesianChartData.Visibility) throws {
        if kind == .line {
            let tooltip = try #require(data.categoryTooltip(index: 0, visible: visible))
            #expect(tooltip.split(separator: "\n").count == sparseCount + 1)
        }
        if kind == .scatter {
            var visits = 0
            let hit = data.scatterHit(at: .zero, visible: visible) { x, y in
                visits += 1
                return CGPoint(x: x, y: y)
            }
            #expect(hit?.series.id == 1)
            #expect(visits == sparseCount + 1)
        }
    }

    private static let sparseCount = 65_536

    private static func sparseElement(_ kind: CartesianChartKind) throws -> Element {
        let keys = (0..<sparseCount).map { "\"k\($0)\"" }.joined(separator: ",")
        let rows = (#"{"x":0,"k0":1},"# + (1..<sparseCount).map { "{\"x\":\($0)}" }.joined(separator: ","))
        return try ChartDataTests().element("""
        "type":"\(kind.rawValue)","data":[\(rows)],"x":"x","series":[\(keys)],"stacked":true
        """)
    }

    private static func sparseChart(_ kind: CartesianChartKind) throws
        -> (Element, CartesianChartData, CartesianChartData.Visibility, CartesianChartGeometry) {
        let chart = try sparseElement(kind)
        let data = try #require(CartesianChartData(chart, kind: kind))
        let visible = data.visibility(hiding: [])
        return (chart, data, visible, CartesianChartGeometry(data: data, visible: visible))
    }

    private static func peakRSS() -> Int? {
        var usage = rusage()
        return getrusage(RUSAGE_SELF, &usage) == 0 ? usage.ru_maxrss : nil
    }

    @Test func stackedAreaCarriesTheSignedTotalThroughMissingCells() throws {
        let props = #""data":[{"x":"A","a":3,"c":-4},{"x":"B","a":1,"b":2}],"series":["a","b","c"],"stacked":true"#
        let area = try #require(try chart(props, .area))
        let visible = area.visibility(hiding: [])
        let areaGeometry = CartesianChartGeometry(data: area, visible: visible)
        #expect(visible.yMagnitude.domain == [-1, 3])
        let middle = try #require(areaGeometry.curves[1]?.first)
        #expect(middle.stroke.left.y == 3)
        #expect(middle.stroke.right.y == 3)
        let last = try #require(areaGeometry.curves[2]?.first)
        #expect(last.stroke.left.y == -1)
        #expect(last.stroke.right.y == 3)
        #expect(last.baseline?.left.y == 3)
        #expect(last.baseline?.right.y == 3)

        let bar = try #require(try chart(props, .bar))
        let barVisible = bar.visibility(hiding: [])
        #expect(barVisible.yMagnitude.domain == [-4, 3])
        let bars = CartesianChartGeometry(data: bar, visible: barVisible).bars
        #expect(bars.count == 4)
        #expect(bars.first { $0.colour == 2 }?.end == 0)
    }

    @Test func numericSeriesWritesDoNotCreateAPrototypeProperty() throws {
        let props = #""data":[{"x":"A","a":2}],"series":["__proto__","a"]"#
        let data = try #require(try chart(props))
        #expect(data.legendKeys.map(\.text) == ["a"])
        #expect(CartesianChartGeometry(data: data, visible: data.visibility(hiding: [])).bars.map(\.end) == [2])
    }

    @Test func chartKeysAndLabelsUseJavaScriptStringIdentity() throws {
        let props = #""data":[{"x":"é","é":2,"e\u0301":7},{"x":"e\u0301","é":4}],"series":["é","e\u0301"]"#
        let data = try #require(try chart(props))
        #expect(data.legendKeys.map { Array($0.text.utf8) } == [Array("é".utf8), Array("e\u{301}".utf8)])
        #expect(data.categories.map { Array($0.utf8) } == [Array("é".utf8), Array("e\u{301}".utf8)])
        let visible = data.visibility(hiding: [.init("é")])
        #expect(data.visibility(hiding: []).last.count == 2)
        #expect(visible.last.count == 1)
        #expect(CartesianChartGeometry(data: data, visible: visible).bars.map(\.end) == [7])
    }

    @Test func categoricalTooltipFollowsTheUpstreamRowOrdinal() throws {
        let props = #""data":[{"x":"A","a":2},{"x":"A","b":9},{"x":"B","a":3}],"series":["a","b"]"#
        let data = try #require(try chart(props, .line))
        let visible = data.visibility(hiding: [])
        #expect(data.row(forCategory: 1)?.label == "B")
        #expect(data.categoryTooltip(index: 1, visible: visible) == "A\na: 0\nb: 9")
    }

    @Test func missingValuesDrawZeroEdgesAndScatterPoints() throws {
        let props = #""data":[{"x":0,"a":2},{"x":1},{"x":2,"a":3}],"series":["a","b"]"#
        for kind in [CartesianChartKind.line, .area] {
            let data = try #require(try chart(props, kind))
            let visible = data.visibility(hiding: [])
            let curves = CartesianChartGeometry(data: data, visible: visible).curves
            #expect(curves[0]?.map { $0.stroke.right.y } == [0, 3])
            #expect(curves[1]?.map { $0.stroke.right.y } == [0, 0])
        }
        let scatter = try #require(try chart(props, .scatter))
        let visible = scatter.visibility(hiding: [])
        let dots = CartesianChartGeometry(data: scatter, visible: visible).dots
        #expect(dots.filter { $0.point.y == 0 }.map { $0.point.x } == [0, 1, 2])
        let hit = scatter.scatterHit(at: CGPoint(x: 1, y: 0), visible: visible) { CGPoint(x: $0, y: $1) }
        #expect(hit?.row.id == 1)
        #expect(hit?.series.id == 0, "The first dataset wins a tie even when a later one paints the dot")
    }
}

extension ChartDataTests {
    @Test func nonNumericScatterValueIsZeroPerTicket() throws {
        let data = try #require(try chart(#""data":[{"x":1,"a":"bad"}],"x":"x","series":["a"]"#, .scatter))
        let visible = data.visibility(hiding: [])
        let dot = try #require(CartesianChartGeometry(data: data, visible: visible).dots.first)
        #expect(dot.point == ScatterCoordinate(x: 1, y: 0))
        let hit = try #require(data.scatterHit(at: CGPoint(x: 1, y: 0), visible: visible) { CGPoint(x: $0, y: $1) })
        #expect(hit.row.id == dot.rowID && hit.series.id == dot.series.id)
        #expect(data.scatterTooltip(row: hit.row, series: hit.series) == "a\nX: 1\nY: 0")
    }

    @Test(arguments: [("Jan", "Feb"), ("Infinity", "NaN")])
    func unavailableScatterXUsesPinnedFallbackOnBothAxes(first: String, second: String) throws {
        let data = try #require(try chart("""
        "data":[{"x":"\(first)","a":3},{"x":"\(second)","a":5}],"x":"x","series":["a"]
        """, .scatter))
        let visible = data.visibility(hiding: [])
        #expect(data.xMagnitude.domain == [0, 100])
        #expect(visible.yMagnitude.domain == [0, 100])
        #expect(CartesianChartGeometry(data: data, visible: visible).dots.isEmpty)
    }

    @Test(arguments: ["-Infinity", "bad", "NaN"])
    func mixedScatterXUsesPinnedExtent(convertedFrom text: String) throws {
        let data = try #require(try chart("""
        "data":[{"x":"\(text)","a":2},{"x":50,"a":3},{"x":60,"a":4}],"x":"x","series":["a"]
        """, .scatter))
        let visible = data.visibility(hiding: [])
        // Pinned wrapper + scatter orchestrator. NaN cannot win the minimum
        // comparison, but negative infinity wins it and resets both extents.
        let fallback = text == "-Infinity"
        #expect(data.xMagnitude.domain.map(data.xMagnitude.transform.value(at:)) == (fallback ? [0, 100] : [50, 60]))
        #expect(visible.yMagnitude.domain.map(visible.yMagnitude.transform.value(at:)) == (fallback ? [0, 100] : [2, 4]))
        #expect(CartesianChartGeometry(data: data, visible: visible).dots.map(\.rowID) == [1, 2])
        var projected: [Double] = []
        let hit = data.scatterHit(at: CGPoint(x: 50, y: 3), visible: visible) { x, y in
            projected.append(x)
            return CGPoint(x: x, y: y)
        }
        #expect(projected == [50, 60])
        #expect(hit?.row.id == 1)
    }

    @Test func nearestHitDistinguishesAdjacentScatterSamplesOnBothAxes() throws {
        let data = try #require(try chart("""
        "data":[{"x":1.0002000000000001e308,"a":1.0002000000000001e308},
                {"x":1.0002000000000003e308,"a":1.0002000000000003e308}],"x":"x","series":["a"]
        """, .scatter))
        let visible = data.visibility(hiding: [])
        let x = data.xMagnitude.domain, y = visible.yMagnitude.domain
        let xMin = try #require(x.first), xMax = try #require(x.last)
        let yMin = try #require(y.first), yMax = try #require(y.last)
        #expect(x.map(data.xMagnitude.transform.value(at:)) == [1.0002000000000001e308, 1.0002000000000003e308])
        #expect(y.map(visible.yMagnitude.transform.value(at:)) == [1.0002000000000001e308, 1.0002000000000003e308])
        // The pinned d3 scales map the two samples to these opposite corners.
        for (row, corner) in [(0, CGPoint(x: 0, y: 200)), (1, CGPoint(x: 200, y: 0))] {
            let hit = data.scatterHit(at: corner, visible: visible) { x, y in
                let point = data.scatterCoordinates(x: x, y: y)
                return CGPoint(x: 200 * ((point.x - xMin) / (xMax - xMin)),
                               y: 200 * (1 - (point.y - yMin) / (yMax - yMin)))
            }
            #expect(hit?.row.id == row)
        }
    }

    @Test func wideMixedSignScatterScaleStaysFinite() {
        let scale = CartesianChartData.Magnitude([-1e308, 1e308])
        let transform = scale.transform
        #expect(scale.domain == [-1, 1])
        #expect(transform.position(0) == 0)
        #expect(scale.domain.map(transform.value(at:)) == [-1e308, 1e308])
    }

    @Test(arguments: [1, 65_536])
    func missingSeriesNameWorkScalesWithInputBytes(keyBytes: Int) throws {
        let missing = String(repeating: "m", count: keyBytes)
        let count = 2_048
        let rows = (1...count).map { #"{"x":\#($0),"y":1}"# }.joined(separator: ",")
        for keys in [[missing, "y"], ["y", missing], [missing, "y", missing]] {
            let series = keys.map { "\"\($0)\"" }.joined(separator: ",")
            let data = try #require(try chart("""
            "data":[\(rows)],"x":"x","series":[\(series)]
            """, .scatter))
            let visible = data.visibility(hiding: [])
            let missingID = try #require(data.seriesByKey[.init(missing)]?.id)
            let knownID = try #require(data.seriesByKey[.init("y")]?.id)
            var implicitProbes = 0
            for row in data.rows {
                // These lookup tables take integer IDs independently of the
                // absent key's length and of its position in dataset order.
                let values: [Int: Double] = row.values
                #expect(values.count == 1 && values[knownID] == 1)
                if visible.firstMissing(in: row)?.id == missingID { implicitProbes += 1 }
                if visible.lastMissing(in: row)?.id == missingID { implicitProbes += 1 }
            }
            #expect(implicitProbes == count * 2)

            let dots = CartesianChartGeometry(data: data, visible: visible).dots
            #expect(dots.count == count * 2)
            #expect(dots.filter { $0.series.id == missingID }.count == count)
            var visits = 0
            let hit = data.scatterHit(at: CGPoint(x: 1, y: 0), visible: visible) { x, y in
                visits += 1
                return CGPoint(x: x, y: y)
            }
            #expect(visits == count * 2)
            #expect(hit?.series.id == missingID)
        }
    }

    // Recorded by running the pinned seriesGeometry.ts builders with d3-shape
    // 3.2.0 and capturing the path context before SVG rounds coordinates.
    private func expectControls(_ cube: CartesianChartGeometry.Cubic, _ coordinates: [Double]) {
        let actual = [cube.left.x, cube.left.y, cube.control1.x, cube.control1.y,
                      cube.control2.x, cube.control2.y, cube.right.x, cube.right.y]
        #expect(actual.count == coordinates.count)
        for (value, expected) in zip(actual, coordinates) {
            #expect(abs(value - expected) < 1e-10)
        }
    }

    @Test func recordedSalesAndCostUseUpstreamsTwoPointChord() throws {
        let data = try #require(try chart(#""data":[{"month":"Jan","sales":3,"cost":2},{"month":"Feb","sales":5,"cost":4}]"#, .line))
        let geometry = CartesianChartGeometry(data: data, visible: data.visibility(hiding: []))
        for (series, start) in [(0, 3.0), (1, 2.0)] {
            let edge = try #require(geometry.curves[series]?.first)
            // Upstream emits L for two points. Its cubic equivalent has
            // controls one and two thirds of the way along the chord.
            expectControls(edge.stroke, [0, start, 1.0 / 3, start + 2.0 / 3, 2.0 / 3, start + 4.0 / 3, 1, start + 2])
        }
    }

    @Test(arguments: [CartesianChartKind.line, .area])
    func missingSampleFollowsUpstreamMonotoneCurve(kind: CartesianChartKind) throws {
        let data = try #require(try chart(#""data":[{"x":0,"a":4},{"x":1},{"x":2,"a":4}]"#, kind))
        let curves = CartesianChartGeometry(data: data, visible: data.visibility(hiding: [])).curves
        let edges = try #require(curves[0])
        #expect(edges.count == 2)
        let first = try #require(edges.first), last = try #require(edges.last)
        expectControls(first.stroke, [0,4, 1.0 / 3,2, 2.0 / 3,0, 1,0])
        expectControls(last.stroke, [1,0, 4.0 / 3,0, 5.0 / 3,2, 2,4])
        if kind == .area {
            expectControls(try #require(first.baseline), [0,0, 1.0 / 3,0, 2.0 / 3,0, 1,0])
            expectControls(try #require(last.baseline), [1,0, 4.0 / 3,0, 5.0 / 3,0, 2,0])
        }
    }

    @Test func stackedAreaKeepsBandsWithEqualEndpointsAndDifferentCurves() throws {
        let props = #""data":[{"x":0,"b":2},{"x":1,"a":1},{"x":2,"a":3},{"x":3,"a":4,"b":2}],"series":["a","b"],"stacked":true"#
        let data = try #require(try chart(props, .area))
        let geometry = CartesianChartGeometry(data: data, visible: data.visibility(hiding: []))
        let edges = try #require(geometry.curves[1])
        let band = try #require(edges.dropFirst().first)
        expectControls(band.stroke, [1,1, 4.0 / 3,1, 5.0 / 3,13.0 / 6, 2,3])
        expectControls(try #require(band.baseline), [1,1, 4.0 / 3,1.5, 5.0 / 3,2.5, 2,3])
    }

    @Test func stackedTangentFindsTheLastDistinctSampleInARepeatedCategory() throws {
        let props = #""data":[{"x":"A"},{"x":"A","b":2},{"x":"A"},{"x":"A"},{"x":"B","a":2},{"x":"C","a":4}],"series":["a","b"],"stacked":true"#
        let data = try #require(try chart(props, .area))
        let geometry = CartesianChartGeometry(data: data, visible: data.visibility(hiding: []))
        let edges = try #require(geometry.curves[1])
        let band = try #require(edges.first { $0.stroke.left.x == 0 && $0.stroke.right.x == 1 })
        expectControls(band.stroke, [0,0, 1.0 / 3,0, 2.0 / 3,4.0 / 3, 1,2])
        expectControls(try #require(band.baseline), [0,0, 1.0 / 3,2.0 / 3, 2.0 / 3,4.0 / 3, 1,2])
    }

    @Test func monotoneTangentsFollowRepeatedAndReversedCategories() throws {
        let data = try #require(try chart(#""data":[{"x":"A","a":1},{"x":"A","a":1},{"x":"A","a":2},{"x":"B","a":4},{"x":"B","a":4},{"x":"C","a":6},{"x":"A","a":3}]"#, .line))
        let geometry = CartesianChartGeometry(data: data, visible: data.visibility(hiding: []))
        let edges = try #require(geometry.curves[0])
        let expected: [[Double]] = [
            [0,1, 0,1, 0,2, 0,2],
            [0,2, 1.0 / 3,10.0 / 3, 2.0 / 3,10.0 / 3, 1,4],
            [1,4, 4.0 / 3,14.0 / 3, 5.0 / 3,31.0 / 6, 2,6],
            [2,6, 4.0 / 3,13.0 / 3, 2.0 / 3,11.0 / 3, 0,3],
        ]
        #expect(edges.count == expected.count)
        for (edge, controls) in zip(edges, expected) { expectControls(edge.stroke, controls) }
    }
}
