// Pixel tests compare a token's declared sRGB value with what was drawn, so
// reading a pixel must not depend on the colour space the bitmap holds.
import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

@Suite @MainActor struct PixelColourSpaceTests {
    @Test(arguments: WindowColorSpace.allCases)
    func pixelsReadAsTheTokensDeclaredSRGBInEitherColourSpace(space: WindowColorSpace) throws {
        let theme = IntelligentTheme.light
        for (index, token) in [theme.chart1, theme.chart2, theme.chart3, theme.chart4, theme.chart5, theme.chart6]
            .enumerated() {
            let image = try #require(ImageRenderer(content: token.color.frame(width: 4, height: 4)).cgImage)
            let bitmap = try #require(NSBitmapImageRep(cgImage: image)
                .converting(to: space.colorSpace, renderingIntent: .default))
            #expect(bitmap.colorSpace.cgColorSpace?.name == space.colorSpace.cgColorSpace?.name)
            let pixel = try #require(bitmap.sRGBColor(x: 2, y: 2))
            let expected = token.sRGB
            // Drawn in 8-bit sRGB, then stored in 8-bit P3: near the edge of
            // the sRGB gamut that loses up to five sRGB steps (0.019 measured
            // for chart2's red). Reading P3 as generic RGB is 0.34 out.
            let step = 0.03
            #expect(abs(pixel.redComponent - expected.redComponent) <= step, "chart\(index + 1) \(pixel)")
            #expect(abs(pixel.greenComponent - expected.greenComponent) <= step, "chart\(index + 1) \(pixel)")
            #expect(abs(pixel.blueComponent - expected.blueComponent) <= step, "chart\(index + 1) \(pixel)")
            #expect(pixel.alphaComponent == 1)
        }
    }
}
