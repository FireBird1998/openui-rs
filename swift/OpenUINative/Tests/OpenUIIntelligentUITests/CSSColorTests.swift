// Reading a raw CSS colour as a browser paints it, checked against
// conversions known independently of this code (sRGB red in each space,
// CSS Color 4's keyword table).
@testable import OpenUIIntelligentUI
import Testing

private func rgb(_ css: String) -> ThemeColor? {
    if case let .color(c)? = CSSPaint.parse(css) { c } else { nil }
}

private func expectColor(_ css: String, _ r: Double, _ g: Double, _ b: Double, _ a: Double = 1,
                         within tolerance: Double = 1e-9, sourceLocation: SourceLocation = #_sourceLocation) {
    guard let c = rgb(css) else {
        Issue.record("\(css) was not read", sourceLocation: sourceLocation)
        return
    }
    #expect(close(c, ThemeColor(red: r, green: g, blue: b, alpha: a), within: tolerance),
            "\(css) gave \(c)", sourceLocation: sourceLocation)
}

@Suite struct CSSColorTests {
    @Test func hexInEachLength() {
        expectColor("#0285ff", 2 / 255, 133 / 255, 1)
        expectColor("#FFF", 1, 1, 1)
        expectColor("#f008", 1, 0, 0, 0x88 / 255)
        expectColor("#0285ff80", 2 / 255, 133 / 255, 1, 128 / 255)
        for bad in ["#", "#ff", "#fffff", "#ggg", "#0285ff8", "#0285ff800"] {
            #expect(CSSPaint.parse(bad) == nil, "\(bad)")
        }
    }

    @Test func namedColoursTransparentAndCurrentColor() {
        expectColor("red", 1, 0, 0)
        expectColor("RebeccaPurple", 0x66 / 255, 0x33 / 255, 0x99 / 255)
        expectColor("grey", 128 / 255, 128 / 255, 128 / 255)
        expectColor("transparent", 0, 0, 0, 0)
        #expect(CSSPaint.parse("currentColor") == .currentColor)
        #expect(CSSPaint.parse("CURRENTCOLOR") == .currentColor)
        for bad in ["ink", "banana", "red blue", "red !important", ""] {
            #expect(CSSPaint.parse(bad) == nil, "\(bad)")
        }
    }

    @Test func rgbAndHslInBothSyntaxes() {
        expectColor("rgb(255, 0, 0)", 1, 0, 0)
        expectColor("rgba(255,0,0,0.5)", 1, 0, 0, 0.5)
        expectColor("rgb(100% 0% 0% / 50%)", 1, 0, 0, 0.5)
        expectColor("rgb(300 -5 none)", 1, 0, 0)
        expectColor("RGB(0 127.5 255)", 0, 0.5, 1)
        expectColor("hsl(120 100% 50%)", 0, 1, 0)
        expectColor("hsl(120deg, 100%, 25%)", 0, 0.5, 0)
        expectColor("hsla(0.5turn 100% 50% / 0.25)", 0, 1, 1, 0.25)
        expectColor("hsl(-120 100 50)", 0, 0, 1)
        expectColor("hwb(0 0% 0%)", 1, 0, 0)
        expectColor("hwb(0 60% 60%)", 0.5, 0.5, 0.5)
        for bad in ["rgb(1, 2)", "rgb(1, 2 3)", "rgb(1, 50%, 3)", "rgb(none, 0, 0)", "rgb(1 2 3 4)",
                    "rgb(1 2 3 /)", "hsl(120, 100, 50)", "hwb(0, 0%, 0%)", "rgb(1 2 3))", "rgb(1deg 2 3)"] {
            #expect(CSSPaint.parse(bad) == nil, "\(bad)")
        }
    }

    @Test func labSpacesAgreeOnRed() {
        // sRGB red, as CSS Color 4's own conversions give it.
        expectColor("oklch(0.627955 0.257683 29.2339)", 1, 0, 0, within: 1e-4)
        expectColor("oklab(0.627955 0.224863 0.125846)", 1, 0, 0, within: 1e-4)
        expectColor("oklch(62.7955% 64.42% 29.2339deg)", 1, 0, 0, within: 1e-3)
        expectColor("lab(54.2917 80.8125 69.8851)", 1, 0, 0, within: 1e-3)
        expectColor("lch(54.2917 106.839 40.8526)", 1, 0, 0, within: 1e-3)
        expectColor("oklch(1 0 0)", 1, 1, 1, within: 1e-6)
        expectColor("oklch(0 0 0 / 0.05)", 0, 0, 0, 0.05)
        expectColor("lab(100 0 0)", 1, 1, 1, within: 1e-4)
    }

