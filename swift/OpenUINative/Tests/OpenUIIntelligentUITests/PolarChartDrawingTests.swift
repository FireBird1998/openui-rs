import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import SwiftTestSupport
import Testing

@Suite(NativeTestWatchdog()) @MainActor struct PolarChartDrawingTests {
    @Test(arguments: ["pie", "donut"], [Int(140), 399, 400, 800])
    func drawsSortedThemeColoursAndStacksLegendAtTheWidthBreakpoint(kind: String, width: Int) async throws {
        let mode: IntelligentUIMode = kind == "pie" ? .light : .dark
        let theme = mode == .light ? IntelligentTheme.light : .dark
        let image = try await picture(kind, width: width, mode: mode)
        let chart = try image.rect("/children/0")
        let after = try image.rect("/children/1")
        #expect(chart.width == CGFloat(width))
        #expect(after.minY == chart.maxY)
        let beside = width >= 400
        let panel = beside ? (chart.width - 20) / 2 : chart.width
        let plot = max(150, min(300, panel))
        #expect(chart.height == (beside ? max(plot, 296) : plot + 20 + 64))
        let center = CGPoint(x: chart.minX + panel / 2, y: beside ? chart.midY : chart.minY + plot / 2)
        let radius = plot * 0.45
        let markDistance = kind == "donut" ? radius * 0.95 : radius * 0.65
        let paint1 = theme.chart1.sRGB
        let paint2 = theme.chart2.sRGB
        #expect(try matches(image.pixel(Int(center.x + markDistance), Int(center.y)), paint1))
        let diagonal = markDistance / sqrt(2)
        #expect(try matches(image.pixel(Int(center.x - diagonal), Int(center.y - diagonal)), paint2))
        if panel > plot {
            #expect(try image.pixel(Int(center.x + radius + 5), Int(center.y)).alphaComponent < 0.01)
        }

        if kind == "donut" {
            #expect(try image.pixel(Int(center.x + radius * 0.22), Int(center.y)).alphaComponent < 0.01)
            let track = try image.pixel(Int(center.x + radius * 0.30), Int(center.y))
            #expect(abs(track.alphaComponent - theme.surface.alpha) < 0.02)
        }
        let swatchX = Int(beside ? chart.minX + panel + 20 + 24 : chart.minX + 24)
        let firstY = Int(beside ? chart.midY - 16 : chart.minY + plot + 20 + 16)
        #expect(try matches(image.pixel(swatchX, firstY), paint1))
        #expect(try matches(image.pixel(swatchX, firstY + 32), paint2))
    }

    @Test(arguments: ["pie", "donut"])
    func heightStackedAndTooltipFormatLeaveTheRestingDrawingUnchanged(kind: String) async throws {
        let original = try await picture(kind, width: 340, options: "")
        let withOptions = try await picture(kind, width: 340, options: #", "percent", true, 20"#)
        let paint = IntelligentTheme.light.chart1.sRGB
        for rendered in [original, withOptions] {
            let chart = try rendered.rect("/children/0")
            #expect(chart.height == 384)
            #expect(try matches(rendered.pixel(Int(chart.midX + 128), Int(chart.minY + 150)), paint))
            #expect(try matches(rendered.pixel(Int(chart.minX + 24), Int(chart.maxY - 48)), paint))
        }
        #expect(try original.rect("/children/0") == withOptions.rect("/children/0"))
        #expect(try original.renderedPixels() == withOptions.renderedPixels())
    }

    @Test func legendCollapsesWhitespaceAndKeepsTheRestOfTheLabel() async throws {
        func draw(_ label: String) async throws -> Data {
            try await LayoutPicture("""
            root = Col([Chart("pie", [{name:"\(label)",value:1}])], 0)
            """, width: 200, height: 260).renderedPixels()
        }
        let expected = try await draw("North 🚀")
        let otherSuffix = try await draw("North 💫")
        #expect(expected != otherSuffix)
        #expect(try await draw(#"North\n \t 🚀"#) == expected)
        #expect(try await draw(#"North\n \t 💫"#) == otherSuffix)
    }

    @Test(arguments: ["pie", "donut"])
    func emptyChartContributesNoSiblingGapAndSingleSliceIsPainted(kind: String) async throws {
        let empty = try await LayoutPicture("""
        root = Col([Divider(), Chart("\(kind)", []), Divider()], 12)
        """)
        let first = try empty.rect("/children/0")
        let last = try empty.rect("/children/2")
        let gap = try #require(Spacing.points(.number(12)))
        #expect(abs(last.minY - first.maxY - gap) < 0.01)

        let single = try await LayoutPicture("""
        root = Col([Chart("\(kind)", [{name:"Only",amount:1}])], 0)
        """, width: 260, height: 350)
        let rect = try single.rect("/children/0")
        let center = CGPoint(x: rect.midX, y: rect.minY + rect.width / 2)
        let colour = IntelligentTheme.light.chart1.sRGB
        let x = center.x + rect.width * (kind == "donut" ? 0.4275 : 0.3)
        #expect(try matches(single.pixel(Int(x), Int(center.y)), colour))
    }

    private func picture(_ kind: String, width: Int, mode: IntelligentUIMode = .light,
                         options: String = "") async throws -> LayoutPicture {
        try await LayoutPicture("""
        root = Col([Chart("\(kind)", [{name:"Small", amount:1}, {name:"Large", amount:3}], "name", ["amount"]\(options)), Divider()], 0)
        """, width: CGFloat(width), height: 500, mode: mode)
    }

    private func matches(_ pixel: NSColor, _ swatch: NSColor) -> Bool {
        pixel.alphaComponent > 0.9 &&
            abs(pixel.redComponent - swatch.redComponent) < 0.05 &&
            abs(pixel.greenComponent - swatch.greenComponent) < 0.05 &&
            abs(pixel.blueComponent - swatch.blueComponent) < 0.05
    }
}
