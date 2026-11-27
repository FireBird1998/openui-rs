@testable import OpenUIIntelligentUI
import SwiftUI
import SwiftTestSupport
import Testing

@Suite(NativeTestWatchdog()) @MainActor struct ImageDrawingTests {
    @Test(arguments: ["\"12px\"", "true", "null", "0"])
    func materializedRadiusMatchesTheUpstreamCorner(_ radius: String) async throws {
        let picture = try await LayoutPicture("root = Image(\"\", \"Photo\", 1, \(radius))", width: 120, height: 120)
        let frame = try picture.rect("/")
        let corner = try picture.pixel(Int(frame.minX + 1), Int(frame.minY + 1))
        #expect((corner.alphaComponent > 1.0 / 255) == (radius != "null"))
    }

    @Test func imageUsesItsWidthAndTheUpstreamRatioRange() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Image("", "Default"), Image("", "Wide", 2.5),
                    Image("", "Lower endpoint", 0.2), Image("", "Upper endpoint", 5)], 0)
        """, width: 150, height: 400)
        for (index, height) in [100.0, 60, 100, 100].enumerated() {
            let frame = try picture.rect("/children/\(index)")
            #expect(frame.size == CGSize(width: 150, height: height))
        }
    }

    @Test func imageFillsTheWidthEvenInAStartAlignedColumn() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Image("", "Photo", 2)], 0, "start")
        """, width: 400, height: 240)
        #expect(try picture.rect("/children/0").size == CGSize(width: 400, height: 200))
    }

    @Test func directAndBoxedImagesUseTheMediaRowRatio() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Row([photo, block], 0, "end"), Row([Box([photo]), block], 0, "end")], 0)
        photo = Image("", "Photo", 2)
        block = Box([], null, null, null, null, null, null, 20, 10)
        """, width: 400, height: 240)
        let direct = "/children/0/children/"
        let boxed = "/children/1/children/"
        #expect(try picture.rect(direct + "0").size == CGSize(width: 132, height: 99))
        #expect(try picture.rect(direct + "1").origin == CGPoint(x: 148, y: 0))
        #expect(try picture.rect(boxed + "0/children/0").size == CGSize(width: 132, height: 99))
        #expect(try picture.rect(boxed + "1").origin == CGPoint(x: 132, y: 188))
    }

    @Test func emptyImageDrawsTheThemeGlyphAndRadius() async throws {
        for mode in [IntelligentUIMode.light, .dark] {
            let picture = try await LayoutPicture("""
            root = Col([Image("", "Rounded"), Image("file:///ignored", "Square", 1.5, 0)], 0)
            """, width: 150, height: 200, mode: mode)
            let rounded = try picture.rect("/children/0")
            let square = try picture.rect("/children/1")
            let theme = mode == .light ? IntelligentTheme.light : .dark
            let fill = try LayoutPicture.swatch(theme.photoEmpty.color)
            let field = try picture.pixel(Int(rounded.midX), Int(rounded.minY + 5))
            #expect(abs(field.alphaComponent - fill.alphaComponent) <= 1.0 / 255)
            let outside = try picture.pixel(Int(rounded.minX + 1), Int(rounded.minY + 1))
            let inside = try picture.pixel(Int(square.minX + 1), Int(square.minY + 1))
            #expect(outside.alphaComponent <= 1.0 / 255)
            #expect(abs(inside.alphaComponent - fill.alphaComponent) <= 1.0 / 255)

            let glyph = try LayoutPicture.swatch(theme.photoEmptyIcon.color, over: theme.photoEmpty.color)
            let x = Int(rounded.midX), y = Int(rounded.midY)
            let hasGlyph = try ((y - 13)..<(y + 13)).contains { row in
                try ((x - 13)..<(x + 13)).contains { column in
                    try picture.pixel(column, row).alphaComponent >= glyph.alphaComponent - 1.0 / 255
                }
            }
            #expect(hasGlyph)
        }
    }
}