    @Test func theThemesGreysAreTheirLightnessCubedAndEncoded() {
        // An achromatic oklch(L 0 h) is linear L³ in every channel: worked
        // out apart from the matrices, for light canvas and light ink.
        let canvas = 0.9920934433000063, ink = 0.011791735160000002
        #expect(close(IntelligentTheme.light.canvas, ThemeColor(red: canvas, green: canvas, blue: canvas), within: 1e-6))
        #expect(close(IntelligentTheme.light.ink, ThemeColor(red: ink, green: ink, blue: ink), within: 1e-6))
    }

    @Test func colorMix() {
        expectColor("color-mix(in srgb, red 40%, blue)", 0.4, 0, 0.6)
        expectColor("color-mix(in srgb, red, blue)", 0.5, 0, 0.5)
        expectColor("color-mix(in srgb, 30% red, blue)", 0.3, 0, 0.7)
        expectColor("color-mix(in srgb, red 20%, blue 20%)", 0.5, 0, 0.5, 0.4)
        expectColor("color-mix(in srgb, red 60%, transparent)", 1, 0, 0, 0.6)
        expectColor("color-mix(in srgb-linear, black, white)", 0.7353569830524495, 0.7353569830524495,
                    0.7353569830524495, within: 1e-9)
        // Upstream's own: ink washes and the ink hover.
        let ink = ThemeColor.oklch(0.097, 0, 0)
        #expect(rgb("color-mix(in srgb, oklch(0.097 0 0 / 1) 4%, transparent)").map { close($0, ink.mixedWithTransparent(0.04)) } == true)
        let hover = rgb("color-mix(in oklch, oklch(0.097 0 0 / 1) 82%, oklch(0.994 0 89.876 / 1))")
        #expect(hover.map { close($0, .oklch(0.82 * 0.097 + 0.18 * 0.994, 0, 0), within: 1e-6) } == true)
        // Hue goes the shorter way unless told otherwise.
        let shorter = rgb("color-mix(in oklch, oklch(0.7 0.1 350), oklch(0.7 0.1 30))")
        #expect(shorter.map { close($0, .oklch(0.7, 0.1, 10), within: 1e-6) } == true)
        let longer = rgb("color-mix(in oklch longer hue, oklch(0.7 0.1 350), oklch(0.7 0.1 30))")
        #expect(longer.map { close($0, .oklch(0.7, 0.1, 190), within: 1e-6) } == true)
        for bad in ["color-mix(in srgb, red)", "color-mix(in hsl, red, blue)", "color-mix(srgb, red, blue)",
                    "color-mix(in srgb, red 0%, blue 0%)", "color-mix(in srgb, red 120%, blue)",
                    "color-mix(in srgb, currentColor, blue)", "color-mix(in srgb longer hue, red, blue)"] {
            #expect(CSSPaint.parse(bad) == nil, "\(bad)")
        }
    }

    @Test func whatABrowserKnowsButThisReaderDoesNotIsNoColour() {
        for unread in ["color(srgb 1 0 0)", "rgb(from red r g b)", "Canvas", "light-dark(red, blue)",
                       "rgb(calc(255) 0 0)", "linear-gradient(red, blue)"] {
            #expect(CSSPaint.parse(unread) == nil, "\(unread)")
        }
    }

    @Test func anyTextEndsWithoutFailing() {
        let deep = String(repeating: "color-mix(in srgb, ", count: 10_000) + "red, blue"
            + String(repeating: ")", count: 10_000)
        #expect(CSSPaint.parse(deep) == nil)
        // Within the limit, nesting works.
        let nested = String(repeating: "color-mix(in srgb, red, ", count: 8) + "red" + String(repeating: ")", count: 8)
        expectColor(nested, 1, 0, 0)
        for odd in ["(", ")", "rgb(", "#", "-", "--", "1e", "+.", "\u{0}", "rgb(1e999 0 0)", "😀", "\u{FEFF}red"] {
            _ = CSSPaint.parse(odd)
        }
        // A channel that overflows a conversion is no colour, never NaN.
        // A hue that overflows its unit reads as 0 degrees.
        expectColor("oklch(0.5 0.1 1e308turn)", 0.5658744664454036, 0.2854684901892496, 0.3796666876061322,
                    within: 1e-9)
        if case let .color(c)? = CSSPaint.parse("oklch(0.5 0.1 0)") {
            #expect(close(c, ThemeColor(red: 0.5658744664454036, green: 0.2854684901892496, blue: 0.3796666876061322)))
        }
        for huge in ["lab(50 1e308 0)", "lch(50 1e308 1e308)", "oklab(0.5 1e308 0)"] {
            #expect(CSSPaint.parse(huge) == nil, "\(huge)")
        }
        // CSS closes an open function at the end of the value.
        expectColor("rgb(255 0 0", 1, 0, 0)
    }
}
