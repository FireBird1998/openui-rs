@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

@Suite @MainActor struct GridDrawingTests {
    private func grid(_ props: String, width: CGFloat) async throws -> LayoutPicture {
        try await LayoutPicture("""
        root = Col([Grid([a, b, a]\(props))], 0)
        a = Box([], null, null, null, "red", null, null, null, 10)
        b = Box([], null, null, null, "blue", null, null, null, 20)
        """, width: width)
    }

    @Test func defaultGridUsesEqualTracksAndStartsTheNextRowBelowTheTallestCell() async throws {
        let picture = try await grid("", width: 200)
        let cell = "/children/0/children/"
        #expect(try picture.rect(cell + "0") == CGRect(x: 0, y: 0, width: 94, height: 10))
        #expect(try picture.rect(cell + "1") == CGRect(x: 106, y: 0, width: 94, height: 20))
        #expect(try picture.rect(cell + "2") == CGRect(x: 0, y: 32, width: 94, height: 10))
    }

    @Test func gridRoundsClampsAndDropsColumnsAtTheMinimumIncludingGap() async throws {
        for (props, width, track) in [
            (", 2.5, null, 1", 308, 100), (", 30, null, 0", 240, 20),
            (", -2, null, 0", 240, 240), (", 4, 100, 1", 308, 100),
            (", 4, 100, 1", 307, 151.5), (", 4, 100, 1", 75, 75),
            (", 2, 10, 0", 200, 100), (", 2, null, 1e308", 200, 100),
            (", 2, 50, 1e308", 200, 200),
        ] as [(String, CGFloat, CGFloat)] {
            let picture = try await grid(props, width: width)
            #expect(try picture.rect("/children/0/children/0").width == track)
        }
    }

    @Test func centeredGridMeasuresOneResponsiveRepetitionOrAllFixedColumns() async throws {
        for (minimum, expectedWidth, rows) in [("60", 60, [0, 14, 28]), ("0", 188, [0, 0, 0])] {
            let picture = try await LayoutPicture("""
            root = Col([Grid([a,a,a], 3, \(minimum), 1)], 0, "center")
            a = Box([], null, null, null, "red", null, null, 60, 10)
            """)
            let grid = try picture.rect("/children/0")
            #expect(grid.width == CGFloat(expectedWidth))
            #expect(grid.midX == 120)
            for (index, y) in rows.enumerated() {
                #expect(try picture.rect("/children/0/children/\(index)").minY == CGFloat(y))
            }
        }
    }

    @Test func negativeMinimumStillUsesAutoFillWhenGapsLimitTheCount() async throws {
        for (width, track) in [(24, 4), (32, 8)] as [(CGFloat, CGFloat)] {
            let picture = try await LayoutPicture("""
            root = Col([Grid([a,a,a], 3, -1, 4)], 0)
            a = Box([], null, null, null, "red", null, null, null, 10)
            """, width: width)
            #expect(try picture.rect("/children/0/children/0").width == track)
            #expect(try picture.rect("/children/0/children/1").minX == track + 16)
            #expect(try picture.rect("/children/0/children/2").minY == 26)
        }
    }

    @Test func stretchedGridCellResolvesPercentageChildAgainstTheRowHeight() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Grid([Box([child]), tall], 2, null, 0)], 0)
        child = Box([], null, null, null, "red", null, null, null, "50%")
        tall = Box([], null, null, null, "blue", null, null, null, 100)
        """)
        #expect(try picture.rect("/children/0/children/0").height == 100)
        #expect(try picture.rect("/children/0/children/0/children/0").height == 50)
    }

    @Test func nestedGridDistributesStretchHeightAcrossAutoRows() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Grid([Grid([tile,tile], 1, null, 0), tall], 2, null, 0)], 0)
        tile = Box([], null, null, null, "red", null, null, null, 10)
        tall = Box([], null, null, null, "blue", null, null, null, 100)
        """)
        let cell = "/children/0/children/0"
        #expect(try picture.rect(cell).height == 100)
        #expect(try picture.rect(cell + "/children/0").height == 10)
        #expect(try picture.rect(cell + "/children/1").minY == 50)
        #expect(try picture.rect(cell + "/children/1").height == 10)
    }

    @Test func gridStretchesAutoHeightCellsAndSizesTypeOnlyInDirectFlowChildren() async throws {
        struct Leaf: View {
            @Environment(\.cardTypography) var typography
            var body: some View {
                Text("Measure").font(typography?.font ?? .system(size: 18)).lineHeight(.exact(points: typography?.lineHeight ?? 32))
            }
        }
        let picture = try await LayoutPicture("""
        root = Col([Grid([Col([Title("Heading", "lg"), Text("Body")]), Card([], "soft", 0, 0), Col([Text("Explicit gap")], 0)], 2, null, 0)])
        """, width: 300, component: { element, _ in
            ["Title", "Text"].contains(element.typeName) ? AnyView(Leaf()) : nil
        })
        let cell = "/children/0/children/"
        let titleHeight = try picture.rect(cell + "0/children/0").height
        #expect(titleHeight == 24)
        #expect(try picture.rect(cell + "0/children/1").height == 22)
        #expect(try picture.rect(cell + "1").height == picture.rect(cell + "0").height)
        #expect(try picture.rect(cell + "2/children/0").height == 32)
    }
}
