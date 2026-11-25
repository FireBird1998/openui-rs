import Foundation
@testable import OpenUIIntelligentUI
import Testing

@Suite struct PolarChartGeometryTests {
    @Test func pieHitsFollowClockwiseSortedWedgesAndDoNotExtendOutsidePlot() throws {
        let geometry = plot(.pie, [3, 1, 0, -4])
        #expect(geometry.sectors.map(\.slice.id) == [0, 1])
        #expect(geometry.hit(at: CGPoint(x: 165, y: 100), size: 200)?.slice.id == 0)
        #expect(geometry.hit(at: CGPoint(x: 52, y: 52), size: 200)?.slice.id == 1)
        #expect(geometry.hit(at: CGPoint(x: 192, y: 100), size: 200) == nil)
        #expect(geometry.hit(at: CGPoint(x: 100, y: 12), size: 200)?.slice.id == 0)
        let nearlyFull = try #require(plot(.pie, [1e11, 1]).sectors.first)
        #expect(nearlyFull.hoverOffset == .zero)
    }

    @Test func donutHitsBothRingsAndRejectsTheHoleAndGaps() {
        let geometry = plot(.donut, [3, 1])
        for radius in [35.0, 85.0] {
            #expect(geometry.hit(at: CGPoint(x: 100 + radius, y: 100), size: 200)?.slice.id == 0)
        }
        #expect(geometry.hit(at: CGPoint(x: 110, y: 100), size: 200) == nil)
        #expect(geometry.hit(at: CGPoint(x: 100, y: 30), size: 200) == nil)
        #expect(geometry.hit(at: CGPoint(x: 100, y: 1), size: 200) == nil)
        let seamInSingleDonut = plot(.donut, [9])
        #expect(seamInSingleDonut.hit(at: CGPoint(x: 100, y: 30), size: 200)?.slice.id == 0)
    }

    @Test func sectorsUsePositiveValuesAndRetainSpaceForDonutRowsWithoutPaint() throws {
        let geometry = plot(.donut, [5, 2, 0, -1, -.infinity])
        let last = try #require(geometry.sectors.last)
        #expect(geometry.sectors.count == 2)
        #expect(abs(last.end - (360 - 3 * 0.5) * .pi / 180) < 1e-12)
        #expect(plot(.donut, [0, -1]).sectors.isEmpty)
        #expect(plot(.pie, [.infinity, 3, 0]).sectors.isEmpty)
        #expect(plot(.donut, [.infinity, 3, -.infinity]).sectors.isEmpty)
        #expect(plot(.pie, [3, -.infinity]).sectors.count == 1)
    }

    @Test func slenderDonutWedgeReachesTheInnerTrackBeforeItsParallelGapWouldMeet() throws {
        let geometry = plot(.donut, [250, 0.5])
        let last = try #require(geometry.sectors.last)
        let middle = (last.start + last.end) / 2
        let point = CGPoint(x: 100 + 25 * sin(middle), y: 100 - 25 * cos(middle))
        #expect(geometry.hit(at: point, size: 200)?.slice.id == 1)
        let outside = CGPoint(x: point.x + 1, y: point.y)
        #expect(geometry.hit(at: outside, size: 200)?.slice.id != 1)
    }

    @Test func finiteTinyValuesSplitTheTurnWhenTheAngleMultiplierOverflows() throws {
        let geometry = plot(.pie, [1e-308, 1e-308])
        #expect(geometry.sectors.count == 2)
        let first = try #require(geometry.sectors.first)
        #expect(abs(first.end - .pi) < 1e-12)
    }

    private func plot(_ kind: PolarChartKind, _ values: [Double]) -> PolarChartGeometry {
        let slices = values.enumerated().map { index, value in
            PolarChartData.Slice(id: index, label: .init(String(index)), value: value, colourIndex: index % 6)
        }
        return PolarChartGeometry(kind: kind, visible: slices)
    }
}
