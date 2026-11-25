import CoreGraphics
@testable import OpenUIIntelligentUI
import Testing

struct TableGeometryTests {
    @Test func tableTracksWrapFlexibleCellsBeforeOverflowing() {
        let minimums: [CGFloat] = [64, 40, 32]
        #expect(TableTracks(minimums: minimums, available: 228).widths == [76, 76, 76])
        #expect(TableTracks(minimums: minimums, available: 120).widths == [64, 40, 40])
        #expect(TableTracks(minimums: minimums, available: 300).width == 300)
        #expect(TableTracks(minimums: minimums, available: nil).width == 136)
    }

    @Test func viewportRowsKeepAbsoluteOffsetsAndExcludeTouchingEdges() {
        let rows = ViewportRows(heights: [20, 40, 10, 30])
        #expect(rows.height == 100)
        #expect(rows.visible(in: 20..<70) == 1..<3)
        #expect(rows.visible(in: 19..<21) == 0..<2)
        #expect(rows.visible(in: 100..<120).isEmpty)
        #expect(rows.visible(in: -30..<0).isEmpty)
        #expect(rows.frame(of: 2) == 60..<70)
        #expect(rows.frame(of: 4) == nil)
        #expect(ViewportRows(heights: []).visible(in: 0..<100).isEmpty)
    }
}
